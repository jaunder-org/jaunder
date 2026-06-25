# Issue #3 — Working-tree-robust coverage gate (untracked files)

**Issue:** [#3](https://github.com/jaunder-org/jaunder/issues/3) — *coverage: make the
gate robust to working-tree state (or fail fast precisely), killing
commit/stage-first friction.*
**Milestone:** Verify-gate hardening.
**Related:** [#11](https://github.com/jaunder-org/jaunder/issues/11) (line shifts —
addressed by the same `195d52d` anchor work), [#37](https://github.com/jaunder-org/jaunder/issues/37)
(the underlying flake source-filter purity smell, deliberately out of scope here).

## Background — what is already fixed

The issue was filed against an older classifier that diffed `HEAD→worktree` and
assumed the Nix report could be in *committed-tree* line space. That premise is
**refuted** by the current code and flake:

- Commit `195d52d` ("map baseline anchor→working-tree, not HEAD (#3, #11)"), already
  on `main`, reworked the diff to **anchor→working-tree**: `baseline_anchor_commit()`
  (the commit that last touched `coverage-baseline.json`) diffed via a single-commit
  `git diff --unified=0 <anchor>` against the working tree (`xtask/src/coverage/mod.rs`
  ~256-295). This makes line-shifting **tracked** edits map correctly without
  committing first — the headline friction of #3/#11.
- The Nix `coverage` check's source is the **working tree** (verified 2026-06-25 by a
  `drvPath` probe): dirty tracked content *and* untracked non-gitignored files are
  included. So the report is in working-tree line space — consistent with the
  anchor→worktree map for tracked files.

## The residual defect — untracked files

Untracked, non-gitignored `.rs` files that the build compiles (i.e. referenced by a
tracked `mod` declaration) **are instrumented and appear in the coverage report**,
but `git diff <anchor>` omits untracked files. So:

1. `parse_unified_diff` produces **no `LineMap`** for the untracked path.
2. In `classify` the path falls back to `empty_map()` (identity), whose
   `added_lines()` is empty.
3. Every uncovered line in the untracked file therefore hits the `else` branch
   (`xtask/src/coverage/classify.rs` ~73-74) and is bucketed as **`regression`** —
   when it is in fact brand-new uncovered code that should be **`new_uncovered`**.

The gate still *fails* on these lines (both buckets fail), so this is not a silent
hole — but the **label is misleading** ("you broke existing coverage" vs "new code
needs tests"), and the stage-first reflex is what currently papers over it. The
`all_added` synthesis that commit `022adb3` added for exactly this case was **dropped**
in the `195d52d` rewrite.

### Realistic scenario (scoping note)

A *standalone* untracked `.rs` that no tracked `mod` references is not compiled and
never reaches the report. The case that matters is a new untracked `foo.rs` **plus**
a tracked `mod foo;` edit: `foo.rs` is instrumented (untracked → needs synthesis),
while the `mod foo;` line rides the normal anchor diff. The fix only needs to
synthesize maps for untracked paths that actually appear in the report.

## Goal

Make the gate fully working-tree-robust: an untracked `.rs` file's uncovered lines
classify as `new_uncovered`, with no staging or committing required. Lock the
behavior (and the existing anchor→worktree behavior) with the end-to-end tests that
are currently absent. Document the working-tree contract.

Chosen approach: **robust** (classify untracked as new), not fail-fast — failing fast
would reintroduce the stage-first friction #3 exists to eliminate.

## Design

### 1. `diffmap.rs` — promote an all-added constructor

Add a non-test constructor that builds a `LineMap` whose `added` set is a given set
of new-side line numbers (its `map`/`offset_after` stay empty, so `map()` is never
consulted — untracked files have no baseline gaps):

```rust
/// A map for a file with no committed preimage (e.g. untracked): every given
/// line number is "added", and no old line maps to it.
pub fn all_added(lines: impl IntoIterator<Item = u32>) -> LineMap { ... }
```

This generalises today's test-only `set_added_for_test` into a real API.

### 2. `mod.rs` — enumerate untracked `.rs` and synthesize maps

- Add `untracked_rs_files() -> Result<Vec<String>>` shelling out to
  `git ls-files --others --exclude-standard -z -- '*.rs'` (NUL-delimited; pinned
  argv, same robustness posture as `diff_args`).
- In `run_inner`, after `current` and `maps` are built: for each untracked path that
  **appears in `current`** and has **no existing `maps` entry**, insert
  `LineMap::all_added(<that file's reported line numbers>)`.

Only files present in the report are synthesized, so untracked `.rs` elsewhere in the
tree (not compiled) is naturally ignored.

### 3. Classifier — unchanged

Once the map reports the file's lines as all-added, uncovered lines bucket as
`new_uncovered` via the existing logic. No change to `classify.rs` behavior.

### Heal / baseline reproducibility — no change, by construction

- An untracked file with uncovered lines → `new_uncovered` → not clean → no heal.
- A fully-covered untracked file contributes no gaps → `Baseline::from_files` records
  nothing for it → committed baseline is unaffected.

So the committed baseline never absorbs untracked-derived state. This is documented
as an explicit invariant rather than guarded with new code.

## Testing

- **Unit (classify):** the currently-missing case — a file present in `current` with
  an all-added map and an uncovered line → `new_uncovered`, not `regression`.
- **Unit (diffmap):** `all_added` constructor builds the expected `added` set and maps
  every old line to `None`.
- **End-to-end (the real gap):** a git-boundary test that creates a temp git repo,
  commits a baseline file, makes a tracked line-shifting edit **and** adds an
  untracked `.rs`, then runs the map-building path and asserts (a) the tracked shift
  maps correctly (locks `195d52d`'s anchor→worktree) and (b) the untracked file's
  lines are all-added. During planning, check for existing git test helpers in xtask;
  if shelling to git inside tests is undesirable, fall back to testing
  `untracked_rs_files()` parsing + the synthesis step with injected inputs.

Per-task gate: `cargo xtask check --no-test`. Final gate: `cargo xtask validate`.

## Documentation

`CONTRIBUTING.md` — add a short **working-tree contract** to the coverage section:

- The gate reflects your **working tree**: dirty tracked content *and* untracked
  non-gitignored files.
- Tracked line-shifting edits no longer need to be committed first (anchor→worktree).
- A new untracked `.rs` is measured and classified as **new uncovered code** — no
  staging required.

Adjust any stale "commit/stage first" guidance accordingly.

## Out of scope

- The flake's bare `cleanSourceWith { src = ./.; }` purity smell — tracked separately
  as **#37**. Changing it here would convert today's mislabeling into a silent
  coverage hole (new files invisible until staged), the opposite of #3's goal.
- Sibling milestone issues #7 (crap-manifest format) and #10 (clippy via Nix).
