# Issue #2056 — a test kills an instrumented child while it writes its profile

## Outcome

The coverage run no longer gets a truncated `.profraw` from a test that SIGKILLs
an instrumented child just as the child exits normally. Coverage tolerance is
unchanged: `llvm-profdata merge` still fails on any corrupt profile.

## Cause

An instrumented process writes its `.profraw` in LLVM's exit hook. A SIGKILL
before that hook means no profile, and nobody notices. A SIGKILL during the
write leaves a short file whose header claims more data than the file holds.
`llvm-profdata` rejects it with "file header is corrupt", so `text-report` fails
(merge-queue run 45050851412).

The CI artifact cannot name pid 15127. The binaries are gone, so the module
signature in the file name maps to nothing, and nextest does not log pids.
(`source` is the build directory name, not a package. `_3` is pid mod the
runner's 4 CPUs, which rules out a forked writer.) An audit of every kill site
in the covered crates found three that can hit a child during its exit:

1. **`jaunder::integration misc::media_handlers::real_worker_response_bound_rejects_and_reaps_before_cleanup`**
   (confirmed). The `oversize` worker fixture writes 128 KiB + JSON and returns,
   so the real `jaunder` worker exits. The owner reads the bounded prefix,
   rejects it with `Protocol`, and at once SIGKILLs the child it thinks is still
   running. Local probe under `cargo llvm-cov` env, idle machine: in 60 of 60
   runs the worker left no profile, because the kill came before the exit hook.
   With a temporary 0.5 ms delay before `start_kill`, the worker left a
   420,744-byte file (a full one is 685,832 bytes), and `llvm-profdata show`
   printed exactly "invalid instrumentation profile data (file header is
   corrupt)". CI's 6256-test load moves the kill time around in the same way.
2. **`ux-sandbox::lifecycle tests::further_interrupt_forces_early_but_preserves_initiating_term_status`**
   (same class, not reproduced in 40 local runs). After the fixture parent
   writes `parent-drained`, the test sends a second INT, which makes
   `ux-sandbox` SIGKILL the group. But the parent writes that marker _before_ it
   exits, so the force can land during the parent's profile write.
3. **`ux-sandbox::lifecycle tests::runtime_io_failure_is_reported_after_owned_teardown`**
   (same class, found in spec review). The fixture parent makes the runtime file
   a directory, so `ready()` fails and `ux-sandbox` stops the group with zero
   grace: TERM and then SIGKILL at once. The fixture's TERM handler returns and
   the parent exits normally, which races that SIGKILL.

Every other kill site in the covered crates hits an uninstrumented program
(`sleep`, Chromium) or a child parked mid-run. A parked child loses its profile,
but the file is never truncated.

## Approach and tradeoffs

Fix the two tests so that a kill never meets an exiting instrumented process.
Product code is unchanged.

- **Worker fixture.** The `oversize` arm writes its response and then parks,
  like the existing `pause` arm, until its owner kills it. That makes the
  rejection path kill a live child every time, which is the hostile-worker case
  the bound exists for. The worker's exit-path coverage cannot drop: its profile
  is already lost on every idle local run, and the `oversize` arm stays covered
  in-process by its unit test.
- **EarlyForce.** Before the forcing INT, the test waits until `ux-sandbox` has
  reaped the fixture parent, not just until the parent writes its marker. The
  force then hits only the resistant descendant, which is what the test asserts
  about (exit status 143, finishing in under 5 s).
- **Runtime-IO fixture.** After the fixture parent creates the blocking
  directory, it parks and never installs a TERM handler. The default TERM action
  then ends it without running exit hooks, so the zero-grace stop never meets an
  exiting process. The test asserts only on the error report, the reaped pid and
  the removed storage, and none of those depend on the parent draining.

Alternatives rejected:

- `--failure-mode=warn`, i.e. tolerating corrupt profiles. It weakens assurance,
  and the issue prefers fixing the source.
- LLVM continuous mode (`%c`). It is crash-safe for the whole class, but on
  Linux it needs runtime counter relocation, and cargo-llvm-cov does not support
  it. That means new toolchain machinery for two tests.

What we accept: a future test that kills an exiting instrumented child would
bring the problem back. If that happens, file it then; no generic guard now.

## Acceptance

- The oversize integration test passes. The existing unit test
  `oversized_control_emits_valid_json_after_the_bounded_prefix` changes to go
  through the release predicate, as the pause test does; with `|| false` it
  would hang. It shows that the arm writes the bounded-prefix-plus-JSON
  response, then parks and returns only on release. That is the regression lock:
  the fixture cannot exit on its own.
- With the same local probe (`cargo llvm-cov show-env`, separate
  `LLVM_PROFILE_FILE` per run) and the temporary 0.5 ms delay re-applied, no
  `jaunder` worker profile is written and no profile is corrupt. Run it, don't
  commit it; record the results in the PR.
- The EarlyForce and runtime-IO tests pass. EarlyForce sends its forcing INT
  only after the parent pid is gone, and the runtime-IO parent installs no TERM
  handler. Only code review can check these two orderings; there is no practical
  regression test for them.
- No change to population reconciliation, `pushFilter`/ADR-0252, merge failure
  mode, or `status.json` semantics.

## Boundaries

No coverage-harness, cargo-llvm-cov or `coverage-sandbox` changes. No product
change to `UploadPreparer`'s kill-on-rejection policy. No new generic guard.
