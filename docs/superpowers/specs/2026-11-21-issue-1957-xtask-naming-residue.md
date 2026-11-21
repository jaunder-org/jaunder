# Issue #1957: remove xtask naming residue

## Outcome

Nothing outside the listed historical records and `.xtask/` run storage names
`xtask` any more. That covers configuration, Nix source filters, warning
prefixes, identifiers, test fixtures and comments. The remaining doc drift in
the issue is corrected too.

## Approach and tradeoffs

The `xtask/` tree no longer exists, so this is deletion and renaming. No
behavior changes except the warning text.

- **Configuration.** Drop `/xtask` from `.github/dependabot.yml`. In
  `nix/checks.nix`, remove the `/xtask/` exclusions from the coverage, clippy,
  `rust-format` and `static-code` source filters. No tracked path has an `xtask`
  component (`.xtask` does not match `/xtask`), so the filters select the same
  files. In the `static-code` asserts, drop the trivially true
  `"/source/xtask" "directory"` assert. Keep the other two with a neutral path
  outside the workspace members: CSS is still admitted
  (`/source/unlisted/theme.css`), and Rust there is still excluded
  (`!… "/source/unlisted/src/lib.rs"`).
- **Warning prefixes.** Each warning is named after the binary that prints it.
  `traced-run` already does this:
  `traced-run: warning: traced-run.stream_summary: …`.

  Each warning keeps its subsystem segment; only the `xtask` names change:

  | Site                           | New warning key                                                           |
  | ------------------------------ | ------------------------------------------------------------------------- |
  | `coverage` `run.rs:110`        | `coverage-check: warning: coverage-check.coverage.failure_dump`           |
  | `coverage` `probe.rs:185`      | `coverage-check: warning: coverage-check.coverage.probe_worktree_cleanup` |
  | `pr-observer` `gh.rs:383`      | `pr-observer: warning: pr-observer.pr.rate_limit_reset`                   |
  | `e2e_provenance.rs:156`        | `e2e-local: warning: e2e-local.e2e.provenance`                            |
  | `e2e_local.rs:241`             | `e2e-local: warning: e2e-local.e2e.workers_config`                        |
  | `adr-promoter` `github.py:183` | `adr-promoter: warning: adr-promoter.pr.rate_limit_reset`                 |

  No program parses these warnings. Tests pin the `coverage` and `pr-observer`
  keys (`run.rs:329`, `probe.rs:468`, `gh.rs:570`), and those tests change with
  them. No test pins the `host-proof` or `adr-promoter` prefixes, so for those
  sites the search below is the only proof.

- **Identifiers.** In `coverage` `probe.rs`, `AdmitsXtaskSource` becomes
  `AdmitsHostToolSource`, and the matching local variable, field and test names
  change with it. The `repository-policy` census stops approving an `xtask` root
  (`snapshot.rs:56`). Its doc comment at `:50` changes from "four" to "three"
  approved trees, and the test at `:165` becomes a negative assert. Fixtures are
  changed according to what each one checks:
  - `census/history.rs` tests approved paths, so it moves to the approved
    `tools/` root.
  - `doctests` `emit.rs:140` guards exclusion, so drop the `"xtask"` disjunct.
  - The `xtask/Cargo.toml` entry in `tests/adr_promoter_package.rs` has no role
    in the test, so delete it (array length 10 → 9).
  - In `wasm_sections.rs`, the producer bytes become neutral test bytes.
- **Doctests path collapse (user decision).** `doctests::check::ScannedFile`
  keeps a second `run_path` only for `--manifest-path xtask/Cargo.toml` runs.
  Production always sets it equal to `path`. Remove the field, key matching on
  `path`, and delete the two xtask-only tests and the `libtest.rs` comment. The
  change also covers `harness.rs:86`, the `file()` test helper and its callers,
  and the "both spellings" comments in `check.rs` and `emit.rs:70`. The simpler
  option of only rewriting the comments would keep machinery that has no caller.
- **Comments.** `repository-policy/src/git.rs:52` should explain the cwd hazard
  without naming xtask. In `reduce-otel-capture.mjs`, the header should give the
  script's real path and a placement reason that still holds against today's
  filters.
- **Doc drift.** `coverage-check.rs`: `probe-source` runs on request, not in CI,
  which matches CONTRIBUTING. `operations.rs`: the test-pattern check prints
  `test-backend-pattern` but is invoked as `test-patterns`. Rename the token to
  `test-backend-pattern` so it matches the name used in the docs, CONTRIBUTING
  and the module, and update `tools/dev` and the tests. CONTRIBUTING: the
  `validate-no-e2e` gate row and the prepush parity table get the platform
  contract (`validate-no-e2e` runs it, `prepush` does not). The other original
  gate-table items are already accurate in the rewritten CONTRIBUTING.

## Acceptance

Proof to execute:

- `rg -n --hidden -g '!.git' --pcre2 '(?i)(?<!\.)xtask(?!_untracked)'` returns
  only the intentional survivors listed under Boundaries.
- The `drvPath` of `rust-format`, `static-code`, the clippy check and the
  coverage derivation is identical before and after the change, which shows the
  source closures did not change.
- `dev source-contract test-backend-pattern` passes, and the old `test-patterns`
  token is rejected.
- The focused tests pass: `coverage`, `pr-observer`, `host-proof`, `doctests`,
  `repository-policy`, `tools/dev` and `adr-promoter`.
- Evaluating those derivations succeeds, so the asserts hold. `dev prepush`
  passes.

Deliverables: the source edits above. No new docs or reports.

## Boundaries

Intentional survivors, left unchanged:

- `.xtask/` run storage and the code that writes it, including the `ux-sandbox`
  `workspace.rs` local.
- `JAUNDER_XTASK_UNTRACKED_INPUTS`.
- ADRs, ADR titles in `docs/README.md`, and ADR links.
- Retirement notes in `CONTRIBUTING.md`, `docs/ARCHITECTURE.md`,
  `tools/pr-observer/README.md` and `docs/application-assets.md`.
- Planning documents under `docs/archive/` and `docs/superpowers/`, including
  this spec until it is deleted at cleanup (user decision).
- The CI log fixtures in `pr-observer` `shared_failure.rs`.

Renaming `.xtask/` is out of scope. So is moving the front-door tests (already
resolved) and changing anything beyond the warning text.
