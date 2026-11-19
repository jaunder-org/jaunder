# Issue #2012 — surface missing traces in every CI lane

## Outcome

When a CI job's `dev` workflow leaves no readable trace, or a diagnostics upload
finds nothing or fails, the run carries an annotation (a warning for a missing
trace or empty upload, the action's error for a failed upload) in every
validation, E2E and performance job alike. No diagnostics step changes a job's
result.

## Approach and tradeoffs

`CONTRIBUTING.md` (Observation and traces) already promises that "missing files
and upload failure warn visibly without changing the workload verdict". Only the
hermetic lane does this today; the others use `if-no-files-found: ignore` and a
failed upload would fail the job. This change brings the workflows up to the
documented contract with two existing mechanisms:

1. **Trace summary step warns when there is no trace.** Each job already runs
   `dev trace summary --markdown` after its workload, and that command already
   exits non-zero (`no traced runs in .xtask/run`) when the directory holds no
   trace. Today `|| true` discards that. Replace it with an
   `|| echo "::warning::..."` fallback saying the trace summary is unavailable
   and to see the step log. The wording is neutral because the same fallback
   also fires on an unreadable trace or a `nix develop` failure; the precise
   cause is already on the step's stderr. This is the "expected trace directory
   absent" signal: an upload that bundles `.xtask/diagnostics/` with
   `.xtask/run/` finds _some_ files when only the trace is missing, so the
   upload alone cannot report it.
2. **Every trace/diagnostics upload uses `if-no-files-found: warn` and
   `continue-on-error: true`.** An empty upload then warns, and a failed upload
   shows the action's own error annotation while the job's result is left to the
   workload. Applies to the host, test-checks, coverage, Chromium E2E, Firefox
   lane trace and Firefox reconciliation uploads in `ci.yml`, and the trace
   upload in `performance.yml`. The reconciliation upload moves from `error` to
   `warn`: its log is always written by the preceding shell redirect, so only
   its upload-failure behaviour really changes.

The Firefox **lane evidence** upload (`.xtask/diagnostics/`, downloaded by the
reconcile job) is producer evidence, not a diagnostic. It moves from `ignore` to
`warn`, so an empty upload is surfaced, but keeps failing the lane on upload
failure ("fails visibly"): its verdict role is unchanged.

The hermetic lane's extra "Warn on hermetic diagnostics upload failure" step and
its now-unused `id:` are removed: with `continue-on-error` the failing upload
already annotates the run, so a per-lane echo step would be duplicated machinery
in seven places.

Alternative rejected: only switching `ignore` to `warn`. It misses the case the
issue names (dev crashed before its root span, other diagnostics present) and
leaves upload failure able to fail the job.

The two storage-snapshot uploads keep `ignore`, with a comment: the snapshot is
written by a best-effort `continue-on-error` probe step and may legitimately be
absent.

## Acceptance

- The existing structural workflow test
  (`assert_jobs_keep_and_summarise_their_trace` in
  `tools/host-proof/tests/workflow_callers.rs`), which covers every validation,
  E2E and performance job, additionally requires each job's trace upload to have
  `continue-on-error: true` and `if-no-files-found: warn`, and its summary step
  to end in the `::warning::` echo fallback, not `|| true`.
- The same test runs each extracted summary command under
  `bash -e -o pipefail -c` with a stub `nix` on `PATH` that exits non-zero, and
  observes a `::warning::` line on stdout and exit status 0.
- A workflow test asserts the Firefox lane evidence upload uses
  `if-no-files-found: warn` and has no `continue-on-error`.
- `tools/traced-run/tests/hermetic_diagnostics.rs` keeps its hermetic-verdict
  assertions (the `dev ci-validate-hermetic` run line and the aggregate's
  `needs.validate-hermetic.result` check); only the removed step's assertions
  and its upload-failure test go.
- The PR's CI run is green, showing the job results are unaffected.

## Boundaries

- No change to `dev trace summary` or tracing code.
- The host resource baseline and performance result uploads (not trace uploads;
  `if-no-files-found: error`) are unchanged.
- No new aggregate check or required job; warnings are annotations only.
- Warning text avoids `": "` and `" #"` so the folded YAML `run:` scalars stay
  valid.
