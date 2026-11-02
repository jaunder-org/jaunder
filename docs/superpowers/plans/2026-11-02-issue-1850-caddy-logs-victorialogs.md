# Caddy Access and Service Logs in VictoriaLogs Implementation Outline

> Execute with `dev-cycle-iterate` under the approved native route in
> [the spec](../specs/2026-11-02-issue-1850-caddy-logs-victorialogs.md). Outline
> needed because this introduces a durable journal source, cursor/restart
> behavior, namespace boundary and uploader permissions.

## Execution summary

First prove the source-to-store boundary with the pinned
`systemd-journal-upload` and VictoriaLogs native endpoint in a bounded VM: Caddy
stdout/stderr and namespace bookkeeping, full record framing/content,
namespace-aware diagnostics, saved cursor and restart behavior, and
credential/hardening protections. Only after that succeeds, wire the supported
NixOS uploader service to the `jaunder` namespace and native endpoint,
preserving its state/restart/hardening. Keep Caddy out of Collector; leave
Jaunder's existing OTLP/Collector/privacy route untouched. Then prove both
managed vhosts and reconcile current architecture without rewriting historical
ADR-0196.

## Task outline

- [x] Task 1: Add the focused `jaunder` namespace upload/source-to-store VM
      regression proof before production wiring (first implementation slice)
  - Contract: route Caddy stdout/stderr to `LogNamespace=jaunder`; use pinned
    systemd 261.2 `systemd-journal-upload` with `--namespace=jaunder`, native
    VictoriaLogs endpoint `/insert/journald/upload` (stack prefix `/logs`), and
    saved cursor state. Treat scope as the namespace, including
    namespace-internal bookkeeping, not as a unit-only filter.
  - Verification: the focused `checks.x86_64-linux.jaunder-caddy-journal-upload`
    VM verifies both vhosts, namespace diagnostics/default exclusion, exact
    Caddy source-message/store equality and journal metadata, native redaction
    of Cookie, Set-Cookie, Authorization, and Proxy-Authorization, endpoint
    acceptance, saved-state restart, and receiver-interruption recovery. It
    counts stored markers before/after restart and recovery, observes a failed
    upload before restoring VictoriaLogs, and bounds reload-message comparison
    to a pre-action cursor. One previously uploaded restart marker replayed
    after a failed receiver request; counts document this allowed recovery
    replay. This path does not promise exactly-once or universal lossless
    delivery. The loopback receiver has no uploader TLS/client credentials
    configured. The existing `checks.x86_64-linux.jaunder-stack-sqlite-bcrypt`
    exercises Caddy credential reload/rotation.
- [x] Task 2: Wire the production native uploader and both managed Caddy vhosts
  - Contract: use `services.journald.upload` for URL, service, saved-state
    arrangement, restart and hardening; override its `ExecStart` only to add
    `--namespace=jaunder` while preserving `--save-state`. Upload only the
    dedicated namespace. Configure one JSON stdout access logger per enabled
    application/optional observability vhost. No Collector Caddy
    receiver/pipeline; Jaunder OTLP and `transform/jaunder_privacy` remain
    unchanged.
  - Verification: `jaunder-stack-module` asserts the production service,
    uploader URL base `http://127.0.0.1:9428/logs/insert/journald` (the pinned
    uploader appends `/upload`), namespace flags, vhost logger count and
    disabled file logger, including Caddy's runtime-credential configuration.
    The production-wired VM verifies the running unit arguments, persistent
    cursor, restart/hardening, native endpoint/store, and file-credential
    source-to-store ingestion through both vhosts initially and through the
    observability vhost after credential reload/restart. Native VictoriaLogs
    retention, loopback endpoints, Collector pipelines, and existing Jaunder
    routing remain unchanged.
- [x] Task 3: Prove production behavior and unchanged boundaries
  - Contract: full native access records and service/error messages, with
    Caddy's default sensitive-header redaction; no body/extra credential
    capture. The initial uploaded source is Caddy plus namespace-internal
    bookkeeping, not the whole host or a claimed exact unit filter.
  - Verification: the production-wired VM compares complete access records and a
    reload service message with source journal records; exercises both vhosts,
    namespace-aware diagnostics, four native header redactions, default-journal
    exclusion, cursor restart and receiver interruption. No duplicate is seen in
    steady state; interruption recovery may replay and its counts are reported.
    The retained human clarification accepts this possible replay; exactly-once
    and universal losslessness are not claimed. The credential-reload VM
    exercises the operator credential path and existing Jaunder OTLP/privacy
    signals. Evaluation retains loopback listeners and native retention.
- [x] Task 4: Reconcile current architecture and decision records
  - Contract: add a separate numberless ADR draft for the approved Caddy
    namespace/upload exception and update `docs/ARCHITECTURE.md` current-state
    projection. Preserve historical ADR-0196 under ADR-0127; keep
    `docs/adr/drafts/native-otlp-process-logs.md`'s Jaunder source-qualified
    policy intact.
  - Verification: ensure architecture distinguishes implemented behavior from
    proof not yet run and links the current Jaunder OTLP policy and Caddy native
    route. Update the closed checker/assertions only as needed for actual
    implementation, retaining closed-world constraints.

## Risk checks

- Human approval is the exact retained reply “I think that’s a reasonable
  direction. proceed.” for `3ea422e6-2db3-4b8b-89e0-994148751fca` /
  `1850-native-journal-route`; it approves the native route and namespace-aware
  diagnostics, with Caddy plus namespace-internal bookkeeping initially. The
  human clarification “Accept possible recovery replay” for
  `afa90716-9923-413f-aa2d-fdf293fde4d6` / `1850-recovery-replay` permits replay
  during recovery while requiring no duplicate logger/records in steady state;
  it adds no exactly-once or universal losslessness promise. Do not add a
  redundant approval gate. Escalate only material new behavior, permissions,
  source expansion or unsupported adapter need.
- Namespace selection is not a strict per-unit filter. Keep uploaded scope at
  the dedicated namespace and verify boundary/bookkeeping; do not claim
  whole-host or exact `caddy.service`-only filtering.
- Pinned NixOS uploader module does not expose namespace configuration: its
  `ExecStart` override must retain `--save-state` while adding
  `--namespace=jaunder`; validate the effective invocation and durable state
  arrangement.
- `journalctl` diagnostics are namespace-aware; the default namespace alone must
  not be represented as showing Caddy logs.
- Preserve Caddy's native redaction of Cookie, Set-Cookie, Authorization and
  Proxy-Authorization; do not enable `log_credentials`; `bytes_read` is not body
  capture.
- No Collector Caddy hop, no change to Jaunder OTLP/Collector/privacy, native
  retention, or loopback/listener behavior. Do not rewrite ADR-0196.
