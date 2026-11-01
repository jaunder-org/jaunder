# Local xtask Invocation Traces in Perfetto

## Outcome

One focused local `cargo xtask test-local` run writes a unique checkout-local
Chrome Trace Event JSON file that opens in Perfetto UI. Rust `tracing` is the
sole timing/instrumentation path for the selected command; no Collector or
network service is required. The first path is
`cargo xtask test-local -- -p storage site_config_primitives_round_trip`,
including xtask startup/preparation and immediate host phases. This is a first
local view, not completion of all #1856 acceptance: subprocess internals and
invocation-ID propagation are deferred.

## Approach and tradeoffs

Enable tracing capture only for `test-local`. Initialize `tracing-chrome` plus
`tracing-subscriber` in synchronous `Threaded` style after CLI parsing and
before checkout preparation. Buffer this bounded host-orchestration trace until
the checkout root is known. `Threaded` records begin/end (`B`/`E`) events on the
same thread, with `ts` in floating-point microseconds; it does not emit `dur`.
Instrument only synchronous scopes entered and exited on one thread. Child
process spans measure host-observed wall time through child completion, not
child internals. Do not imply CPU time, async task lifetime, or causal parentage
from visual nesting. Concurrent scopes use separate thread tracks; Chrome slices
on one track must remain properly nested. The adapter writes a top-level JSON
array (not an object with `traceEvents`); args are omitted unless
`include_args(true)` is enabled, and included fields are string-valued. Match
`B`/`E` by `(pid, tid)` and per-thread nesting order using stable, unique phase
span names. `ts` is floating-point microseconds, not an integer duration; derive
result milliseconds by subtracting paired timestamps and rounding down to the
existing precision.

Instrument preparation and immediate phases: checkout admission, hook setup,
dispatch, artifact-lock wait (before acquisition), CSR-bundle admission/Nix
build and validation, image-worker build, and the `devtool pg run`/Nextest
invocation. Use stable phase names and record a bounded outcome event for each
phase. Remove the matching `Instant`/`StepResult::with_duration` timers for
these phases. Match nested `B`/`E` events by thread and stack order; calculate
elapsed microseconds from their timestamps, then project the same measured
intervals into the existing in-memory `CommandResult`, human output, stdout
`--json`, and GitHub summaries. Round down to the existing integer-millisecond
result precision. Keep `CommandResult.duration_ms` at its existing dispatch
boundary: it starts after checkout preparation and hook setup. Add a nested
dispatch span at that boundary and derive the field from it; the outer command
span and preparation phases remain separately visible in the trace. Do not
retain a second timer for these intervals. Other commands and their result
timing are unchanged. The outer `cargo xtask` launcher/build is outside this
Rust process and is not measured.

The Perfetto Rust SDK adapter is deferred in favor of the simpler direct JSON
path; it has a managed session stop/drain/write lifecycle and its Rust project
is community-maintained with the documented support/stability/feature caveats
versus official C++. No native SDK or parallel instrumentation path is added.

## Capture finalization and artifact

Write a unique file at `.xtask/traces/<run-id>/trace.json`, print its exact
path, and retain each run until the user removes it. Do not overwrite prior runs
or add a pruning framework; local disk growth is an explicit tradeoff. Use the
`tracing-chrome` arbitrary-writer API with a shared in-memory writer so spans
can begin before the checkout root/output path is resolved. The exact
Cargo.lock-resolved adapter version and its writer/configuration API are not yet
verified; confirm arbitrary writer, `Threaded`, and `include_args` support
against that version before implementation. If unavailable, return to spec
approval rather than adding a second timing adapter. On success and handled
early-error paths: close every measured scope; emit the terminal command
outcome; drop/join the `FlushGuard`; validate the complete top-level JSON array
and expected matching begin/end pairs; derive selected result durations; attempt
fallible temporary-file/write/flush/rename persistence; then report existing
outputs and exit with the original command verdict. Reporting still occurs if
persistence fails. This is a small capture-finalization path, not a general
artifact framework.

The guard does not return a reliable write/finalization result; do not treat its
drop alone as proof. If the buffer is malformed, expected spans/outcome are
missing, or file persistence fails, emit
`xtask-trace: status=unavailable reason=<closed-reason>` on stderr and preserve
the actual command verdict. On success emit
`xtask-trace: status=complete path=<path>`. A command failure still produces a
trace when checkout root is known. If a selected phase interval is missing or
unmatched, keep the existing `duration_ms = 0` default, emit a capture warning,
and do not recover timing with a second timer; zero can also represent a real
sub-millisecond interval. If checkout-root resolution itself fails, report
capture unavailable and do not write elsewhere. Abrupt kill or power loss may
prevent finalization; unhandled termination is not promised to produce a
readable trace.

## Result and sidecar behavior

The selected trace contains bounded phase span names and one terminal
`xtask.command.outcome` event (`command`, `outcome`, `exit_code`, and handled
`exit_override`, if any). Phase outcome events contain only stable phase name
and success/failure/skipped state. Do not export argv, environment, cwd, raw
process output, `StepResult.detail`, or free-form failure text.

Keep `.xtask/last-result.json`, in-memory `CommandResult`, stdout `--json`,
human output, and GitHub summaries unchanged for now. For `test-local` only, the
existing sidecar/result durations are populated from the same tracing intervals;
no duplicate manual timing is retained. Sidecar removal and all
artifact-consumer, CI-upload, documentation, and external-policy migrations are
deferred improvements, not prerequisites to this MVP. Manager owns any future
external canonical policy/skill migration before schema/path removal. Do not
export PR, coverage, flaky, or unrelated result payloads into this initial
trace; they remain available through existing result contracts. There is no new
general result-event/query API.

## Acceptance

- Host tests cover nested same-thread intervals, two controlled concurrent
  threads on separate tracks, lock-wait visibility, interval matching/rounding,
  command preparation and phase outcomes, early `Err`, handled `exit_override`,
  malformed/missing trace events, file write/flush/rename failure, and explicit
  guard drop before `process::exit`. A sequential real run need not contain
  overlapping sibling phases.
- Confirm `test-local` result durations are derived from trace `B`/`E` pairs and
  matching manual timers are removed; all existing result-output contracts and
  exit codes remain unchanged.
- Run the representative test through `devtool run`, then open its exact trace
  in `https://ui.perfetto.dev` using the intended browser. Verify local-file
  selection/drop and that preparation/phase nesting, durations, and
  separate-thread concurrency are understandable. Browser capability has not
  been verified; hosted file loading is not an offline-only guarantee.

## Boundaries and deferred improvements

No subprocess instrumentation/propagation, child-internal timing, CPU or
human-effort attribution, async-task migration guarantee, full-suite phase
instrumentation, all-command timing, result-payload export, sidecar removal,
CI/external-policy migration, automatic pruning, viewer packaging, or native
Perfetto SDK. These are deferred improvements, not MVP prerequisites. Manager
owns any future external canonical policy/skill migration before sidecar/path
removal; this cycle makes no external edits. This slice does not satisfy the
issue's parent → child → nested-tool acceptance and does not close #1856.

The current draft authorizes no implementation, test execution, tracker claim,
commit, merge, or viewer startup. **Decision requested:** approve or correct
this bounded MVP. Spec approval follows a fresh soundness review; outline
approval and implementation authority remain separate gates. Any future sidecar
removal is deferred until Manager completes external-consumer coordination.
