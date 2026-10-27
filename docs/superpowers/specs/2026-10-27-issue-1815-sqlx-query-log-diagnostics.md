# Issue #1815: useful SQLx query logs in VictoriaLogs

## Outcome

SQLx query logs in VictoriaLogs include the SQL text and execution diagnostics
that SQLx actually emitted. Ordinary message-less query events show SQL text
instead of VictoriaLogs' missing-message placeholder; slow-query events retain
their original diagnostic message and expose the associated SQL and timings.

## Approach and tradeoffs

- Keep the stock Rust tracing-to-OTLP bridge and the existing OTLP ingress,
  shared Collector transform, batching, and VictoriaLogs export path.
- At the shared Collector log-policy boundary, admit SQLx query fields only for
  the exact `sqlx::query` producer, using the existing scope/facade provenance
  rule. Conflicting target attributes must not broaden admission.
- Retain strings `summary` and `db.statement`, debug-formatted duration strings
  `elapsed` and emitted `slow_threshold`, floating-point `elapsed_secs`, and
  stock integer `rows_affected`/`rows_returned` carriers (including exact
  decimal strings for unsigned overflow). Do not cast counters or parse duration
  text. Retain existing producer metadata, severity, resource identity,
  timestamps, and trace/span correlation without modification.
- For an absent or empty-string body, use non-empty SQLx `db.statement` as the
  body, otherwise non-empty `summary`. SQLx uses an empty statement field when
  its summary already contains the complete query. Preserve non-empty original
  bodies, including SQLx's slow-query diagnostic.
- Preserve emitted SQL bytes rather than sanitizing, truncating, fingerprinting,
  or reconstructing the statement. Do not capture bind arguments separately or
  claim that bound values are available when SQLx does not emit them.
- The user explicitly accepts that SQL text, including literals and identifiers,
  may expose private content or secrets in durable logs. This is a narrow log
  retention exception, not permission to export arbitrary fields, headers,
  request bodies, or additional trace-event/span attributes.
- A constant fallback would hide the symptom without restoring diagnostics.
  Changing the native bridge would add custom machinery when the Collector
  already owns producer-qualified durable retention. The accepted cost is more
  sensitive/larger operator logs and a SQLx-specific policy branch.
- Record the exception in a numberless ADR and the architecture view; update
  operator guidance and maintained policy validation with the implementation.

## Acceptance

The following proof is required during implementation; it has not yet been
executed. Existing Collector configuration validation alone is not ingestion
proof.

1. Drive real SQLx normal and slow query events through the native OTLP bridge
   and the shared Collector policy. Show the captured statements, execution
   durations and row counts match the executed fixture queries; use synthetic
   data only. A short query whose statement field is empty still shows its SQL
   via the summary. Compare the exported slow-query message byte-for-byte with
   the original `slow statement: execution time exceeded alert threshold`.
2. Query the resulting VictoriaLogs records: the normal query has SQL in `_msg`,
   SQL attributes remain inspectable for both normal and slow events, and
   severity and available trace/span IDs match the emitted records. No
   missing-message placeholder occurs for these real query events.
3. Keep regression coverage for actual producer selection, fallback precedence,
   preservation of existing messages, supported field carriers, and rejection of
   unrelated fields. Other producers and SQLx trace-event/span retention retain
   their existing policy; conflicting producer attributes cannot claim SQLx.
4. Run the existing Collector policy/config validation and affected targeted
   tests, then an actual Collector/VictoriaLogs smoke scenario. Store captures
   and temporary harnesses in ignored/session storage, not tracked reports.

## Boundaries

No general body fallback, SQL parser, query classifier, new filtering policy,
additional bind-value instrumentation, service-name correction, historic log
repair, or change to log routing/provider lifetime. Non-SQLx bodyless records
remain outside this issue. Existing unrelated working-tree changes are excluded.
