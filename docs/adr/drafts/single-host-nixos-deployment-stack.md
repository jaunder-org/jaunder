# ADR-DRAFT: Single-Host NixOS Deployment Stack

- Status: proposed
- Date: 2026-09-15
- Issue: [#1489](https://github.com/jaunder-org/jaunder/issues/1489)

## Context

[ADR-0142](../0142-declarative-nixos-deployment-package-outputs.md) defines
`nixosModules.jaunder` as a deliberately narrow adapter from Jaunder's process
configuration to one systemd service. That boundary is valuable for operators
who already own their reverse proxy, database, and observability architecture,
but it leaves a new operator to assemble every production dependency.

[ADR-0008](../0008-deployment-model.md) requires an external reverse proxy, and
[ADR-0011](../0011-unified-observability.md) makes OpenTelemetry the
application's trace and metrics interface. A production-shaped single-host
deployment can compose those external responsibilities without moving them into
the Jaunder binary or expanding the minimal module's compatibility surface.

The chosen observability stores must remain practical for a small self-hosted
instance. The Victoria family provides persistent single-node metrics, logs, and
traces with built-in query interfaces, avoiding a separate Grafana service.
VictoriaTraces is explicitly work in progress upstream, so its retained data
cannot yet support a cross-version compatibility promise.

## Decision

The flake exports a second supported module, `nixosModules.jaunder-stack`, which
imports the minimal Jaunder module and exposes `services.jaunder.stack`.
Enabling it owns one complete single-host composition:

- Caddy is the only public listener and terminates automatic HTTPS for a
  required host name before proxying to loopback-bound Jaunder.
- Jaunder runs in production mode with JSON-formatted structured logs and
  exports traces and metrics to a loopback-only OpenTelemetry Collector.
- The collector routes Jaunder metrics to VictoriaMetrics, traces to
  VictoriaTraces, and parsed structured events read only from `jaunder.service`
  to VictoriaLogs without collapsing their named fields into plain messages.
- The collector, Victoria stores, and each store's built-in web UI remain on
  loopback. By default they are operator surfaces reached locally or through SSH
  forwarding, not public application endpoints.
- The Victoria services always use native `/metrics`, `/logs`, and `/traces`
  HTTP path prefixes, and Collector exporters use their prefixed ingestion
  endpoints directly over loopback. An optional second HTTPS host exposes those
  prefixes through Caddy. It requires a non-whitespace Basic Auth username
  matching `[A-Za-z0-9._-]+` and a password-hash option; recognized
  `$2a$`/`$2b$` bcrypt and `$argon2id$` prefixes select the matching Caddy
  algorithm, while plaintext and unrecognized hashes fail module evaluation. The
  password hash is a literal evaluated Nix string embedded in generated Caddy
  configuration and the Nix store, not a runtime-file secret. Caddy
  authenticates every external request before proxying it to the credential-free
  loopback services.
- The stack's closed database choice is SQLite by default or local PostgreSQL.
  PostgreSQL mode additively enables the host's ordinary shared PostgreSQL
  instance, ensures a `jaunder` database owned by a matching login role with the
  privileges required for initialization and migrations, and connects over a
  Unix socket with peer authentication. It creates no separate cluster, chooses
  no PostgreSQL package, and assigns no PostgreSQL listener, authentication, or
  global policy. On the pinned NixOS module, `enableTCPIP = false` still retains
  a localhost TCP listener; the stack adds no non-loopback listener, host HBA
  rule, firewall opening, or other network exposure.

The stack guarantees that its constituent services are enabled. Their existing
NixOS modules remain the configuration surface for retention and other tuning;
the stack does not mirror those options. Grafana, unauthenticated observability
exposure, external databases, clustered storage, and whole-host telemetry remain
custom compositions.

Victoria data is persistent, retained operational evidence, but it is outside
Jaunder's application backup and restore contract. The module promises fresh
VictoriaTraces deployment and same-version restart only, not compatible on-disk
upgrades or stable third-party query APIs.

## Consequences

Operators gain a small declaration that produces a complete public Jaunder
service and a locally queryable observability stack. The existing minimal module
keeps its current options, defaults, and ownership boundary for custom
composition.

The deployment runs five additional services—Caddy, the collector, and three
Victoria stores—but avoids Grafana and external object storage. Only Caddy and
ports 80 and 443 are public. Remote access to the Victoria interfaces is an
explicit, coarsely authorized single-operator surface rather than an account,
role, or SSO system.

PostgreSQL mode composes with databases, roles, package selection, listeners,
authentication, and global policy declared by other NixOS modules; it does not
claim that the shared instance is globally socket-only. Caddy OTLP telemetry and
PostgreSQL metrics are feasible future extensions, but are excluded until
production evidence justifies their additional signal, credential, cardinality,
and retention policy. Backing up Victoria data or promising VictoriaTraces
upgrade compatibility likewise requires a separate decision.
