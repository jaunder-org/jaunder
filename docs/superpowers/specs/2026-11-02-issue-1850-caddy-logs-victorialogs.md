# Caddy Access and Service Logs in VictoriaLogs — DRAFT

> **DRAFT for focused soundness review.** The approved route and scope below are
> authorized for matching implementation after review; this draft is not runtime
> certification.

## Outcome

In the managed single-host Jaunder NixOS stack, make Caddy's full native HTTP
access records and service/error messages searchable in the existing loopback
VictoriaLogs store. Keep Jaunder's native OTLP/Collector route and privacy
policy unchanged.

## Approach and tradeoffs

Configure Caddy's stdout/stderr records in a dedicated systemd journal namespace
named `jaunder`. Initially that namespace contains Caddy records and
namespace-internal bookkeeping; the upload source boundary is the namespace, not
a claimed exact per-unit filter. Use the supported `systemd-journal-upload`
service with saved cursor state, restart behavior and its existing hardening,
targeting the pinned VictoriaLogs native journald upload endpoint
`/logs/insert/journald/upload`. Do not route Caddy through the OpenTelemetry
Collector. The pinned systemd 261.2 uploader accepts `--namespace`, `--url`, and
`--save-state`; NixOS `services.journald.upload` supplies the service,
state/restart/hardening, while its `ExecStart` must be overridden to add
`--namespace=jaunder` and preserve `--save-state`. The pinned VictoriaLogs
1.52.0 tagged receiver registers `/insert/journald/upload`; with the stack's
`/logs` prefix the configured endpoint is `/logs/insert/journald/upload`.

The namespace changes where local Caddy diagnostics are found: use
namespace-aware `journalctl` (for example,
`journalctl --namespace=jaunder -u caddy.service`). Do not assume
default-namespace `journalctl -u caddy.service` shows these records. The
approved source scope is Caddy plus namespace-internal bookkeeping, not
whole-host journal upload; namespace selection does not claim a strict unit-only
filter. Caddy's stdout/stderr journal records, namespace boundary/bookkeeping,
uploader state/cursor and restart behavior, and diagnostics must be proven in
the first bounded source-to-store runtime slice before production wiring.

Explicitly configure exactly one native JSON access-log stream to stdout for
each managed Caddy virtual host: the application host and the optional
observability host when enabled. Avoid NixOS Caddy's default per-host file
destination and duplicate loggers. Preserve the full records and messages as
emitted, without parsing, projecting, redacting, classifying or dropping
content. This includes native request/response fields (including URI, IP,
headers, TLS details when emitted, `bytes_read`, `user_id`, duration, size,
status and response headers) and complete service/error message text. Caddy's
native redaction of `Cookie`, `Set-Cookie`, `Authorization`, and
`Proxy-Authorization` remains enabled; do not set `log_credentials`.
`bytes_read` is not body capture. The approved full-record scope does not
authorize body capture, disabling protections, or additional credential capture.

The uploader accesses the dedicated namespace rather than the whole default
journal; it is not a unit filter, and possible namespace-internal entries are
within the stated initial source boundary. Protect native uploader credentials
and preserve the module's saved cursor/state, restart behavior and hardening.
Keep VictoriaLogs' native seven-day retention and native operator override
unchanged; add no retention knob. The accepted privacy tradeoff is that full
operator-only Caddy records may persist sensitive values for that native
retention period.

### Approved scope and provenance

Human implementation approval is the exact reply **“I think that’s a reasonable
direction. proceed.”**, retained by the Manager for request
`3ea422e6-2db3-4b8b-89e0-994148751fca` / `1850-native-journal-route`. It
approves this native route, including the `jaunder` namespace and
namespace-aware diagnostics, with initial contents limited to Caddy plus
namespace-internal bookkeeping. It replaces the earlier Collector-route
proposal; it is not a new human gate. Escalate only if focused review or runtime
proof identifies material new behavior, permissions, source expansion, or need
for an unsupported adapter beyond this approved route.

The separately retained human content decision under Manager request
`d188df15-8241-428b-8cbb-4d38446aef36` was “Retain everything” for access logs
and accepted full error-message retention, including operator-only disclosure of
paths, endpoints and sensitive values. Read together with the approved route,
this means full native Caddy records/messages subject to Caddy's existing
sensitive-header protections—not body capture, additional credential capture,
disabling protections, whole-host upload, or changing Jaunder privacy. Caddy
v2.11.4 format/redaction evidence is `result:caddy-shape#1`.

### Existing Jaunder route and architecture records

Jaunder application OTLP logs continue through the existing Collector and
`transform/jaunder_privacy` policy unchanged. Do not merge Caddy into Jaunder's
source-qualified admission. The Collector remains in the existing Jaunder signal
route but is not a Caddy hop.

Reconcile current-state architecture against the existing native-OTLP
process-logs proposal and ADR-0196, following ADR-0127: do not rewrite
historical ADR-0196. Implementation work may add a separate numberless ADR draft
for the Caddy exception and update the current `docs/ARCHITECTURE.md`
projection, preserving ADR-0196's remaining commitments. This spec/outline
revision changes no ADR, architecture, checker, or implementation file.

## Acceptance

First, a bounded VM source-to-store proof using pinned systemd 261.2, the
supported uploader service, and pinned VictoriaLogs 1.52.0 must establish: Caddy
stdout/stderr and namespace-internal bookkeeping reside in `jaunder`;
namespace-aware diagnostics find Caddy records; source framing/content reaches
VictoriaLogs with the full native record/message and journal fields; native
credential protections remain; uploader credentials are protected; saved
cursor/state survives restart and resumes correctly; service restart/hardening
are effective; and no default/whole-host journal is uploaded. Test endpoint
acceptance against the pinned native upload endpoint. This is the first
implementation slice and must pass before production wiring.

Then verify both enabled managed vhosts emit exactly one JSON access stream to
stdout, with no default file logger or duplicate. Through production wiring,
compare captured source journal access records and service/error messages with
stored VictoriaLogs records, verify namespace scope/bookkeeping and absence of
unrelated default-journal sources, exact native redaction, no duplicates, and
namespace-aware diagnostics. Verify existing loopback listeners, native
retention settings, metrics/traces, and Jaunder OTLP logs/privacy behavior are
unchanged. Nix evaluation alone is not ingestion proof.

Pinned Caddy v2.11.4 source establishes access JSON fields, non-terminal
service-log JSON defaults and native redaction control; service messages/fields
vary, and no fixed successful reload record is guaranteed
(`result:caddy-shape#1`). Pinned systemd 261.2 executable supports
`--namespace`, `--url`, and `--save-state`; pinned NixOS module supplies the
uploader service/state/restart/hardening but no namespace option, hence the
stated `ExecStart` override. Pinned VictoriaLogs 1.52.0 source establishes
`/insert/journald/upload`; the stack prefix yields
`/logs/insert/journald/upload`. These are qualified source/interface facts, not
runtime proof. No new build or VM is part of this draft revision; framing,
endpoint acceptance, record fidelity, namespace contents, cursor/restart and
credential behavior remain implementation-time proofs.

## Boundaries

Only Caddy stdout/stderr and `jaunder` namespace-internal bookkeeping are
initially uploaded. Do not add Collector Caddy ingestion,
whole-host/default-journal upload, body or extra credential capture, change
native header protections, alter Jaunder OTLP/privacy, change loopback/listeners
or native retention, or introduce an unsupported journal adapter. Keep both
managed vhosts in scope as applicable. Do not claim unproven runtime behavior.
