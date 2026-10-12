# ADR-DRAFT: Native OTLP Process Logs

- Status: proposed
- Date: 2026-10-11
- Issue: [#1743](https://github.com/jaunder-org/jaunder/issues/1743)

## Context

[ADR-0011](../0011-unified-observability.md) gives native processes a shared
OTLP endpoint and process-owned trace/metric lifecycle. Logs use stdout
formatting. [ADR-0196](../0196-single-host-nixos-deployment-stack.md) adds a
deployment-specific journald receiver and JSON-body transformation before
VictoriaLogs ingestion. The reported deployment stores useful text in alternate
fields while its default message field shows a missing-message placeholder.
Application logs are not currently exported by an application OTLP logger.

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

Consume the Collector-owned retention foundation to be delivered by
[#1742](https://github.com/jaunder-org/jaunder/issues/1742). The 2026-10-12
operator-selected revision supersedes this draft's earlier shared active-sink
filter policy: `JAUNDER_LOG_FILTER`, then `RUST_LOG`, then established
ordinary/verbose defaults (including storage INFO) govern local ordinary output
only. The proposed `TelemetryConfig::local_log_filter()` accessor is local-only,
not an OTLP logger input. Native logs offer every application severity without
application-level severity, target or context filtering. #1742 owns unfiltered
spans/events and AlwaysOn recording despite a clear upstream sampled hint,
retaining valid trace/parent IDs. No separate OTLP log filter is added.

The Collector owns preservation; local disablement cannot disable OTLP output,
metrics or independent WARN+/panic capture. Shared span-field retention and
ErrorLayer/SpanTrace do not depend on active stdout. Unfiltered severity is not
blanket field admission. #1742 lands first; native logging consumes the
delivered foundation after rebasing, without competing subscriber-policy
implementation.

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
privacy and single-ingestion behavior are tested through the actual stack.

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
