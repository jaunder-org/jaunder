# ADR-0257: html5ever parser stepping records are dropped at the log bridge

- Status: accepted
- Date: 2026-11-20
- Issue: [#2006](https://github.com/jaunder-org/jaunder/issues/2006)

## Context

[ADR-0222](0222-independent-log-and-trace-filtering.md) admits every application
severity to the OTLP layers, with no application-level severity, target or
context filter. [ADR-0224](0224-native-otlp-process-logs.md) keeps every
occurrence of the exact html5ever targets (`tokenizer`, `tree_builder`,
`serialize`) at every severity, with the text withheld at the Collector and no
event dropping.

html5ever logs only through the `log` crate. Below WARN it reports every
character, tokenizer state and tree-builder token it processes. CI run
44383337174 (unsplit SQLite/Chromium, 516 tests) measured the cost:

- html5ever's TRACE and DEBUG records were 4.26 GB of the 4.34 GB
  `otel-logs.jsonl` (98%). They were 6.71 M of its 6.79 M records.
- Five call sites produced 96% of the file: "processing in state", "got
  character", "got characters", "processing EOF" and the tree builder's
  per-token message.
- The server used 7m02s of CPU and sent 3 GB to the Collector. The Collector
  used 3m48s of CPU, mostly to redact these records and write them to the file.
- 550 spans hit the SDK's limit of 128 events per span, which silently discarded
  1.24 M events. Parser records filled the limit, so some real events that came
  later were lost too: the tower_http `on_response`/`on_eos` events of 29
  request spans, and later SQL events.
- No automated consumer reads the log file or server span events. Once the
  Collector withholds their text, the records carry only a level and a source
  line. html5ever's WARN sites did not fire.

## Decision

The process's `log` → `tracing` bridge in `host::telemetry` wraps the stock
`tracing_log::LogTracer`. The wrapper discards a `log` record when both of these
hold:

- its target is `html5ever` or starts with `html5ever::`, which includes
  `char_ref`;
- its level is below WARN.

It applies this check in `log()`, which the plain `log` macros call without
`enabled()`, and in `enabled()`, which html5ever's `log_enabled!` guard calls. A
discarded record is never formatted and never reaches a sink: no OTLP log, no
span event, no stdout line.

Everything else is unchanged:

- html5ever WARN and ERROR records pass, and the Collector still withholds their
  text.
- The Collector's exact-target redaction rule is unchanged. It still covers any
  native `tracing` output from a future html5ever, which this wrapper cannot
  see.
- Every other `log` and `tracing` producer is admitted at every level, with no
  sampling.

This is the one named exception to ADR-0222's no-filter rule. It replaces
ADR-0224's "every occurrence at all severities, no event dropping" for html5ever
records below WARN.

No stock option does this:

- tracing-log's `ignore_crate` drops a crate at every level, WARN included.
- The `log` crate's `max_level_*` features apply to the whole build, so they
  would also drop `sqlx::query` DEBUG.
- A subscriber filter on the OTLP layers is the application-level filter that
  ADR-0222 forbids.

## Consequences

- E2E captures, OTLP transport and production VictoriaLogs lose about 98% of
  their log volume. The server stops spending CPU to emit the records, and spans
  that render posts keep their real events within the 128-event limit.
- Parser stepping cannot be re-enabled by configuration; it needs a code change
  to the wrapper.
- Jaunder maintains a small `log::Log` wrapper. Its all-levels test in
  `host::telemetry` pins which records are dropped and which pass.
- A new chatty `log` producer is still admitted at every level. Dropping another
  target needs its own decision.
