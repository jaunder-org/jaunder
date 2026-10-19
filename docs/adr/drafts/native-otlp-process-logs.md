# ADR-DRAFT: Native OTLP Process Logs

- Status: proposed
- Date: 2026-10-11
- Issue: [#1743](https://github.com/jaunder-org/jaunder/issues/1743)

## Context

[ADR-0011](../0011-unified-observability.md) gives native processes a shared
OTLP endpoint and process-owned telemetry lifetime.
[ADR-0196](../0196-single-host-nixos-deployment-stack.md) used a journald
receiver and JSON-body transformation before VictoriaLogs. Native OTLP logs
replace that deployment-specific ingestion path with canonical message, severity
and trace correlation through the same Collector as traces/metrics.

The earlier #1743 implementation used custom application projection and SDK
isolation. The qualified human approval JAU-1743-COLLECTOR-AMENDMENT-001 (“both
approved”, against the exact #1743 contract and latest decision card,
acknowledged by Lead) selects stock emission and shared Collector privacy policy
instead. The governing technical contract SHA256 is
`c5ee2ad723645acbff1cda627e3154282f95e468ff04991ae8a28cec879580f6`. Earlier
completed implementation/proofs remain historical evidence, not certification of
this replacement. This draft projects the implemented stock bridge and shared
policy. Focused subscriber/failure and both-backend Victoria proofs support
those seams; final integrated-tree coverage, browser-matrix reconciliation and
CI remain separate acceptance requirements.

## Decision

### Ordinary emission and separate local sinks

Use standard `opentelemetry-appender-tracing` 0.32.0 without experimental
metadata/span-map copying, ordinary SDK batching/providers and the existing
endpoint precedence/resource identity. Remove application privacy/admission
visitors, target-retention rules, Debug parsing, classifiers and the proposed
SpanProcessor decorator; do not relocate that framework behind another wrapper.

Without an endpoint, local stdout retains pretty/JSON formatting and local
filter precedence. With configured OTLP, ordinary logs are native records, not
mirrored to stdout/journald, even on logger setup failure. CLI stdout results,
local ErrorLayer/SpanTrace fields and independent WARN+/panic capture stay
separate. The delivered
[local-filter/AlwaysOn foundation](independent-log-and-trace-filtering.md)
remains semantic input: all application severities are offered regardless of
local verbosity, with valid sampled/unsampled-parent IDs. No new application
filter, synthetic span or invented correlation ID is selected.

### Shared Collector privacy policy

One shared production/test policy precedes `batch` and every durable log/trace
exporter; metrics are unchanged. The canonical shared fragment is
`nix/otel-processors.yaml` with `transform/jaunder_privacy` and
`error_mode: propagate`. Closed checker, module assertions for both backends,
launchers/VMs and source closures must agree on the effective merged pipelines;
array merging must not silently bypass the policy.

This **amends ADR-0011's pre-Collector privacy interpretation**: raw SDK data,
OTLP transport and trusted Collector input are accepted processing locations,
not durable export exceptions. Victoria/file outputs and persisted Collector
error/debug/export-failure diagnostics must exclude withheld data. Invalid
policy fails startup without raw fallback. Disable raw debug exporters/context
logging. `propagate` may reject/retry unrelated batch data and does not itself
redact diagnostics; actual synthetic failure-path proof is required.

Producer retention is qualified, not a union of safe-looking names. Ordinary
`instrumentation_scope.name` identifies the producer; only scope `log` uses
facade `log.target`. Conflicting attributes cannot broaden admission. This is
trusted instrumentation provenance, not hostile-producer authentication.

- Exactly `host::error` retains reviewed scalar/string error carriers:
  `error.kind`, `error.class`, `error.public`, `error.source`, `error.context`,
  `error.disposition`, `telemetry.origin`.
- Prefixes `host::`, `storage::`, `web::`, `jaunder::`, `test_support::` retain
  scalar `db.system`.
- Exactly `web::registration::api`, `web::password_reset::api`,
  `web::email::api` retain scalar integer or reviewed signed-decimal-string
  `user.id`; exactly `tower_http::trace::on_response` retains scalar integer or
  reviewed bare decimal-string `status`. Malformed/None/aggregate forms are
  absent. Preserve stock carriers, including exact overflow decimal strings;
  retire custom integer casts and synthetic Debug-Option parsing.
- Keep verified producer/location/level metadata. Discard unknown event fields,
  raw stacks (`error.span_trace` included), span/request/header/body aggregates
  and secondary message copies. Trace events use the same producer-qualified
  rules plus bridge target/level/location metadata.

At exactly `html5ever::tokenizer`, `html5ever::tree_builder`,
`html5ever::serialize`, retain every native log/trace-event occurrence at all
severities with context, replacing complete body/event-name text with
`HTML parser diagnostic; text withheld` and retaining only producer/location/
level metadata. No event dropping, classifier, prefix or source-line match;
`char_ref` remains outside this selection. Safe state/EOF detail and future
warning explanation are intentionally lost.

Other explicit messages and typed error carriers remain trusted under the
existing constructor/source discipline, not universally sanitized. Source-level
span admission under ADR-0011 and
[ADR-0156](../0156-sink-specific-telemetry-interfaces.md) remains intact. Clear
span status descriptions but retain codes. Preserve existing operation
names/determinants, IDs/times, resources, browser boot/test evidence and
metrics; no global span allowlist. Remove explicit raw secondary aggregates, not
safe paths. Current bridge links have empty attributes: preserve that and link
IDs, without a universal link sanitizer. New attributed-link/content-bearing
producers require review before inclusion.

### Correlation, failure safety and lifetime

Preserve actual explicit/contextual/root parent semantics and sampled/unsampled
IDs; no unrelated ambient SDK IDs for roots/missing context. Retire direct-log
`jaunder.request.uri` enrichment if it requires a bespoke Layer. Operators use
the log's correct trace/span IDs to find the linked request trace's safe path,
not timestamps. Root logs legitimately have no linked request. Standard
scope/facade producer fields replace `jaunder.target`. VictoriaLogs stores the
native producer as `scope.name`, the message as `_msg`, and correlation as
`trace_id`/`span_id`. The response fixture stores INFO severity as strings
`"INFO"`/`"9"` and status as string `"401"`; this store representation is not a
claim that native OTLP status is string-typed.

Providers remain process-owned with ordinary bounded shutdown and one-shot
draining. Telemetry failures cannot change application outcomes, disclose
credentials/raw endpoints/exporter payloads or report recursively through the
failing path. Exact private per-kind diagnostics, partial-rejection promotion,
coalescing and 256-byte guarantees retire, not this safety floor. Collector
policy cannot sanitize SDK stderr or prevent producer-side feedback.

The demonstrated composition retains only context and SDK-isolation mechanics:
an owning weak-dispatch event adapter resolves public OTel context for the stock
bridge; suppression-preserving Activation supports direct SDK parent/metric
context; a dynamic trace-layer gate excludes SDK-scope trace self-observation,
not every possible source of telemetry volume. The process-owned transport
runtime uses two workers/two blocking workers with the process-global
subscriber, not scoped-dispatch propagation. Repeated same-ID entries retain the
pinned latest-ID exit limitation; arbitrary entry-instance restoration and
worker containment are not promised. Guard shutdown is meter → tracer → logger →
runtime, unlike diagnostic scratch producer ordering.

`SafeSdkFormat` wraps both host fmt variants and the independent server WARN+
JSON writer. At target `opentelemetry-otlp`, exactly
`Tonic{Logs,Traces,Metrics}Client.{ExportFailed,PartialSuccess}` retain actual
level, target and name, the closed 17 tonic code names or signed scalar rejected
counts, and fixed `remote detail withheld` message/details. Selected records
omit secondary span copies. All other events delegate unchanged. This is not a
universal SDK sanitizer, global SDK hide, per-kind classifier or coalescer.
Opaque setup and nonrecursive shutdown fallbacks remain necessary. Source-input
taps belong only to tests, not production or diagnostic-output exemptions.
Bounded queues/shutdown do not guarantee delivery, containment of residual work
or volume recovery.

## Consequences and acceptance

Operators get one three-signal application transport and one shared production/
test privacy policy, while local stdout remains independent. Raw trusted
processing is broader; downstream privacy now depends on the effective policy
and safe persisted failure paths. Safe detailed parser wording, free-text status
descriptions, direct-log paths and private diagnostic distinctions are explicit
losses. Integer/string query types follow the stock bridge.

Focused public subscriber/lifecycle/failure tests and admitted synthetic
Collector cases exercise the selected seams; both-backend real Victoria fixtures
verify meaningful `_msg`, severity, producer identity and exact log span ID →
stored request span → URI/method/remote-parent linkage, without duplicate
ingestion. SQLite also exercises persistence. Those workloads do not establish
parser occurrence coverage for a workload that emitted none, a complete browser
matrix, authoritative source coverage or shipping readiness. Review precedes
final current-tree proof/CI. Keep existing resource, retry, time and
verification requirements. The normal commands and confidence split are in
[CONTRIBUTING](../../../CONTRIBUTING.md#running-e2e-tests-locally).

No production activation, merge, historical repair, storage Collector (#1753),
new public interface, universal producer sanitizer or capacity-recovery promise
is granted. CONTEXT.md is unchanged: no domain term changes. This draft stays
numberless/proposed through feature shipping; the architecture projection and
operator procedures describe current behavior without claiming unrun gates.
