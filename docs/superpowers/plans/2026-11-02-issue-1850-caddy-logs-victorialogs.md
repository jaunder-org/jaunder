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
    VM must verify both vhosts, namespace diagnostics/default exclusion, exact
    Caddy source-message/store equality and journal metadata, native redaction
    of Cookie, Set-Cookie, Authorization, and Proxy-Authorization, endpoint
    acceptance, saved-state restart, and receiver-interruption recovery. Count
    previously stored markers before and after restart/recovery; prove an upload
    failure before restoring VictoriaLogs; bound the reload-message comparison
    to a pre-action journal cursor. This VM observed one previously uploaded
    restart marker replay after a failed receiver request; it records counts and
    proves recovery without claiming exactly-once delivery. The production
    no-duplicates acceptance remains unresolved. The proof uses the loopback
    receiver without uploader TLS/client credentials, which this route does not
    configure. Existing `checks.x86_64-linux.jaunder-stack-sqlite-bcrypt`
    separately exercised Caddy credential reload/rotation. Production wiring
    remains Task 2 and is not included in this slice.
- [ ] Task 2: Wire the production native uploader and both managed Caddy vhosts
  - Contract: use `services.journald.upload` for URL, service, saved-state
    arrangement, restart and hardening; override its `ExecStart` only to add
    `--namespace=jaunder` while preserving `--save-state`. Upload only the
    dedicated namespace. Configure one JSON stdout access logger per enabled
    application/optional observability vhost. No Collector Caddy
    receiver/pipeline; Jaunder OTLP and `transform/jaunder_privacy` remain
    unchanged.
  - Verification: Nix evaluation checks the composed service invocation,
    namespace, endpoint, state, restart/hardening and both vhost outputs;
    confirm no default-file or duplicate access logger. Preserve native
    VictoriaLogs retention, loopback endpoints, metrics/traces and existing
    Jaunder routing.
- [ ] Task 3: Prove production behavior and unchanged boundaries
  - Contract: full native access records and service/error messages, with
    Caddy's default sensitive-header redaction; no body/extra credential
    capture. The initial uploaded source is Caddy plus namespace-internal
    bookkeeping, not the whole host or a claimed exact unit filter.
  - Verification: compare production source journal records/messages to
    VictoriaLogs; test access through each enabled vhost, representative
    service/error activity, namespace-aware diagnostics, native redaction, no
    duplicates, cursor/restart behavior and exclusion of default/whole-host
    journal sources. Confirm Jaunder OTLP still follows existing privacy policy,
    metrics/traces and listeners are unchanged, and native retention remains
    unchanged.
- [ ] Task 4: Reconcile current architecture and decision records
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
  diagnostics, with Caddy plus namespace-internal bookkeeping initially. Do not
  add a redundant approval gate. Escalate only material new behavior,
  permissions, source expansion or unsupported adapter need.
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
