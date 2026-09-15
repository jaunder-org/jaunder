# NixOS deployment stack

Issue: [#1489](https://github.com/jaunder-org/jaunder/issues/1489)

## Outcome

Operators can import one production-shaped NixOS module that deploys Jaunder
behind automatic HTTPS together with persistent, locally queryable application
logs, traces, and metrics. The existing minimal Jaunder module remains the
building block for custom deployments.

## Load-bearing decisions

- The flake exports a separate `nixosModules.jaunder-stack` module. Importing it
  adds `services.jaunder.stack`; it does not widen the public options or
  responsibilities of the existing `nixosModules.jaunder` module.
- `services.jaunder.stack.enable` owns a complete single-host composition:
  Jaunder, Caddy, OpenTelemetry Collector, VictoriaMetrics, VictoriaLogs, and
  VictoriaTraces. A deployment that disables required constituents should use
  the minimal module and compose its own stack instead.
- `services.jaunder.stack.hostName` is required. Caddy is the only public
  listener, opens ports 80 and 443, obtains HTTPS certificates automatically,
  and proxies the named host to loopback-bound Jaunder. Existing Caddy options,
  including its ACME account email, remain the operator's configuration surface.
- Jaunder runs in production mode with `JAUNDER_LOG_FORMAT=json` and exports
  traces and metrics to one loopback-only OTLP receiver owned by the collector.
- The collector routes Jaunder metrics to single-node VictoriaMetrics and
  Jaunder traces to single-node VictoriaTraces. It reads only the
  `jaunder.service` journal, parses Jaunder's JSON events without flattening
  them into plain messages, and sends their named fields to single-node
  VictoriaLogs.
- VictoriaMetrics, VictoriaLogs, VictoriaTraces, their built-in web UIs, and the
  collector bind only to loopback. Without further configuration, operators
  reach each UI through local access or SSH forwarding.
- The stack gives VictoriaMetrics, VictoriaLogs, and VictoriaTraces the native
  HTTP path prefixes `/metrics`, `/logs`, and `/traces`. Collector exporters use
  the corresponding prefixed ingestion endpoints directly over loopback; they
  never traverse Caddy or Basic Auth.
- An optional `services.jaunder.stack.observability.hostName` adds one
  Caddy-served HTTPS operator host for those three prefixes. Configuring it
  requires a non-whitespace
  `services.jaunder.stack.observability.basicAuth.username` matching
  `[A-Za-z0-9._-]+` and a non-whitespace
  `services.jaunder.stack.observability.basicAuth.passwordHash`. The hash must
  begin with a recognized bcrypt prefix (`$2a$` or `$2b$`) or `$argon2id$`; any
  other value, including plaintext, fails module evaluation. The prefix selects
  Caddy's `bcrypt` or `argon2id` algorithm, respectively. `passwordHash` is a
  literal evaluated Nix string embedded in generated Caddy configuration and the
  Nix store, not a runtime-file secret. Caddy authenticates every external
  request before proxying to the credential-free loopback services. Grafana is
  not part of the stack.
- The stack exposes a closed database choice,
  `services.jaunder.stack.database = "sqlite" | "postgresql"`, defaulting to
  SQLite.
- Selecting PostgreSQL ensures the host's ordinary `services.postgresql`
  instance is enabled, additively ensures a `jaunder` database owned by a
  matching login role with the privileges required to initialize and migrate its
  schema, and connects as the `jaunder` system user over a Unix socket with peer
  authentication. It creates no separate cluster or data directory, chooses no
  PostgreSQL package, and assigns no PostgreSQL listener, authentication, or
  global policy. On the pinned NixOS module, `enableTCPIP = false` still retains
  a localhost TCP listener; the stack itself adds no non-loopback listener, host
  HBA rule, firewall opening, or other network exposure. Other services may
  share and independently configure the same PostgreSQL instance. External
  PostgreSQL remains a custom composition through the minimal module.
- Native NixOS options remain the tuning surface for Caddy, PostgreSQL, the
  collector, and each Victoria service. The stack does not duplicate their
  retention or storage controls. Consequently the initial native retention is 31
  days for metrics and 7 days each for logs and traces.
- Telemetry stores are persistent operational evidence but are not Jaunder
  application data. Jaunder backup and restore neither include nor promise to
  recover them.
- VictoriaTraces is accepted despite its upstream work-in-progress warning. The
  stack promises fresh deployment and same-version restart, not compatible
  on-disk upgrades or stable third-party query APIs.
- This decision extends rather than replaces the minimal deployment contract:
  Jaunder remains a single binary, TLS remains external to the application, and
  operators can continue importing only `nixosModules.jaunder`.

## Acceptance

- The flake exposes both `nixosModules.jaunder` and
  `nixosModules.jaunder-stack`, and existing minimal-module evaluations retain
  their current options and service behavior.
- A minimal stack declaration with a host name evaluates to a Caddy-fronted,
  production-mode Jaunder service plus the collector and all three Victoria
  services.
- The generated configuration exposes only Caddy on ports 80 and 443; Jaunder,
  OTLP ingestion, all Victoria listeners, and the VictoriaMetrics, VictoriaLogs,
  and VictoriaTraces built-in web UIs remain loopback-only. Each built-in UI
  responds through local access, and the default configuration creates no
  external observability route.
- When an observability host and complete Basic Auth configuration are supplied,
  unauthenticated requests receive HTTP 401 while authenticated requests reach
  the metrics, logs, and traces UIs and their query APIs at the three documented
  path prefixes. Positive cases prove both bcrypt and Argon2id authentication.
  Module evaluation rejects a configured observability host with either
  credential field absent, whitespace-only, or with a username outside
  `[A-Za-z0-9._-]+`, and rejects plaintext, malformed, and unsupported password
  hashes.
- Effective collector configuration proves that its metrics, logs, and traces
  exporters use each store's prefixed loopback ingestion endpoint without
  traversing the authenticated Caddy host.
- A fresh SQLite deployment serves Jaunder through the Caddy route and records a
  driven request in queryable metrics and traces. The corresponding
  `jaunder.service` JSON event is queryable in VictoriaLogs with at least one
  named structured field preserved separately from its message.
- After those signals are captured, a host reboot leaves the metrics, logs, and
  traces queryable and restores the HTTPS application route and all three local
  query UIs.
- A fresh PostgreSQL deployment provides the same observable behavior while
  proving that Jaunder uses its additively configured database in the shared
  local instance over a Unix socket with peer authentication and without a
  database password. PostgreSQL catalog inspection proves that the `jaunder`
  login role owns the database, and fresh initialization and migrations execute
  successfully as that role. The stack assigns no PostgreSQL listener,
  authentication, or global policy; although pinned NixOS retains a localhost
  TCP listener when `enableTCPIP = false`, stack-only configuration adds no
  non-loopback listener, host HBA rule, firewall opening, or other network
  exposure, and composes without replacing an unrelated database or role
  declared by another module.
- Module evaluation rejects an enabled stack without its application host name
  and rejects an unknown database choice.
- Evaluation or runtime inspection proves the effective initial retention of 31
  days for metrics and 7 days each for logs and traces, including native
  defaults represented by an omitted command-line argument.
- Documentation shows the minimal and stack imports, the required declaration,
  database selection, local access to each built-in web UI, optional
  authenticated observability-host configuration, the exact `[A-Za-z0-9._-]+`
  Basic Auth username grammar, password-hash generation on a trusted admin
  machine with a compatible Caddy binary on `PATH`, plaintext and
  offline-guessing guidance, literal-Nix-string hash storage, native retention
  overrides, telemetry persistence and backup boundaries, and the VictoriaTraces
  upgrade caveat.

## Boundaries

- No Grafana, unauthenticated observability exposure, per-user accounts, roles,
  SSO, dashboards, alert rules, or notification routing.
- No external or clustered database mode, PostgreSQL password lifecycle, or
  database backup-policy change.
- No Caddy or PostgreSQL telemetry in this slice. Caddy can emit OTLP metrics
  and traces, and PostgreSQL can be observed through the collector's contrib
  receiver or a Prometheus exporter; those remain possible follow-ups if
  production evidence demonstrates the need.
- No whole-host logs, host metrics, node exporter, or recursive collection of
  the observability services' own journals.
- No high availability, multi-node storage, object storage, remote telemetry
  export, or compatibility promise for retained Victoria data across upgrades.
- No change to Jaunder's HTTP, storage, backup, or domain behavior.
