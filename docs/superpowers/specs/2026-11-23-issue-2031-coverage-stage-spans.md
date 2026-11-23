# Coverage producer stages as trace spans (#2031)

## Outcome

When `dev coverage` (or any gate that composes it) builds the Rust coverage
output, the run's trace shows each producer stage (workspace resolution, profile
cleanup, test census, instrumented test run, population reconciliation, text,
LCOV and CRAP reports) as a span at its true position under the coverage step's
`build` span. The producer's `diagnostics/` (and so the CI
`validate-coverage-diagnostics` artifact) also carries Cargo's timings report
for the census build.

## Approach and tradeoffs

The producer, `coverage-sandbox emit`, runs inside the `checks.coverage` Nix
build. It cannot join the `dev` trace from there: the sandbox has no
`JAUNDER_TRACE_DIR`, and a trace written in the build directory is discarded.
`dev` therefore builds the stage spans on the host, after the build, from the
`status.json` the output already carries.

1. **Stage start times.** Today `status.json` records only each stage's
   `duration_ms`. `emit` will also record `started_unix_ms`, the wall-clock
   start, at the single point where it records a duration. `validate()` requires
   the two fields to appear together: both or neither, and neither on a
   `not-run` stage. The version stays at 1, following the precedent of
   `duration_ms`, which was also added under version 1. The only parsers are the
   producer's own pre-write validation and `coverage-gate`'s `validate-status`,
   and both run against the same derivation graph, so the stricter rule rejects
   no file anyone keeps.
   - _Alternative considered:_ leave the schema alone and stack the durations
     back to back, ending at the build's end. Rejected because every span would
     shift by the untimed install phase and inter-stage gaps (#1981 measured
     about 15 s of install).
   - The times come from the Nix builder's clock. A coverage output is always
     built locally (`preferLocalBuild`, `allowSubstitutes = false`). An output
     copied in from elsewhere reads as `reused` and gets no stage spans, so a
     foreign clock never reaches the trace.
2. **Spans in `dev`.** A small, generic helper in `tools/dev` takes a parent
   span context plus recorded phases (name, start, duration, ok/failed). It
   writes them as finished spans with explicit start and end times, using the
   existing record writer, so OpenTelemetry accepts the already-ended `build`
   span as their parent. The Nix build step keeps the `build` child's span
   context and gains an optional phase source. Only when `jaunder.cache` is
   `built` does it read the phases from the output and write them under `build`,
   inside the same never-fail suppression as the cache query. `coverage` passes
   a reader for `$out/status.json`.
   - The reader checks only `stages[].stage`, `outcome`, `started_unix_ms` and
     `duration_ms`, where the last two must be non-negative integers. A
     malformed or missing file emits nothing, all or nothing. A stage without a
     start (`not-run`) gets no span. `ok` means `outcome == "success"`, and
     every other outcome is `failed`.
   - A `reused` output gets no stage spans, because its stages ran in an earlier
     build. Nothing here changes a step's status or output.
   - #2030 (E2E lane phases) can call the same helper with its own parent
     context. Its `e2e-producer` step is not a `nix_build`, so the helper is not
     tied to `nix_build`. #2030's own plumbing stays out of scope.
3. **Census timings.** The census command (`cargo nextest list`) gains
   `--timings` (bare, which produces HTML; the pinned nextest 0.9.143 lists the
   flag). `emit` copies `<target-dir>/cargo-timings/cargo-timing.html` into
   `diagnostics/`. The target directory is the one nextest reports in the census
   JSON, because `show-env` may redirect it. The copy is best-effort on both the
   success and the evidence-error paths. When the census yields no parseable
   target directory, the report is simply missing, like any other
   `write_diagnostic` gap. It is never a stage failure.

**Decision record.** ADR-0254 limits step accounting to attributes on existing
span records. Writing spans afterwards from producer evidence, on a clock other
than `dev`'s, is a new kind of span that #2030 will reuse. A short draft ADR
(`jaunder-adr`) records the rule: a `dev` step may write after-the-fact child
spans from the timed evidence of an output it built in this run, and never from
a reused output.

## Acceptance

Proof to execute:

- `dev` tests on the existing fake Nix store (whose output is `built`) show:
  - a valid `build-out/status.json` yields one `build` child per started stage,
    at `started_unix_ms` with `duration_ms`, marked `ok`/`failed`;
  - a `reused` output, a `not-run` stage, and a missing or malformed file yield
    no stage spans, and the step's status is unchanged.
- Rust unit tests show that the duration recorder sets `started_unix_ms` too,
  that `validate()` rejects a start without a duration and a duration without a
  start, and that the census command carries `--timings`.
- One real `dev coverage` run, the first build of the changed derivation, where:
  - the merged `.xtask/run/<id>.trace.json` (span ids in `args`) has the eight
    stages parented to `coverage`'s `build`, with durations equal to
    `status.json`;
  - `dev trace summary` shows `build`'s untraced time (the Nix overhead);
  - `.xtask/gcroots/coverage/diagnostics/cargo-timing.html` exists.

Deliverables to commit: the code and tests above, the draft ADR, and
CONTRIBUTING's "Observation and traces" prose updated. It currently says only
`csr-prerequisite` joins a run and that in-sandbox work is invisible beyond the
cache result; it will describe the stage spans and the timings file.

## Boundaries

- No change to coverage verdicts, `coverage-gate`, `coverage-policy`, CI
  workflow files or the artifact's upload paths (`diagnostics/` is already
  uploaded).
- No timings for the instrumented test run's rebuild or for other producers. The
  E2E lane phases (#2030) and the Elisp producer stay separate.
- No OTLP export and no producer-side span writing inside the sandbox.
