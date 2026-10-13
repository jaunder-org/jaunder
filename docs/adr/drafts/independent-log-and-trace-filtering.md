# ADR-DRAFT: Local log filtering and Collector-owned OTLP retention

- Status: proposed
- Date: 2026-10-12
- Issue: [#1742](https://github.com/jaunder-org/jaunder/issues/1742)

## Context

Shared subscriber filtering coupled local logging with OTLP tracing. Quiet
ordinary-log settings suppressed INFO request spans, while installed tracing
still enabled inbound parent assignment, producing misleading warnings and
incomplete traces.

A separately filtered OTLP layer can decouple verbosity, but requires another
operator policy and per-span admission bookkeeping. For the expected small
Jaunder installations, either local logging suffices without OTLP or a Collector
owns preservation decisions. Filtering individual spans before that Collector
can remove information it needs and fragment traces.

The operator selected unfiltered OTLP output instead of the earlier unlanded
independent-filter proposal. This draft records that selected decision; earlier
branch history preserves the rejected proposal, not a second accepted policy.

## Decision

Local ordinary logs retain `JAUNDER_LOG_FILTER`, then `RUST_LOG`, then
established ordinary/`--verbose` defaults, including the storage INFO exception.
Those controls govern local output only. `TelemetryConfig::local_log_filter()`
is the local-only accessor for subscriber reuse, resolved as immutable
process-periphery input under
[ADR-0158](../0158-peripheral-process-configuration.md).

When an endpoint is configured, the OTLP trace layer admits generated spans and
span events without application-level severity, target or context filters. There
is no `JAUNDER_TRACE_FILTER` or substitute transport-verbosity switch. No
per-span admission marker is needed for an installed unfiltered trace layer.

The SDK uses `Sampler::AlwaysOn` for Jaunder's spans. Valid incoming W3C trace
IDs and parent IDs are preserved, but the caller's unsampled hint does not
suppress Jaunder recording. The caller's own discarded spans cannot be
recovered. Collector-side sampling, filtering and retention are operational
decisions, not Jaunder verbosity policies.

An endpoint remains necessary for OTLP providers/export; without one, local
logging and diagnostics remain useful. Endpoint precedence, metrics, bounded
export buffering, setup/shutdown fallback and provider-flushing contracts remain
intact. Export is best-effort: unfiltered admission is not guaranteed delivery
under transport failure or abrupt process death. It covers instrumentation
present in the binary, not data a dependency/compiler never generated.

HTTP extraction/adoption is enabled by the installed tracer/subscriber
capability. Adoption precedes SDK context startup and retains safe fields;
without a tracer, requests do not attempt assignment. HTTP request IDs are
preserved or generated and propagated in `x-request-id` response headers, not
exported as span attributes. OTLP correlation uses top-level trace, span and
parent span IDs. Genuine SDK failures use bounded nonrecursive local
diagnostics. Missing or invalid parent headers remain normal root-context cases.

Error span stacks and safe initial/late fields do not depend on local stdout
admission. A distinct ErrorLayer field cache/updater retains them with pretty,
JSON or absent local output. Independent WARN+ diagnostics, panic capture and
slow-span reporting remain intact.

#1743 separately owns native OTLP logs, sink selection/no mirroring, logger
lifetime and removal of journal ingestion. Its native OTLP sink likewise admits
application logs at every severity, regardless of local filters; it must not
consume the local-only accessor. The implementation sequence remains #1742
first, then #1743 rebase and combined-signal proof.

Unfiltered output does not broaden permitted data classes. Privacy, redaction
and sink-specific field admission remain governed by
[ADR-0011](../0011-unified-observability.md),
[ADR-0147](../0147-decision-path-observability.md), and
[ADR-0156](../0156-sink-specific-telemetry-interfaces.md). Independent local
SDK, exporter, configuration, formatter, parent and shutdown diagnostics stay
outside failing/native export paths to avoid recursion.

### Implemented SDK isolation and lifetime boundary

A fixed, non-configurable per-OTLP-layer SDK-context suppression guard isolates
SDK self-observation, not application retention. Public context activation
preserves suppression when SDK work enters an already-admitted application span.
An owned private exporter-only runtime keeps transport workers and their
connection/reconnect descendants in suppressed context. Ordinary trace and
metric SDK builders use that runtime; ordinary application telemetry remains
unfiltered. Local diagnostics stay independent of this OTLP-only boundary.

`TelemetryGuard` shuts down the metric provider, then the trace provider, then
the exporter runtime, keeping transport available through both provider flushes.
Runtime destruction belongs to an ordinary owner thread so dropping the guard
inside an application async task does not destroy a Tokio runtime there. Runtime
shutdown uses a fixed five-second best-effort budget, with no configuration
knob. Unfinished blocking work can remain until it returns or the process exits,
including when the telemetry guard is dropped while the process continues.
Joining the runtime owner does not prove that work stopped; Tokio's shutdown API
reports no timeout outcome.

Activation callbacks identify spans, not entry instances. Repeated entries of
the same tracing span exited out of order can expose an interleaved external
OpenTelemetry context early, and can lose suppression after that external scope
exits. The pinned bridge has the same ordinary latest-span-ID exit behavior.
Distinguishable-context LIFO restoration is supported; arbitrary entry-instance
restoration is not promised. This accepted limitation is documentation, not a
new API restriction or enforced ordering rule.

## Consequences

Quiet local logging does not constrain Collector input. Operators budget
Collector storage, sampling and retention for the increased span/event/log
volume; Jaunder adds no application-level retention mechanism. Legacy local log
settings cease to be OTLP controls. AlwaysOn deliberately replaces parent-based
SDK sampling; upstream recording preferences do not govern this installation.

Proof must observe the actual production subscriber/provider, DEBUG/TRACE
admission, sampled and unsampled upstream correlation, independent local output,
field/diagnostic/privacy controls and short-lived export. Native all-severity
logs and combined no-mirroring/shutdown proof belong to #1743 once that sink
exists. Current operator guidance is updated with the implementation; no
Collector deployment policy is part of this decision.
