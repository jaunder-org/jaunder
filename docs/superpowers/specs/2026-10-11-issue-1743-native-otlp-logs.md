# Issue #1743 — native OTLP logs and meaningful VictoriaLogs messages

## Outcome

Jaunder exports logs, traces and metrics through its configured OTLP Collector.
VictoriaLogs presents actual application messages rather than its
missing-message placeholder. Without OTLP, ordinary logs continue through stdout
in the existing configured format, supporting containers and bare-metal
operation.

## Load-bearing decisions

- The existing OTLP endpoint selection and precedence apply to all three
  signals; no VictoriaLogs-specific endpoint or dependency is added to Jaunder.
- Consume #1742's independent sink-admission contract rather than redesign it:
  `JAUNDER_LOG_FILTER`, then `RUST_LOG`, then existing ordinary/`--verbose`
  defaults (including storage INFO) govern whichever ordinary-log sink is
  active. No separate native-OTLP-log filter is introduced.
- `JAUNDER_TRACE_FILTER` independently controls traces/span events, defaulting
  to INFO; ordinary-log settings and `--verbose` do not alter it. #1742 owns its
  resolution, safe invalid-input fallback and per-layer wiring. Log `off` must
  not suppress traces or scoped diagnostics; trace `off` must not suppress
  ordinary logs, metrics or scoped diagnostics. Native records do not inherit
  trace policy.
- With OTLP configured, ordinary application logs use native OTLP records rather
  than stdout/journald. Do not mirror them to stdout or create duplicate
  ingestion. This does not redirect or suppress CLI command results written to
  stdout.
- The supported stack receives all three signals through its Collector's OTLP
  receiver. Replace its Jaunder journald log ingestion and JSON-body parsing;
  collecting arbitrary host journals is outside this contract.
- A record's explicit message is its human-readable body, not an opaque dump of
  structured fields or span stacks. Preserve severity, safe structured
  diagnostic fields and available trace/span correlation; do not fabricate
  absent context.
- Preserve existing privacy boundaries: credentials, request bodies, user PII
  and raw span stacks must not become OTLP log bodies or attributes. This
  explicitly excludes `error.span_trace`, even when it is an event field rather
  than inherited span data. Admit application messages and structured fields
  deliberately; generic Debug support is not telemetry admission. Audit newly
  exported process events.
- Retain the existing safe log target and request-path context used by stack
  queries, alongside approved error classifications and stable identifiers.
  Document any necessary field-shape change; do not silently discard diagnostics
  or promote arbitrary external-library fields into this operator interface.
- Correlation follows the event's resolved tracing parent: an explicit parent
  wins over the current span; contextual events use their active parent;
  explicit root events remain uncorrelated. Do not manufacture IDs for
  absent/filtered context.
- Logs share process-owned telemetry lifetime with traces and metrics. One-shot
  commands flush buffered records before exit; server shutdown releases all
  installed providers. Respect existing logging filters in either mode.
- Setup/export/flush failure remains best effort and must not change application
  results or exit status. Emit local stderr diagnostics without recursive OTLP
  reporting, stdout mirroring or unbounded buffering/retries. SDK/exporter
  failure events are ineligible for OTLP log export regardless of user filter
  settings; diagnostics use bounded, redacted classifications, not endpoint
  credentials or raw exporter payloads. This does not promise delivery during an
  outage.
- Keep server-scoped WARN+ E2E diagnostic capture independent of ordinary-log
  routing and log/trace `off`, with independent panic evidence and unchanged
  zero-panic acceptance. Consume #1742's sink-independent formatted span-field
  retention and ErrorLayer/SpanTrace behavior, including late-recorded fields;
  their local availability does not authorize raw-stack OTLP export.
- Record the change to ADR-0196's journal-based log topology in
  [the native OTLP logging decision](../../adr/drafts/native-otlp-process-logs.md)
  and project it into the architecture view. No domain-glossary change is
  needed.

## Acceptance

- Without OTLP, real process logs retain stdout formatting/filter behavior;
  normal CLI results remain usable. With OTLP, meaningful log messages reach the
  Collector without an ordinary-log stdout duplicate or journald collection
  dependency.
- Regression coverage exercises the supported real stack on SQLite and
  PostgreSQL, querying VictoriaLogs for deterministic safe application records
  with exact meaningful `_msg` values. Record existence or alternate message
  fields alone are not sufficient; verify severity, retained diagnostic fields
  and available trace/span correlation as well as existing credential-exclusion
  assertions.
- Prove provider lifecycle for one-shot flush and shutdown, filter behavior,
  non-fatal exporter failures, bounded redacted stderr diagnostics and recursion
  avoidance, including permissive user filters and repeated failures. Test
  explicit event parents, contextual events and root/absent-context cases.
  Regression coverage guards the absence of duplicate stack log ingestion.
- Prove the combined per-sink contract using native log/export observations: log
  WARN + trace INFO, log INFO/DEBUG + trace WARN, independent log/trace `off`,
  and target-specific directives. Metrics and independent WARN+/panic capture
  survive disabled log/trace admission; initial and late span fields remain
  available locally with the stdout sink absent. Do not widen OTLP admission.
- Existing trace/metric export and scoped E2E diagnostic/panic capture remain
  working; preserve their verification policies and resource budgets.
- Show comparable before/after default VictoriaLogs message presentation for a
  deterministic safe fixture, using the same query/state and viewport. Keep
  screenshots, traces and measurements in ignored/session evidence, not source.
- Maintain observability/deployment procedures for routing, message verification
  and failure behavior. Commit implementation, regression fixtures and current
  documentation; run evidence is not a maintained deliverable.

## Boundaries

No historical record repair, production deployment or production log injection.
No changes to underlying theme-upload errors, trace-parent warnings,
credentials, retention, authentication, storage schema, browser telemetry or
public application APIs. Native OTLP logging is the scope, not a generic log
forwarding framework. An implementation outline is required after spec approval
because this changes telemetry topology, process lifecycle and the privacy
boundary across host/stack.

## Coordinated prerequisite

[#1742](https://github.com/jaunder-org/jaunder/issues/1742) owns shared
log/trace filter resolution, per-sink admission, span-field retention and
trace-parent failure handling. #1743 owns native log records/export,
logger-provider lifetime, no-mirroring routing and removal of journal ingestion.
#1742 lands first; rebase #1743 onto its delivered contract before final
subscriber implementation and combined proof. Native logs consume that
interface, not a parallel filter implementation or a reversal of #1742's
independent diagnostics.
