# Issue #2006: Discard html5ever's parser stepping records at the log bridge

## Outcome

Jaunder processes stop converting html5ever's TRACE and DEBUG `log` records into
tracing events. Those records describe the parser stepping through each
character, state and token. Without them, E2E captures, OTLP transport and
production VictoriaLogs lose about 98% of their log volume. html5ever's WARN and
ERROR records, and every other signal, still flow unchanged.

## Measured volume (run 44383337174, unsplit SQLite/Chromium, 516 tests)

| Signal                                | Bytes   | Records | Share of file |
| ------------------------------------- | ------- | ------- | ------------- |
| `otel-logs.jsonl` total               | 4.34 GB | 6.79 M  | 100%          |
| html5ever TRACE/DEBUG `log` records   | 4.26 GB | 6.71 M  | 98.1%         |
| `sqlx::query` DEBUG                   | 57 MB   | 38 k    | 1.3%          |
| `tower_http` request/response/eos     | 13 MB   | 31 k    | 0.3%          |
| `otel-traces.jsonl` total             | 172 MB  | —       | 100%          |
| html5ever span events (≤128 per span) | 70 MB   | 126 k   | 41%           |

Five html5ever 0.40 call sites produce 96% of the log file:

| Level and site                | Message                                     | Share of file |
| ----------------------------- | ------------------------------------------- | ------------- |
| tokenizer TRACE `mod.rs:725`  | `processing in state {:?}`, every step      | 33%           |
| tokenizer TRACE `mod.rs:293`  | `got character {c}`, every character        | 27%           |
| tree builder DEBUG `:324`     | `processing {token} in insertion mode {:?}` | 22%           |
| tokenizer TRACE `mod.rs:321`  | `got characters {d:?}`, every text run      | 8%            |
| tokenizer DEBUG `mod.rs:2761` | `processing EOF in state {:?}`, every parse | 6%            |

Most of the remaining html5ever records are `char_ref` stepping and the same
call sites in html5ever 0.39. html5ever logs at WARN in only a few places:
foster parenting, an unusual namespace and a missing serializer element record.
None of them fired in this run.

The lane's costs:

- `jaunder.service` used 7m02s of CPU and sent 3 GB to the Collector, and the
  Collector used 3m48s.
- 550 of 59,607 spans (0.9%) hit the SDK limit of 128 events per span, which
  silently discarded 1.24 M events. Almost all of them were html5ever, but some
  were real events that arrived after the limit filled: the tower_http
  `on_response`/`on_eos` events of 29 request spans, and later SQL events.

What each consumer reads:

| Consumer                                    | Reads                                                          |
| ------------------------------------------- | -------------------------------------------------------------- |
| `#[server]` flow coverage                   | span names, `uri`, `code.namespace`, parents                   |
| Boot-decomposition coverage, trace analysis | `e2e.test`/`e2e.page` spans and attributes, server span timing |
| Seed trace verifier                         | seed storage spans                                             |
| Zero-panic gate, failure triage             | `diag.log` (WARN+ and panics), journal                         |
| Human failure diagnosis                     | `otel-logs.jsonl` (sqlx, tower_http, app records)              |

No automated consumer reads `otel-logs.jsonl` or the server's span events.
html5ever never appears in `diag.log`.

## Approach and tradeoffs

The server, CLI and `test-support` install the `log` → `tracing` bridge in one
place, `host::telemetry`, where it is currently `LogTracer::init()`. That
becomes a small `log::Log` wrapper around the stock `LogTracer::new()`. The
wrapper reports a record as disabled when both of these hold:

- its target is `html5ever` or starts with `html5ever::`;
- its level is below WARN.

Otherwise it delegates to `LogTracer`. The wrapper applies one predicate at both
entry points:

- **`log()`**, which serves the plain `trace!`/`debug!` macros. Those macros
  never call `enabled()`, so `log()` checks the predicate itself before
  forwarding. Forwarding unconditionally would let about 74% of the volume
  through.
- **`enabled()`**, which serves html5ever's single `log_enabled!(Level::Debug)`
  guard. That guard sits in front of the one expensive message,
  `to_escaped_string(token)`.

`format_args!` is lazy, so a rejected record is never formatted and never
becomes a tracing event. It reaches no sink: no OTLP log, no span event, no
stdout line.

One shared function installs the bridge for production and for the test:
`log::set_boxed_logger`, then `log::set_max_level(Trace)` only when the install
succeeds. That is exactly what `LogTracer::init()` does, because host does not
enable tracing-log's `interest-cache` feature. The function returns
`SetLoggerError`. Production keeps its existing non-fatal stderr message, and
the test unwraps.

No stock option produces "html5ever below WARN" at the bridge:

- tracing-log's `ignore_crate` drops a crate at every level, WARN included.
- The `log` crate's `max_level_*` features apply to the whole build, so they
  would also drop `sqlx::query` DEBUG.
- A subscriber filter on the OTLP layers is the application-level filter that
  ADR-0222 forbids.

What stays:

- html5ever WARN and ERROR records.
- The Collector's html5ever redaction rule in `nix/otel-processors.yaml`, which
  still withholds the text of surviving WARN records. It also backstops native
  `tracing` output from a future html5ever, which the wrapper cannot see, but
  only for its three exact targets (`tokenizer`, `tree_builder`, `serialize`).
- Every other producer, at every level, with no sampling. That includes
  `sqlx::query` DEBUG and tower_http.

The Collector policy, the pipelines and the policy digest do not change.

Why at the source instead of the Collector:

- A source drop also removes the server's emission CPU and the 3 GB per lane of
  OTLP transport.
- It frees the 128-event limit on spans that render posts, so their real events
  stop being discarded.
- A Collector drop saves only the file, the hashing, the artifact egress and the
  Collector's redaction and export work.

**Accepted cost:**

- This is one named exception to ADR-0222's "no application-level severity,
  target or context filter".
- It reverses ADR-0224's "retain every occurrence at all severities / no event
  dropping" for html5ever records below WARN, `char_ref` DEBUG included. The
  bridge adds a prefix-matched drop qualified by level. The Collector's exact,
  non-dropping redaction is unchanged.
- Re-enabling parser stepping needs a code change.
- Jaunder maintains about 15 lines of wrapper code.

A new numberless ADR draft records the decision. ADR-0222's no-filter sentences
(lines 34 and 66) and ADR-0224's html5ever paragraph each point to it. These
docs get the same one-line exception, pointing to the draft
(`jaunder-adr-projection`):

| Doc                     | Lines     | Current wording                                 |
| ----------------------- | --------- | ----------------------------------------------- |
| `docs/observability.md` | 59-64     | no level/target/context filter                  |
| `docs/observability.md` | 141-146   | every occurrence survives; no parser dropping   |
| `docs/ARCHITECTURE.md`  | 2613-2616 | every occurrence at every severity; no dropping |
| `docs/ARCHITECTURE.md`  | 2683-2685 | no application-level filtering                  |
| `docs/DESIGN.md`        | 155-157   | parser events survive at every severity         |

## Acceptance

Proof to execute:

1. The existing all-levels test in `host::telemetry`
   (`actual_composition_retains_native_facade_and_parser_levels_under_every_local_policy`)
   changes in three ways:
   - It installs the bridge through the shared function, and drops its own
     `log::set_max_level` call.
   - It keeps its tracing-native `html5ever::tokenizer` emitters at every level.
     They prove the drop applies only to `log` records.
   - It also emits `log` records at every level for the targets
     `html5ever::tokenizer`, `html5ever::tokenizer::char_ref`, `html5ever` and
     the near-miss `html5everx`. It matches those rows by their `log.target`
     attribute.

   Under every local filter policy it must show:
   - html5ever `log` TRACE/DEBUG/INFO produce no native log record;
   - html5ever `log` WARN/ERROR are exported;
   - tracing-native html5ever records, `html5everx`, and every other `log` and
     tracing record are exported from TRACE through ERROR;
   - `log::log_enabled!(target: "html5ever::tree_builder", Level::Debug)` is
     false, and the same check at WARN is true.

2. A focused `dev e2e-local` spec that renders sanitized post HTML. Its capture
   contains html5ever TRACE/DEBUG records on `main` and none on this branch. The
   branch's capture still holds the `sqlx::query`, tower_http and application
   records, and the `request`, `web.*` and `storage.*` spans.
3. Existing policy and source-contract checks stay green, with
   `nix/otel-processors.yaml` unchanged.
4. The full E2E matrix stays green in CI. That includes server-fn coverage
   verification on the SQLite/Chromium pair, boot-decomposition coverage, the
   zero-panic gate and seed trace verification.
5. Before/after figures for the three SQLite/Chromium lanes, all read from each
   lane's `build.log`. "Before" is the latest green `main` run. "After" is this
   PR's CI run. They are recorded in the PR and on the issue, not committed:
   - `otel-logs.jsonl` and `otel-traces.jsonl` bytes, from the
     `capture-source-manifest` line;
   - result-lift time: "test script finished in" minus the VM uptime on the
     `otel-collector.service: Consumed` line;
   - CPU for `otel-collector.service` and `jaunder.service`, and the jaunder
     outgoing traffic, from the systemd `Consumed` lines;
   - Playwright duration, for context.

Deliverables to commit: the bridge wrapper and its test, the ADR draft, and the
ADR-0224, ARCHITECTURE, observability and DESIGN edits.

## Boundaries

- No change to other producers, levels, sampling, the Collector configuration,
  lane topology, caps or the archive/hash procedure.
- The mechanics of `diag.log`, mail and WebSub capture are unchanged.
- html5ever output emitted through native `tracing` (none today) remains covered
  only by the existing Collector redaction rule.
