# NixOS Deployment Stack Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded slices through
> `jaunder-dispatch` when useful. This outline exists because the approved spec
> adds a public NixOS module with authentication, storage ownership, and
> cross-service telemetry-routing contracts.

## Scope

In:

- A separately exported single-host stack module composed from existing NixOS
  service modules.
- SQLite and additive local PostgreSQL modes.
- Jaunder-only metrics, traces, and structured journal ingestion into the three
  Victoria stores.
- Default loopback query UIs and optional Basic-Auth Caddy ingress.
- Evaluation and NixOS VM proof of the public, security, persistence, and
  ownership contracts.
- Operator documentation and the accepted architecture projection.

Out:

- Grafana, alerts, dashboards, SSO, per-user observability authorization, or
  unauthenticated query access.
- External/clustered databases, separate PostgreSQL clusters, whole-host
  telemetry, Caddy/PostgreSQL telemetry, or Victoria-data backup and upgrade
  compatibility.

## Task outline

- [x] Task 1: Export and evaluate the stack's public configuration contract.
  - Contract: `mkJaunderStackModule` in `nix/nixos.nix` imports
    `mkJaunderModule`; the public `nixosModules.jaunder-stack` export owns
    `services.jaunder.stack.enable`, required application `hostName`, the closed
    `database` choice, and optional `observability.hostName` plus
    `observability.basicAuth.{username,passwordHash}`. Hash prefixes map
    `$2a$`/`$2b$` to bcrypt and `$argon2id$` to Argon2id; absent,
    whitespace-only, plaintext, malformed, and unsupported combinations fail
    evaluation.
  - Contract: the stack enables Caddy, `opentelemetry-collector-contrib`, and
    the three Victoria services; fixes their loopback listeners and native
    `/metrics`, `/logs`, and `/traces` prefixes; sets production/JSON Jaunder
    configuration; and points direct collector exporters at prefixed loopback
    ingestion endpoints. The collector remains a non-root dynamic user with only
    the `systemd-journal` supplementary group needed by its filtered journal
    receiver. Existing minimal-module evaluations remain unchanged.
  - Verification: `checks.jaunder-stack-module` covers both database values,
    default and optional ingress, both accepted hash families, every rejection
    case, Caddy-only firewall openings, effective native retention, collector
    exporter URLs, minimum journal-read authorization, and unchanged
    minimal-module options. Run
    `devtool run -- nix build -L --accept-flake-config .#checks.x86_64-linux.jaunder-stack-module`.

- [x] Task 2: Prove the complete SQLite deployment and observability lifecycle.
  - Contract: `mkJaunderStackVmCheck` in `nix/checks.nix` owns the shared
    stack-test node and fixture contract; its SQLite bcrypt case owns signal
    production and reboot persistence, while its Argon2id case reuses the same
    ingress assertions. Deterministic test TLS replaces public ACME only inside
    these checks.
  - Verification: drive a real request through Caddy; prove unauthenticated 401
    and authenticated access to all three prefixed built-in UIs/query APIs for
    bcrypt and Argon2id cases; query the resulting metric, trace, and parsed
    `jaunder.service` event with a named field; reboot; then prove the route,
    UIs, and previously captured signals remain available. Inspect listeners and
    the collector identity/groups to prove only Caddy is non-loopback and the
    collector reads the journal without root. Export
    `checks.jaunder-stack-sqlite-bcrypt` and
    `checks.jaunder-stack-sqlite-argon2id`; build both directly before Task 3.

- [ ] Task 3: Prove additive shared PostgreSQL composition.
  - Contract: PostgreSQL mode uses the host's ordinary `services.postgresql`,
    adds an owned `jaunder` database and matching login role through native
    additive options, and connects over `/run/postgresql` with peer
    authentication. `jaunder.service` requires and starts after the NixOS
    PostgreSQL readiness target so its `preStart` initialization cannot race a
    cold database. The stack neither chooses the package nor creates or
    overwrites unrelated databases, roles, global settings, network
    authentication, or firewall rules.
  - Verification: `checks.jaunder-stack-postgresql`, built from
    `mkJaunderStackVmCheck`, composes an unrelated database and role, cold-boots
    both services, inspects the catalog owner, runs fresh Jaunder
    initialization/migrations as the `jaunder` role, drives the shared
    application/telemetry fixture, and proves no stack-owned password or network
    exposure. Run
    `devtool run -- nix build -L --accept-flake-config .#checks.x86_64-linux.jaunder-stack-postgresql`.

- [ ] Task 4: Publish the operator contract and current architecture.
  - Contract: documentation gives minimal and stack import examples, required
    host configuration, database selection, local UI/SSH access, optional
    authenticated host and password-hash generation, native retention tuning,
    and the VictoriaTraces maturity and telemetry-backup exclusions.
  - Verification: fold the deployment projection from **Committed direction**
    into current architecture once the output exists; update the flake-output
    summary; run documentation links/ADR projection checks, then
    `devtool run -- cargo xtask check`.

## Risk checks

- Only Caddy binds publicly; OTLP receivers, stores, ingestion APIs, and UIs
  remain loopback even when the optional operator host is enabled.
- Caddy authenticates before proxying; hashes, never plaintext passwords, enter
  Nix configuration. Internal collector traffic never traverses Caddy.
- Native path prefixes are reflected in UI/query routes and every collector
  exporter endpoint so optional external routing cannot break ingestion.
- The journal receiver selects only `jaunder.service`, parses JSON fields, and
  does not recursively ingest the collector or Victoria services. The collector
  remains non-root and receives only the journal-reading supplementary group;
  evaluation and runtime checks prove that authorization boundary.
- PostgreSQL mode requires and starts Jaunder after the NixOS PostgreSQL
  readiness target; the VM test exercises a cold boot before initialization.
- PostgreSQL changes remain additive under Nix module merging and preserve
  another service's package, database, role, and network policy.
- Persistence testing observes data captured before reboot rather than merely
  proving that empty services restart.
- Native retention defaults are verified without redundantly setting values to
  their existing defaults.
- The existing minimal module, production-baseline configurations, and e2e
  module consumers keep their established behavior.
- `CONTEXT.md` needs no change: this deployment composition introduces no domain
  vocabulary.
