# ADR-DRAFT: Retain SQLx query diagnostics in native logs

- Status: proposed
- Date: 2026-10-27
- Issue: [#1815](https://github.com/jaunder-org/jaunder/issues/1815)

## Context

The [native OTLP process logs decision](../drafts/native-otlp-process-logs.md)
keeps the stock tracing-to-OpenTelemetry log bridge and qualifies retained
fields by producer at the shared Collector. The external-producer `keep_keys`
rule in `nix/otel-processors.yaml` currently strips SQLx query diagnostics. The
stock bridge maps tracing's `message` field to the native log body; it does not
invent a body from other attributes. Normal SQLx query events have no message,
so retaining only producer/location metadata leaves operators without either a
useful message or the query execution diagnostics.

SQLx 0.9.0's `sqlx-core/src/logger.rs` (`QueryLogger::finish`) emits at target
`sqlx::query`: a short SQL `summary`, `db.statement`, row counts, and elapsed
time. The statement contains the full SQL when it differs from the summary;
otherwise SQLx emits an empty statement string because the summary already
contains the SQL. Slow events additionally emit `slow_threshold` and the static
message `slow statement: execution time exceeded alert threshold`.

Literal SQL can contain sensitive text even without bound argument values. The
operator explicitly approved this exposure: "Yes, I want SQL text inclued in the
logs, accepting the exposure." This records that narrow choice, not a general
relaxation of telemetry privacy. The exposure choice is approved; implementation
of the design below remains pending approval. This draft does not claim that
runtime behavior or durable output has already changed.

## Decision

1. Retain query diagnostics for **native logs from exactly `sqlx::query`**.
   Resolve the producer using the existing Collector convention:
   `instrumentation_scope.name`, with facade `log.target` used only within scope
   `log`. A field with the same name from another producer is not admitted. Keep
   existing producer/location metadata and these SQLx fields with their existing
   bridge carriers:

   | Field                            | Retained carrier                                                 |
   | -------------------------------- | ---------------------------------------------------------------- |
   | `summary`, `db.statement`        | Strings, including SQLx's empty statement string                 |
   | `rows_affected`, `rows_returned` | Stock integer carriers, including exact overflow decimal strings |
   | `elapsed`                        | SQLx's debug-formatted duration string                           |
   | `elapsed_secs`                   | Floating-point seconds                                           |
   | `slow_threshold`                 | SQLx's debug-formatted duration string, when emitted             |

   Do not parse duration strings, cast counters, reconstruct statements, or
   invent query identities. Retention does not add fields SQLx did not emit.

2. For a message-less SQLx log, use the available nonempty `db.statement` as the
   log body; if SQLx emitted an empty statement, use `summary`. Keep the SQL
   attributes as well. Preserve an explicit body unchanged, including SQLx's
   slow-query diagnostic message; the slow record's SQL remains in attributes.
   Do not substitute a generic placeholder, suppress the event, or synthesize
   SQL from absent data.
3. Own both the narrowly qualified retention rule and body population in the
   canonical shared Collector policy, `nix/otel-processors.yaml`, before batch
   and durable log export. Adjust the external-producer keep rule so it does not
   erase the admitted fields. Keep the stock native log bridge rather than
   replacing it with an application visitor or custom appender merely to fill
   this producer's body. The Collector already owns producer-qualified privacy
   and the durable-output boundary; this is the smallest ownership change.
4. This exception permits literal SQL in these **log attributes and the derived
   log body only**. It does not expand trace-event or span retention, capture
   additional bind values, change local/native severity or target filtering,
   change exporters, or broaden any other producer's retained fields. All other
   privacy, correlation, transport, and failure-path rules remain unchanged.

## Consequences

- Operators can inspect the SQL and execution measurements SQLx actually emits,
  instead of a bodyless diagnostic stripped to its producer. Normal records show
  SQL as the body; slow records retain their warning explanation alongside the
  SQL attribute.
- Durable log bodies and attributes may expose sensitive literal SQL. This is an
  expressly accepted exception to the prior SQLx log-field retention policy, not
  a claim that SQL text is safe, sanitized, or free of secrets. Existing policy
  against exposing other withheld data, including persisted Collector
  failure/debug diagnostics, still applies.
- Stock bridge conversion and the shared Collector remain the only owners needed
  for this behavior; no replacement application log bridge, new query
  identifier, additional bind capture, or trace-retention exception is
  justified.
- Implementation and its evidence remain pending approval. The implementation
  must demonstrate normal and slow events, SQLx's empty-statement fallback,
  carrier preservation, exact producer qualification, unchanged other-producer
  and trace retention, and stored message/attribute behavior. This documentation
  change supplies no runtime or ingestion proof.
