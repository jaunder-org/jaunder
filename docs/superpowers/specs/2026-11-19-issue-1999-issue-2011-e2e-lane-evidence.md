# E2E lane evidence: producer cache state and per-test results on timeout (#1999, #2011)

## Outcome

- **#1999.** The trace of a `dev e2e-producer <lane>` run (alone, or inside
  `dev e2e`) records `jaunder.cache` (`reused` / `built` / `substituted`) for
  the lane's producer output, as every other Nix build step does. The verifier
  build that follows it is a `dev` step with its own cache value, so no Nix
  build in the E2E workflow goes unrecorded.
- **#2011.** A lane whose Playwright run is killed (timeout, exit 124) still
  has, in its retained `build.log`, one line per finished test attempt naming
  the project, file:line, title, outcome, retry and duration.

## Approach and tradeoffs

### #1999 — `dev` probes around the build-and-lift step, then verifies

Today `dev e2e-producer` runs `bash scripts/e2e-workflow producer <lane>` as one
plain step. The script builds `.#checks.x86_64-linux.e2e-<lane>` with
`--keep-failed`, `--log-lines 50`, `--json` and a tee'd `build.log` (needed to
recover a failed build's kept diagnostics), lifts the lane's artifacts, and then
runs the verifier from a second `nix build .#host-proof` of its own.

Change:

1. `steps.py` factors the probe `nix_build` already does into one helper:
   evaluate the top-level output path and check its validity before the step; if
   the step succeeds, record `reused` if it was valid before, else `built` if
   `nix path-info` marks it `ultimate`, else `substituted`; a failed query
   records nothing. `nix_build` keeps exactly its current rule through the
   helper, and a plain step may name the installable it realizes to get the same
   recording.
2. The script's `producer` mode becomes build-and-lift only: `producer <lane>`,
   with its build flags, keep-failed recovery and lift rules unchanged. Its
   `evidence()` helper, verify/regenerate tail and `action` argument are
   removed.
3. `dev e2e-producer` runs that step with the lane check as its probed
   installable, then — only when it succeeded, as today — builds the verifier
   with `steps.nix_build("host-proof", ".#host-proof", keep=False)` (what
   `dev e2e-reconcile` already does) and runs it as a further step against the
   output held by `.xtask/gcroots/e2e-<lane>`: `server-fn-coverage verify` for
   `sqlite-chromium-unsplit`, `verify` for every other lane.
   `dev e2e-regenerate-coverage` does the same with `regenerate`. The report
   rendering and argv the verifier gets are unchanged.

So failure order is preserved (a verifier build problem still surfaces only
after artifacts are lifted), and the verifier path needs no new hand-off: `dev`
owns both the build and the call, as for reconciliation.

Rejected, per the issue's "less total machinery" rule:

- _Script writes the attribute_ through `JAUNDER_TRACE_DIR`: duplicates the
  classification in bash and adds a child-to-step record contract.
- _Move the whole build into `dev`_: `nix_build` would need a stderr tee into
  `build.log`, `--json` handling and the kept-directory recovery ported to
  Python.

Accepted cost: one extra `nix eval` of the lane check per producer step. On a
dirty local tree that is a full evaluation per lane (eight under `dev e2e`) —
the same cost every `nix_check` step already pays. Lane checks are
`nonSubstitutable`, so a lane records `reused` or `built`; `substituted` stays
possible for the verifier.

### #2011 — Playwright's built-in `list` reporter

The lane's Playwright stdout is printed into the build log
(`nix/checks.nix:603`) after `machine.execute` returns, which it does on a
timeout too, and `scripts/e2e-workflow` tees that log to
`.xtask/diagnostics/e2e-<lane>/build.log` on every path; CI uploads it. That is
the one carrier already known to survive a failed build. The configured `line`
reporter, though, prints a line only when a test _starts_ (`[n/N] title`).
Playwright's built-in `list` reporter, without a TTY, appends one line when each
attempt _ends_: status mark (passed / failed or timed out / skipped), the index,
`[project] › file:line › title`, a retry suffix and the duration, with failure
detail inline as `line` prints it.

Change: replace `["line"]` with `["list"]` in `end2end/playwright.config.ts` and
update the comments that call it the line reporter. All other reporters and the
JSON reports are unchanged. Local runs through `host-proof e2e-local` pass their
own `--reporter=html,line` and are unaffected.

Rejected: a custom JSONL reporter writing into `test-results/`. Files the VM
writes land in the NixOS test driver's `$out`, which the script's kept-failed
recovery (`$kept/emit-out/diagnostics`) does not appear to reach, so on a
timeout the file would likely be lost unless new recovery machinery were added.
Structured output would only matter to a consumer, and none is planned. Accepted
limitation: lines carry durations but no wall-clock timestamps, and one `✘` mark
covers failed, timed-out and interrupted attempts (the inline failure detail
tells them apart).

## Acceptance

Proof to execute (recorded in the PR, not committed):

1. **Forced timeout.** One local `dev e2e-producer <lane>` run with only that
   lane's `playwrightTimeout` temporarily lowered (uncommitted, kept well above
   the census preflight), so Playwright is killed mid-run. Its
   `.xtask/diagnostics/e2e-<lane>/build.log` has one `list` line per finished
   attempt. Every named test is in that lane's selected population, and the
   count of finished attempts is consistent with the run being cut short. The
   trace records no cache value for the failed build.
2. **Cache state.** A successful `dev e2e-producer <lane>` run's trace
   (`dev trace summary`) shows `jaunder.cache` on the `e2e-producer` step and on
   the sibling `host-proof` step; a repeat run shows `reused` for both.
3. **Unchanged success path.** That run lifts the same files and its verifier
   report is the same as before.

Deliverables to commit:

- `tools/dev` tests (fake Nix, given a `path-info` branch) for the producer
  step: `reused`, `built`, `substituted`; no value on a failed build or failed
  query; the verifier built and run only after a successful build-and-lift;
  regeneration only through `dev e2e-regenerate-coverage`. Update the pinned
  argv expectations in `tools/dev/tests/test_workflows.py` and the existing
  `test_e2e_workflow.py` cases to the new split.
- `CONTRIBUTING.md` trace section drops the "`scripts/e2e-workflow`'s own builds
  record none" exception; `docs/ARCHITECTURE.md` cache paragraph and the
  `checks.nix` reporter comments match.

## Boundaries

- No change to the script's build flags, keep-failed recovery, lift rules or
  verdicts; no change to server-function coverage capture or reconciliation.
- Making the Playwright output reach the CI job log live, rather than in one
  burst at the end, is out of scope.
- No new reporter, lifted file or verifier consumer.
- No other per-lane phase spans.
