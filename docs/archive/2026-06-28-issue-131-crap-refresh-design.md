# Issue #131 — Surface + harden the CRAP-manifest refresh path

**Issue:** [#131](https://github.com/jaunder-org/jaunder/issues/131) — _coverage: surface + harden the CRAP-manifest refresh path (recovery command + candidate)_

**Builds on:** #88 / ADR-0030 (the baseline `cargo xtask coverage reanchor` command, candidate-promotion model, "no accept-all" rule), #87 (gate prints actionable `file:line: text`), #7 (line-independent CRAP compare via the ordinal key).

**Milestone:** Verify-gate hardening (1).

## Problem

#88 gave a coverage-baseline *lowering* a first-class, safe recovery: `cargo xtask coverage reanchor` re-anchors a benign line-shift in place, or — on a genuine lowering — writes a candidate to `.xtask/coverage-baseline.candidate.json` and refuses, printing the exact offending lines and a manual `cp` promotion recipe.

The symmetric **CRAP-regression** path has no equivalent:

1. The gate's `failure_report` CRAP branch (`xtask/src/coverage/mod.rs`) prints **prose only** — "reduce complexity / improve coverage; refresh `crap-manifest.json` (with approval) only if it is stale drift" — with **no command**, no pointer to the fresh report, and no candidate-promotion analog.
2. The manual refresh is fiddly: `check` Fix-mode regenerates the manifest **only when there are no CRAP regressions** (`mod.rs` ~251), so an *approved* regression must be hand-overwritten from the fresh report and re-run.

## Goal (from the issue's "Done when")

- A CRAP-regression failure tells the operator exactly **what to run** (a command, not prose).
- Refreshing an approved CRAP drift is a single, discoverable, safe step that lands as a **reviewable diff** — a deliberate `cp`, never an automatic lowering.

## Design

### `cargo xtask coverage refresh-crap [--gcroot DIR]`

A new nested `coverage` subcommand, sibling to `reanchor`, with the same `--gcroot`
default (`.xtask/gcroots/coverage`). It **consumes the existing `crap-report.json`**
that the gate already built under the gcroot — it never rebuilds coverage (exact
mirror of `reanchor`). It reads the committed `crap-manifest.json`, runs the existing
`crap::compare`, and dispatches on a pure plan:

- **No regressions → refresh in place, succeed.** Write the fresh report (pretty,
  canonicalized) to the committed `crap-manifest.json`. If there is no CRAP-relevant
  drift — the line-independent canonical forms are equal (only `line` hints differ, or
  nothing changed) — it is a **no-op success** ("already current"), matching Fix-mode's
  churn-avoidance (`mod.rs` ~258, `if new_canon != old_canon`). This case is provably
  non-worsening (improvements, new/removed functions), the CRAP analog of `reanchor`'s
  safe line-shift path.
- **Regressions present → write candidate, refuse.** Write the full fresh report
  (pretty) to `.xtask/crap-manifest.candidate.json` and **fail (non-zero)**, printing
  each offending `file::fn old → new` plus the inspect/promote recipe:
  `git diff --no-index crap-manifest.json .xtask/crap-manifest.candidate.json`, then
  `cp .xtask/crap-manifest.candidate.json crap-manifest.json && git add crap-manifest.json`.
  Promotion stays a deliberate, reviewable `cp` — there is no flag that accepts a
  regression automatically.

`.xtask/` is gitignored, so the candidate never dirties the tree or gets instrumented
(same property the baseline candidate relies on).

### Module layout — `crap.rs` becomes the owner of CRAP-manifest logic

Mirror `reanchor.rs`'s shape (`CANDIDATE_PATH` + `ReanchorPlan` + `plan_reanchor` +
`refusal_report`). In `crap.rs`:

- `pub const CRAP_CANDIDATE_PATH: &str = ".xtask/crap-manifest.candidate.json";`
- `pub enum CrapRefreshPlan { Refresh { manifest: Option<String> }, Refuse { candidate: String, regressions: Vec<CrapRegression> } }`
  — `Refresh { manifest: None }` is the "already current" no-op; `Some(json)` carries the
  pretty manifest to write.
- `pub fn plan_crap_refresh(fresh_report: &str, old_manifest: &str) -> Result<CrapRefreshPlan>`
  — pure (no I/O), runs `compare`, decides, and pre-renders the bytes to write.
- `pub fn refusal_report(regressions: &[CrapRegression]) -> String` — the offending
  `file::fn old → new` list (capped, with "… N more") + the candidate path, inspect
  diff, and `cp` recipe. Parallels `reanchor::refusal_report`.

To keep `plan_crap_refresh` pure and self-contained, the two CRAP-manifest helpers
currently in `mod.rs` move into `crap.rs`: `normalize_crap_without_line` →
`crap::normalize_without_line`, and `pretty_json` → `crap::pretty_manifest`. The two
call sites in `mod.rs::run_inner` update to the relocated names. This is a small,
focused refactor in service of the goal: `crap.rs` becomes the single home of
CRAP-manifest concerns, exactly as `reanchor.rs` owns baseline-reanchor logic.

`mod.rs` keeps only a thin I/O wrapper, mirror of `reanchor` / `reanchor_inner`:

```rust
pub fn refresh_crap(out_dir: &str) -> StepResult
fn  refresh_crap_inner(out_dir: &str) -> Result<StepResult>
```

`refresh_crap_inner` reads `<out_dir>/crap-report.json` (clear error if missing, like
`reanchor_inner`'s report check), reads the committed `crap-manifest.json`
(`unwrap_or_default` for first-run), calls `crap::plan_crap_refresh`, then performs the
write (committed manifest, or candidate via a parent-dir-creating write) and sets the
exit status.

### Gate failure report

`failure_report`'s CRAP branch changes from prose-only to the actionable form, mirroring
the coverage-lowering branch's `run: cargo xtask coverage reanchor`:

> → CRAP: reduce the function's complexity or improve its coverage; if this is
>   approved drift (not a real regression), refresh the manifest for review:
>   run:  cargo xtask coverage refresh-crap

The existing **category-conditional** structure is preserved: `reanchor` stays
baseline-only (never shown for a CRAP-only failure), and `refresh-crap` is shown only
when there are CRAP regressions. The existing tests pinning that split
(`failure_report_guidance_is_category_conditional`) are updated for the new string.

### CLI wiring (`lib.rs`)

- `CoverageCommand::RefreshCrap { gcroot }` with the same `--gcroot` default and an
  `after_help` examples block, sibling to `Reanchor`.
- `command_name` arm → `"coverage-refresh-crap"`.
- `run` dispatch arm → `coverage::refresh_crap(&gcroot)`.
- Parse tests: default gcroot + explicit `--gcroot`.

## Testing

TDD via xtask's own host unit suite (`steps::host_tests`). `/xtask/` is excluded from
the Nix coverage instrumentation (the coverage-src denylist), so this code is **not**
coverage-gated and there is **no coverage-baseline impact** — its safety net is its own
unit tests.

- `crap::plan_crap_refresh` — three outcomes: (a) no regressions + a CRAP-relevant
  change → `Refresh { manifest: Some(pretty) }`; (b) no regressions + only a line-shift
  / no change → `Refresh { manifest: None }` (already current); (c) a regression →
  `Refuse` carrying the candidate JSON and the regression list.
- `crap::refusal_report` — contains `file::fn`, `old → new`, `CRAP_CANDIDATE_PATH`,
  `git diff --no-index`, and the `cp` recipe; caps long lists.
- `failure_report` — the CRAP branch now contains `cargo xtask coverage refresh-crap`;
  the category-conditional test still confirms a CRAP-only failure does **not** mention
  `reanchor` and a lowering-only failure does **not** mention `refresh-crap`.
- `lib.rs` CLI parse tests for `coverage refresh-crap` (default + custom gcroot).
- The relocation of `normalize_without_line` / `pretty_manifest` is behavior-preserving:
  their existing tests (`crap_normalize_*`, `crap_pretty_json_is_multiline`) move with
  them and stay green.

## Documentation

- **CONTRIBUTING.md** (coverage section): document `cargo xtask coverage refresh-crap`
  and its candidate-promotion flow alongside the existing `reanchor` paragraph.
- **`docs/adr/0030-coverage-reanchor-text-identity.md`**: append `## Supplement (#131)`
  recording the symmetric CRAP path — same candidate-promotion model, no accept-all,
  the no-regression refresh writes in place while a regression refuses to a candidate.
  (The existing `## Supplement (#88)` already anticipates this symmetric path.)
- **`docs/README.md`**: no new ADR row (this supplements 0030, not a new ADR).

## Out of scope (YAGNI)

- Changing the CRAP comparison itself (`crap::compare`, the ordinal key, `EPSILON`) —
  #7 owns that; this issue only adds a recovery path around the existing compare.
- Any `--accept` / auto-promote flag — deliberately excluded; promotion is a manual `cp`.
- `refresh-crap` rebuilding coverage — it consumes the existing report, like `reanchor`.
- Touching the coverage baseline or `reanchor` behavior.

## Separable concerns

None surfaced — the design is self-contained within the existing coverage tooling and
mirrors an already-landed pattern.
