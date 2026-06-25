# Coverage pipeline → maintainable internal Rust tool

**Date:** 2026-06-24
**Status:** Approved design (pre-implementation)
**Tracking (immediate CI failure, separate track):** #28

## Context

CI's `Validate` job (`cargo xtask validate`) recently failed and was read as a
"coverage regression." It was not: the real cause was **disk exhaustion** on the
runner (PostgreSQL SQLSTATE `53100`, "No space left on device") during the heavy
`jaunder-coverage` Nix derivation, which made three Postgres-backend tests panic,
which failed the `nextest` pass, which failed the derivation (exit 100), which CI
surfaced opaquely as a coverage-step failure. Three layers of misattribution
(tracked in #28).

That misattribution is a symptom of the coverage tooling's structure:

- `scripts/check-coverage` (~330 lines of bash/`awk`/`jq`) still owns the
  in-sandbox `--emit` orchestration and a legacy percent-based gate.
- `xtask/src/coverage/` (Rust) owns a newer, more sophisticated **gap-based**
  host-side gate (`coverage-baseline.json` = accepted-uncovered lines, healed
  host-side) plus the CRAP gate.
- The two models coexist: the `coverage-update` derivation (flake.nix:809) still
  emits the **vestigial** percent manifest `.coverage-manifest.json` via the
  shell path, while the host gate uses the gap model.

This design migrates the coverage pipeline into maintainable Rust, collapses the
dual models into one, makes failures self-describing and their first-hand data
preserved/exfiltrable, and folds in four related open issues.

## Goals / non-goals

**Goals**
- Replace `scripts/check-coverage` (bash/`awk`/`jq`) with Rust.
- Stand up an extensible **internal sandbox tool** (`devtool`) that the other
  in-sandbox scripts can migrate into later.
- Collapse the dual coverage models into the single gap-based Rust model; delete
  the legacy percent path.
- Fold in #2 (failure attribution), #7 (CRAP-heal JSON drift), #3 (working-tree
  robustness), #11 (diffmap line shifts since baseline).
- Preserve first-hand failure data and exfiltrate it as CI artifacts.

**Non-goals (this iteration)** — each gets a follow-up issue:
- Migrating `scripts/with-ephemeral-postgres`, `scripts/seed-e2e-fixtures.sh`,
  `scripts/audit-wasm-bundle`, `scripts/analyze-otel-traces`,
  `scripts/run-e2e-trace-analysis` into `devtool`.
- Changing the e2e VM checks.
- The actual disk-exhaustion fix for #28 (Track 1, separate).

## Crate architecture

Three pieces, honoring the existing `/xtask/` cache-exclusion boundary in the
coverage derivation (so that frequently-edited host gate logic does not bust the
expensive in-sandbox coverage build cache):

- **`coverage` library crate** (new workspace member, *not* under `xtask/`):
  all reusable logic — report/CRAP parsing types, path normalization, the
  gap-based baseline model, classify, diffmap. Written once, used by both sides.
- **`devtool`** (new bin crate, *not* under `xtask/`, so it is cache-eligible in
  the sandbox): the internal sandbox tool. Subcommands:
  - `coverage emit` — implemented now.
  - `pg …`, `seed-e2e …` — not implemented now; placeholders tracked by
    follow-up issues.
- **`xtask`** (host, stays excluded from the coverage cache): keeps `validate`
  orchestration + the host-side gate/heal/report, now calling into the shared
  `coverage` lib instead of its private `xtask/src/coverage/` modules.

**Trade-off:** moving parse/classify into the shared lib means the sandbox build
cache busts when that lib changes. Acceptable: the emit step genuinely must live
in-sandbox and changes rarely; the frequently-edited host gate logic stays in
`xtask` (excluded).

## The emit pipeline (`devtool coverage emit`)

A behavior-preserving port of `check-coverage --emit` to Rust:

1. Clean stale profraw (keep instrumented build).
2. Run `cargo llvm-cov nextest` under an ephemeral PostgreSQL (for now still via
   `scripts/with-ephemeral-postgres`; that script's migration is a follow-up).
3. `cargo llvm-cov report --text` and `--lcov`.
4. `cargo crap`.
5. **Path normalization in Rust** — replaces the bash `while-read`/parameter
   expansion and the `awk`/`jq` strip of the Nix-sandbox absolute prefix.

It emits the same `$out` artifacts the host gate reads (`coverage-report.txt`,
`crap-report.json`), plus the diagnostics bundle below.

## Producer / consumer / host-gate layering

Belt-and-suspenders, three layers:

1. **Producer derivation** (the renamed/restructured `jaunder-coverage`): runs
   `devtool coverage emit`. It **always succeeds and always realizes `$out`**,
   capturing the result rather than `set -e`-dying on a failed test pass. `$out`
   contains the reports, the diagnostics bundle, and a **`status.json` sentinel**
   reflecting what is knowable in-sandbox: `category ∈ {tests-ok, test-failure,
   infra}` with evidence.
2. **Consumer derivation** (tiny `runCommand` depending on the producer `$out`):
   reads `status.json` and `exit 1` on a bad sentinel. An independent Nix-level
   red for in-sandbox failures even if `xtask` is bypassed (e.g. a direct
   `nix flake check`).
3. **Host gate** (`xtask`, via the `coverage` lib): reads the producer `$out` and
   computes the **full categorized verdict**, including coverage/CRAP
   regressions — which are inherently host-only (they need the committed
   baselines + git context unavailable in the sandbox). Decides final pass/fail
   and category; heals baselines in `Mode::Fix`.

Coverage: in-sandbox failures (test/infra) are caught by both the consumer and
the host gate; coverage/CRAP regressions are caught by the host gate (the only
place they can be computed).

Caching note: the producer keeps the existing `pushFilter`
(`jaunder-coverage|jaunder-e2e`) so its test results are never substituted from
cachix — tests always re-run. The consumer is trivial and re-runs with it.

## Failure attribution (#2 + #28 infra axis)

Categories, with where each is decided:

- **infra** — emit scans tool output / PG errors for `ENOSPC` / SQLSTATE `53100`
  / OOM. Decided in-sandbox (sentinel) and re-affirmed host-side. Reported as an
  infrastructure failure, never blamed on tests/coverage.
- **test-failure** — `nextest` reported failed tests. Decided in-sandbox
  (sentinel, with the failing-test list as evidence); re-affirmed host-side.
- **coverage-regression** — emit succeeded and tests passed, but the host gate
  found real coverage/CRAP regressions. Host-only.

`xtask validate` surfaces the category in its `StepResult`/summary so CI logs and
the run summary say *which* class of failure occurred.

## Line-map reference frame (#3 + #11, unified)

**Verified Nix behavior (2026-06-24):** the report is built by Nix from the
**working tree** — dirty edits to *tracked* files ARE included (the "Git tree is
dirty" warning fires and the coverage `.drvPath` changes after a tracked edit);
only *untracked* files are excluded. In CI the checkout is clean, so the working
tree equals HEAD there. (An earlier assumption that the report came from
committed HEAD was disproven and the corresponding memory note deleted.)

**Root cause (shared by both issues):** the baseline's gaps are numbered at the
**anchor commit** (the commit that last healed `coverage-baseline.json`), but the
host gate builds its line-map from `git diff HEAD` — whose *start* point is HEAD,
not the anchor. So line shifts in commits between the anchor and HEAD misalign
the gaps and manufacture phantom regressions.

- #11: lines shift across the **commits since the baseline** was healed; a
  HEAD-anchored map ignores them entirely.
- #3: the "commit-first" friction *is* that same anchor<HEAD misalignment — once
  the map starts at the anchor, the friction disappears (subsumed by the #11
  fix), with no separate working-tree handling needed.

**Fix:** build the diffmap from the **baseline anchor commit → working tree**:
`git diff <anchor> --` (a single commit arg diffs the anchor against the working
tree, so it spans both committed shifts since the anchor *and* any uncommitted
edits — matching the working-tree report). In CI this reduces to anchor→HEAD
automatically. The current `git ls-files --others` untracked special-case is dead
under this model (untracked files never reach the report) and is removed.

This is the **riskiest** section; it is sequenced last and isolated behind its
own tests.

## CRAP-heal JSON drift (#7)

The compact-JSON churn the issue describes **appears already resolved** in the
Rust port (`xtask/src/coverage/mod.rs:193-203`): the heal writes pretty-printed,
key-sorted JSON and compares via a normalized (formatting-independent) form. The
migration will:

1. Verify this with a regression test (heal is idempotent; no spurious rewrite).
2. Close the residual **line-attribution drift** as part of the #3/#11
   reference-frame work (CRAP entries are keyed partly by line; stale line
   numbers are the same root cause).

Net: #7 likely closes via confirmation + a regression test rather than new heal
logic.

## Failure-data preservation & exfiltration

**Canonical diagnostics dir + machine-readable status.** `devtool coverage emit`
always writes a **diagnostics bundle** into `$out`:

- captured `nextest` output,
- `llvm-cov` / `crap` stderr,
- the ephemeral PostgreSQL server log,
- a disk-usage snapshot (`df`, `target/` and PG-datadir sizes),
- the raw + normalized reports,
- `status.json` (the sentinel + evidence).

Observation becomes a structured read, not a 498 KB log grep.

**Backstop for catastrophic infra (ENOSPC).** If the disk is too full to write
the bundle, `xtask`'s `build_check` passes `nix build --keep-failed` and, on
derivation failure, copies the kept build dir's diagnostics + the nix log to a
host path `.xtask/diagnostics/<check>/`.

**CI wiring.** Add an `actions/upload-artifact@v4` step with `if: always()` to
`.github/workflows/ci.yml`, uploading `.xtask/diagnostics/` and the gcroots
reports, with a sane retention period. Every failing (and optionally passing) run
leaves a downloadable bundle.

**e2e note:** the same `.xtask/diagnostics/` convention gives the later e2e-script
migrations (Playwright traces, `analyze-otel-traces`) a home — designed-for, not
built now.

## Nix wiring & deletions

- `checks.coverage` producer buildPhase: `bash ./scripts/check-coverage --emit`
  → `cargo run -p devtool -- coverage emit`. Drop `gawk` / `jq` from
  `nativeBuildInputs`.
- Add the consumer `runCommand` derivation that gates on the sentinel.
- `coverage-update`: re-baseline routes through `devtool`; **delete the legacy
  `.coverage-manifest.json` percent path**.
- **Delete `scripts/check-coverage`** and the dead percent model.

## Follow-up issues to open

`tooling`-labeled issues for migrating into `devtool`:

- `scripts/with-ephemeral-postgres`
- `scripts/seed-e2e-fixtures.sh`
- `scripts/audit-wasm-bundle`
- `scripts/analyze-otel-traces`
- `scripts/run-e2e-trace-analysis`

## Testing

- Unit tests in the `coverage` lib (TDD): text-report parser, path normalization,
  diffmap reference-frame mapping, the attribution classifier, heal idempotence.
- Integration: a fixture `$out` (reports + `status.json`) exercised by the host
  gate across the three categories.
- Final gate: `cargo xtask validate` green in the Nix sandbox.

## Sequencing & risk

1. Extract the `coverage` library crate from `xtask/src/coverage/` — pure,
   behavior-preserving refactor, fully tested.
2. Add `devtool` + `coverage emit`; wire the producer; delete `check-coverage`;
   confirm byte-parity of emitted artifacts.
3. Add the producer/consumer split + always-emit + `status.json`; move all gating
   host-side.
4. Fold in #2 attribution (categories + evidence + reporting).
5. Add diagnostics bundle + `--keep-failed` backstop + CI artifact upload.
6. Fold in #3/#11 reference-frame correctness (riskiest — isolated, heavily
   tested).
7. Confirm #7 + regression test.

**Biggest risks:**
- In-sandbox build cost of the new `coverage` lib + `devtool` crate (mitigated by
  keeping them small and the host gate excluded).
- Reference-frame correctness (#3/#11) — sequenced last, isolated, test-first.
- Producer/consumer restructuring of the flake (verify parity against the current
  gate before deleting the shell path).
