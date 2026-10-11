# ADR-DRAFT: Independent ordinary-log and trace filtering

- Status: proposed
- Date: 2026-10-11
- Issue: [#1742](https://github.com/jaunder-org/jaunder/issues/1742)

## Context

A shared subscriber filter currently controls both console output and OTLP
tracing. The ordinary filter suppresses INFO request spans, but an installed
tracer still enables inbound parent assignment. Attempting assignment to a
filtered span produces a misleading stderr warning for each request. Enabling
INFO eliminates that warning and restores request-parent correlation, but also
increases console output.

Quiet logs and useful distributed traces are distinct operator goals. Merely
suppressing the warning preserves the missing request spans. Raising the shared
filter collects them but makes console verbosity a prerequisite for tracing.
Independent sink policies provide useful tracing without forcing noisy logs, at
the cost of more exported telemetry and explicit configuration semantics.

## Decision

Ordinary-log admission and OTLP trace collection have independent filters. The
ordinary-log policy retains `JAUNDER_LOG_FILTER`, then `RUST_LOG`, then the
established ordinary or `--verbose` defaults, including the storage INFO
exception. It governs whichever ordinary-log sink is active, not trace
collection. #1742 keeps the current stdout routing; #1743 separately owns native
OTLP log export, logger lifetime and the stack's removal of journal ingestion.
That work consumes the same ordinary-log policy rather than the trace filter or
a new, independently resolved log filter.

`JAUNDER_TRACE_FILTER` uses the existing tracing filter grammar for OTLP traces
and their events. When unset, it defaults to INFO, independently of ordinary-log
settings and `--verbose`. Valid DEBUG, WARN, target-specific directives and
`off` are supported. Blank, malformed or unreadable inherited values emit a
fixed, safe local warning and use INFO; configuration handling remains
best-effort and does not fail the application. The raw configured value is never
included in that warning.

An OTLP endpoint remains necessary for export; trace filtering does not enable
an exporter by itself. Explicit `off` prevents trace export without disabling
metrics, ordinary logs, or server diagnostics. Ordinary-log `off` likewise does
not disable trace collection or scoped diagnostics. Existing endpoint
precedence, sampling and provider shutdown contracts remain unchanged. The same
policy applies to the server, production CLI commands and test-support.
Configuration remains immutable input resolved at the process periphery and
injected into telemetry, as required by
[ADR-0158](../0158-peripheral-process-configuration.md).

Intentional filtering is normal absence, not a trace-parent failure. Enabled
request spans still adopt valid inbound W3C parents. Genuine assignment failures
remain observable through the nonrecursive local fallback, with bounded reasons
and no header, token or arbitrary source-text dump. Ordinary absent or invalid
parent headers do not become assignment failures.

Filtering or replacing an ordinary-log sink must not remove the active span
stack or safely formatted, late-recorded fields required for error SpanTrace
capture. Span-field retention must not depend on the stdout formatter being an
active log sink. The independent WARN+ `diag.log` sink and panic fallback remain
intact. Field admission and privacy remain governed by
[ADR-0011](../0011-unified-observability.md),
[ADR-0147](../0147-decision-path-observability.md), and
[ADR-0156](../0156-sink-specific-telemetry-interfaces.md).

## Consequences

Default production tracing gains request/operation spans while console output
stays at its established verbosity. Operators budget collector storage and
retention accordingly; no new sampling or retention mechanism is introduced.
Existing log-filter overrides cease to be trace controls. Operators who want
tracing disabled or narrowed use `JAUNDER_TRACE_FILTER`, and the operator guide
must explain that compatibility change explicitly.

Proof must exercise actual subscriber composition: trace IDs/parent IDs,
independent ordinary-log and trace admission, filtered request handling, error
span fields, diagnostic capture and short-lived process flushing. Checking
resolver values or absence of a warning alone is insufficient.
