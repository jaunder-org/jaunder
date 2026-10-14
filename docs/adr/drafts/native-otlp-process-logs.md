# ADR-DRAFT: Native OTLP Process Logs

- Status: proposed
- Date: 2026-10-11
- Issue: [#1743](https://github.com/jaunder-org/jaunder/issues/1743)

## Context

[ADR-0011](../0011-unified-observability.md) gives native processes a shared
OTLP endpoint and process-owned trace/metric lifecycle. Its initial log path
used stdout formatting.
[ADR-0196](../0196-single-host-nixos-deployment-stack.md) adds a
deployment-specific journald receiver and JSON-body transformation before
VictoriaLogs ingestion. The reported deployment stores useful text in alternate
fields while its default message field shows a missing-message placeholder. That
journal-based topology motivates a native application OTLP logger.

A field-selection repair alone would preserve different collection paths for the
same application's three signals. Native OTLP records can carry a canonical
message, severity and trace correlation without depending on systemd or a
particular log store. This changes the log-ingestion portion of ADR-0196; its
service composition, ingress, persistence and other ownership boundaries remain.

## Decision

Native process telemetry exports logs, traces and metrics through the same
configured OTLP endpoint, using existing configuration precedence and process
resource identity. Jaunder does not connect specifically to VictoriaLogs; the
Collector owns routing to each store.

Without an OTLP endpoint, ordinary logging retains stdout formatting and
filtering. With an endpoint configured, ordinary application logs use native
OTLP records, without stdout mirroring or journal-based duplicate ingestion. CLI
result output and server-scoped E2E diagnostic capture remain separate
consumers, not alternate ways to re-export ordinary logs.

Consume the delivered Collector-owned retention foundation from
[#1742](https://github.com/jaunder-org/jaunder/issues/1742). The 2026-10-12
operator-selected revision supersedes this draft's earlier shared active-sink
filter policy: `JAUNDER_LOG_FILTER`, then `RUST_LOG`, then established
ordinary/verbose defaults (including storage INFO) govern local ordinary output
only. The delivered `TelemetryConfig::local_log_filter()` accessor is
local-only, not an OTLP logger input. Native logs offer every application
severity without application-level severity, target or context filtering. #1742
owns unfiltered spans/events and AlwaysOn recording despite a clear upstream
sampled hint, retaining valid trace/parent IDs. No separate OTLP log filter is
added.

The Collector owns preservation; local disablement cannot disable OTLP output,
metrics or independent WARN+/panic capture. Shared span-field retention and
ErrorLayer/SpanTrace do not depend on active stdout. Unfiltered severity is not
blanket field admission. #1742 has landed and native logging consumes its
delivered foundation, without a competing subscriber-policy implementation.

Explicit event messages become human-readable log bodies. Severity and approved
structured diagnostic fields remain available, including safe
target/request-path context. Event parent semantics govern available trace/span
IDs: explicit parent, contextual parent, or explicit root. Native export is not
permission to serialize arbitrary values. The privacy rules of
[ADR-0011](../0011-unified-observability.md) and the admission principles of
[ADR-0156](../0156-sink-specific-telemetry-interfaces.md) remain authoritative;
raw span stacks and `error.span_trace` are excluded from OTLP logs. Newly
exported process events require a privacy review rather than blanket Debug
serialization.

The process guard owns every installed signal provider and flushes/releases them
at ordinary shutdown, including one-shot commands. Telemetry remains best
effort: setup, export and shutdown failure cannot change application results.
Report failures locally to stderr using bounded redacted classifications, never
by re-exporting SDK/exporter, setup/shutdown, parent-assignment or field-format
fallback diagnostics through OTLP logs or restoring ordinary stdout mirroring.
Local event filters must not clamp native records or this non-recursive failure
path. Bounded buffering does not guarantee delivery during outages or hard
process termination.

The supported single-host stack consumes logs through its existing OTLP receiver
and forwards them to VictoriaLogs. Remove its Jaunder journal receiver and
journal-JSON transformation; do not expand into whole-host journal collection.
Canonical message presentation, severity, safe structured context, correlation,
privacy and single-ingestion behavior require actual-stack proof; source
integration alone does not certify ingestion or presentation.

### Implemented admission and lifetime boundaries

The native projection retains `jaunder.target` and path-only
`jaunder.request.uri` from the reviewed HTTP request span. The closed event
profile admits the `host::error` carrier's `error.kind`, `error.class`,
`error.public`, `error.source`, `error.context`, `error.disposition` and
`telemetry.origin`, application scalar `db.system`, audited numeric `user.id`
from credential-consumption producers, and Tower response-completion `status`.
Unknown fields and arbitrary Debug/aggregate/inherited-span serialization are
not admitted. Typed error-source and message constructors still owe ADR-0011's
safe-text discipline; closed admission is not blanket text sanitization.

Missing parent/tracer context and explicit roots use a span-free SDK emission
scope so ambient context cannot fabricate correlation. SDK-suppressed events and
self/fallback targets remain ineligible before emission. The delivered
activation foundation retains its pinned same-span limitation: repeated entries
of one span exited out of order can expose interleaved external context early
and lose suppression after that scope exits. Arbitrary entry-instance
restoration is not promised; this adds no caller-ordering restriction.

The logger shares the process resource and private suppressed exporter runtime.
Guard shutdown runs meter → tracer → logger → runtime. Logger shutdown uses the
pinned SDK five-second default without an unbounded pre-shutdown flush; runtime
shutdown has a separate five-second best-effort budget. These are not a
whole-process bound. Acknowledgment proves neither delivery nor containment;
residual blocking export work may outlive a timeout or guard destruction.

Initialization resolves a private owned SDK `BatchConfig` snapshot and passes it
as a complete `with_batch_config` replacement. The pinned public processor
builder constructs a discarded environment-derived default before replacement.
This constructor-only third-party read cannot change effective policy; no
application callback, worker, reconnect or teardown rereads effective policy.
This is the narrow accepted SDK limitation, not a literal zero-third-party-read
claim, public configuration extension or general environment reader. Ordinary
pinned batching supports queue size, batch size and scheduled delay, not the
experimental async export-timeout feature. SDK default queue/batch bounds are
2048/512 records; saturation can drop logs.

New logger failure diagnostics are fixed redacted stderr lines, once per kind
per process and below 256 bytes. Export errors retain failure classifications;
positive partial rejection fails, while zero-rejection warnings remain
successful with a fixed warning diagnostic. No raw exporter or partial-success
text is rendered. Queue-drop classification may be observed at shutdown rather
than synchronously. The failure path never feeds native logs or restores stdout.

The stack and shared E2E harness implement OTLP log routing. Harness logs use a
separate `file/logs` exporter and `otel-logs.jsonl`; existing trace capture,
independent WARN+/panic diagnostics and whole-directory retention remain
unchanged. Real-stack, browser and combined-signal runtime acceptance remain
separate proof obligations, not consequences of these source contracts. The
numberless decision remains proposed through feature shipping.

## Consequences

Container and bare-metal stdout deployments continue to work without OTLP.
Configured deployments share one application-to-Collector interface across all
three signals. Operators must use OTLP log queries rather than depend on
ordinary Jaunder log copies in the service journal, and failure diagnostics
remain local.

Adding all-severity logs creates lifecycle, privacy and volume responsibilities.
Audit newly visible DEBUG/TRACE producers; do not serialize arbitrary payloads,
create exporter recursion or disclose credentials through span traces. Operators
manage preservation and potentially increased volume at the Collector, not by
application verbosity. Preserve scoped capture and #1742's revised trace/metric
behavior. SDK version compatibility and real ingestion proof are required, not
assumptions from the presence of a logger API.

Historical log rewriting, production rollout, runtime observability credentials,
retention changes, browser telemetry and the underlying application errors
behind individual messages are outside this decision. CONTEXT.md is unchanged
because no domain term or relationship changes.
