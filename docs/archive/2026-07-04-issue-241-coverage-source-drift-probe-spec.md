# Spec — #241: drvPath drift-guard probe for the Nix coverage source filter

- **Issue:** jaunder-org/jaunder#241 (milestone "Verify-gate hardening")
- **Follow-up to:** #231 (bounded the coverage derivation `src`), #37 (original
  impurity + the deferred "probe regression check" acceptance)
- **Status:** approved

## Context

#231 bounded the Nix `coverage` derivation's `src` to cargo sources plus an
explicit `csr/index.html` re-admission (`flake.nix`, the
`coverage = craneLib.mkCargoDerivation` block). That closed the #37 impurity:
files that match no admit branch of the filter (build junk, editor temp files,
and the like) no longer perturb the coverage derivation's hash. What remains
from #37's acceptance — "a probe regression check so the source filter can't
silently drift" — is a **drvPath drift guard**, deferred into this issue.

The guard exists because the `src` filter is load-bearing but silent: if a
future edit broadens it (re-admitting junk → the #37 impurity returns) or
narrows it (dropping source → those lines silently stop being measured, a
coverage hole the stateless gate can never see because the lines aren't in the
report), nothing today would catch it. A probe that asserts the filter's two
contract invariants, run automatically in CI, converts a silent drift into a
loud failure.

## Load-bearing subtlety (why attempt #1 failed) — empirically re-verified

A Nix flake's source is **`git ls-files` (tracked + staged paths), read with
working-tree content** — the well-known "nix flakes ignore untracked files"
behavior. The decisive axis is **tracked-vs-untracked, not clean-vs-dirty.**
Verified against this repo's nix (three `nix eval …coverage.drvPath`
experiments, appendix below):

| Change to the tree                             | Seen by nix / moves drvPath?           |
| ---------------------------------------------- | -------------------------------------- |
| New file, **untracked** (not `git add`-ed)     | **No** — ignored, even on a dirty tree |
| New file, `git add`-ed (staged)                | **Yes**                                |
| New file, committed                            | Yes (same drvPath as staged)           |
| Edit to an existing tracked file, **unstaged** | **Yes**                                |
| Edit to an existing tracked file, staged       | Yes (same as unstaged)                 |

**This corrects the issue's own diagnosis.** #241 attributes attempt #1's false
negative to the worktree being _clean_ and prescribes "make the tree dirty." But
a _dirty_ tree also ignores untracked new files — dirtying is **not**
sufficient. The probe's `.rs` and junk files are _new_ files, so they are
invisible to nix until **`git add`-ed**. Merely touching a tracked file to dirty
the tree (the previous spec's approach) would have reproduced attempt #1's false
negative — `rs != base` failing on every honest run.

Therefore the probe **stages its new probe files with `git add`** (which both
makes them visible to nix and dirties the tree — no separate dirtying step
needed). Consequence for day-to-day gate correctness: a developer's _unstaged
edits to existing tracked files_ are measured normally; only brand-new files
need staging to be seen.

## Decisions (resolved in design interview)

1. **Orchestration: ephemeral worktree, never the real tree.** The probe creates
   a throwaway `git worktree add --detach HEAD` under a git-ignored location
   (`.xtask/` — already ignored, same home as xtask run logs), does all mutation
   there, and removes it. Cleanup rides on an explicit RAII scope guard (a
   `WorktreeGuard` whose `Drop` runs `git worktree remove --force`), so removal
   covers `?`-based early returns **and** panics — not a best-effort cleanup at
   the end of the happy path. Rationale: the repo norm is that gates never
   mutate the working tree (`validate` is verify-only); an in-place save/restore
   probe touches the user's real tree and risks leaving cruft on interrupt. All
   mutation and cleanup are contained in the ephemeral worktree.

2. **Include probe files by staging them (`git add`), not by dirtying a tracked
   file.** Per the truth table above, a _new_ file is invisible to nix until
   staged, so the probe `git add`s each probe file into the ephemeral worktree's
   index before evaluating. Staging both makes the file visible **and** dirties
   the tree, so there is no separate "touch a tracked file" step. `base` is the
   clean-HEAD eval (no probe files staged); each subsequent state stages exactly
   one probe file, resetting between states so the comparison isolates a single
   variable.

3. **Probe-file paths must be non-gitignored** (else `git add` refuses them
   without `-f`, and the intent — a normally-addable file — is muddied):
   - **junk**: root `probe.txt` — matches no admit branch of the filter (not
     `.sql`/`.css`/ `scripts/*`/`csr/index.html`/cargo-source) and is not
     gitignored (`git check-ignore` confirms). (The exact junk example from the
     issue.) Staged → nix includes it → filter drops it → drvPath unchanged.
   - **instrumented `.rs`**: `server/src/__drift_probe.rs` — under a cargo
     source tree (admitted by `filterCargoSources`), not gitignored. It need not
     be referenced by any `mod`: the drvPath is a hash of the filtered source
     tree, so an unreferenced staged `.rs` still changes it (empirically
     confirmed).

4. **Pure verdict boundary.** A pure
   `probe_verdict(base, junk, rs) -> Result<(), _>` takes the three drvPath
   strings and encodes the contract; the impure orchestration (worktree, evals)
   lives outside it. Unit-tested per the issue's mandate.

5. **CI wiring: a step in the existing `validate-no-e2e` job**, not per-commit
   `check`/`validate` and not a dedicated job. Rationale: the job already has
   nix + cachix + xtask-cache set up, and the probe is eval-only (no
   compilation), so a step is near-free; a dedicated job would re-pay minutes of
   setup for seconds of eval. Runs with `if: always()` so a drift signal is not
   masked by an unrelated `validate` failure. It is deliberately _not_ in
   per-commit `check`/`validate` (issue: "only needs to run in CI / on
   request").

6. **No ADR.** The source-filter bounding itself (#37/#231) did not warrant an
   ADR; this is its regression guard, a test mechanism, fully captured here and
   in code comments. Adding one would be inconsistent with #231.

## Design

New user-facing subcommand `cargo xtask coverage probe-source` (nested
`CoverageCommand` enum on the top-level `Command`, mirroring the existing
`Adr(AdrCommand)` pattern).

Handler `coverage::probe::probe_source() -> StepResult` orchestrates:

1. `git worktree add --detach HEAD <tmp>` where `<tmp>` is a git-ignored path
   (e.g. `.xtask/coverage-probe.worktree`) — an ephemeral checkout owned by a
   `WorktreeGuard` whose `Drop` runs `git worktree remove --force <tmp>`, so it
   is removed on every exit path (early return, error, or panic). Git operations
   run with hooks disabled (`git -c core.hooksPath=` or equivalent) so the
   repo's pre-commit/other hooks can't fire during the probe's index
   manipulation.
2. **State A (base):** eval the clean-HEAD worktree —
   `nix eval --raw --accept-flake-config <tmp>#checks.x86_64-linux.coverage.drvPath`.
3. **State B (junk):** create `<tmp>/probe.txt`, `git add` it, eval → `junk`;
   then `git rm --cached`
   - delete it to return to the clean index.
4. **State C (rs):** create `<tmp>/server/src/__drift_probe.rs`, `git add` it,
   eval → `rs`.
5. `probe_verdict(base, junk, rs)`:
   - `junk == base` — else the filter **admits junk** (impurity regression) →
     fail.
   - `rs != base` — else the filter **drops source** (coverage hole) → fail.
   - both hold → `Ok`.
6. Remove the ephemeral worktree (via the guard); return a `StepResult` whose
   `detail` names the specific broken invariant on failure.

Each staged eval prints a "Git tree is dirty" warning to stderr; that is
expected, not a failure — stderr is captured separately and not treated as an
error signal.

Files touched:

- `flake.nix` — unchanged (the filter under test).
- `xtask/src/coverage/probe.rs` (new) — `probe_verdict` (pure) + `probe_source`
  (orchestrator).
- `xtask/src/coverage/mod.rs` — expose the `probe` module.
- `xtask/src/lib.rs` — `CoverageCommand` enum, `Coverage(..)` on `Command`,
  dispatch arm.
- `xtask/src/steps/nix.rs` — add a `nix eval --raw` helper for an arbitrary
  installable at an arbitrary flake dir, evaluating `.drvPath`. Note this is not
  drop-in: the existing helper (`eval_out_path`, private) is hardcoded to
  `.#checks.{SYSTEM}.{check}.outPath` in the current dir, so generalizing the
  flake dir + switching to `.drvPath` is real work. (`.drvPath` per the issue;
  `.outPath` moves with it, but drvPath is the input-addressed identity and
  needs no realized output.)
- `.github/workflows/ci.yml` — a step in `validate-no-e2e` running
  `nix develop .#ci --accept-flake-config -c cargo xtask coverage probe-source`
  (matching the sibling steps' invocation form); the probe then nests its own
  `nix eval` calls.
- `CONTRIBUTING.md` and/or the coverage docs — document the on-demand probe and
  its CI role.

## Acceptance criteria (observable)

1. **AC1 — probe passes today.** On the current tree,
   `cargo xtask coverage probe-source` exits 0.
2. **AC2 — catches broadening.** If the `src` filter is edited to admit
   `probe.txt` (e.g. add a `.txt`/junk-admitting branch),
   `cargo xtask coverage probe-source` exits non-zero with a message identifying
   the _admits-junk / impurity_ invariant. (Demonstrable by a temporary local
   filter edit; not committed.)
3. **AC3 — catches narrowing.** If the `src` filter is edited by _any_ narrowing
   that stops admitting `server/src/__drift_probe.rs` (e.g. add an explicit
   exclusion for `__drift_probe.rs`, or restrict `.rs` admission to a fixed
   allowlist that omits it), `cargo xtask coverage probe-source` exits non-zero
   identifying the _drops-source / coverage-hole_ invariant. (Demonstrable by a
   temporary local filter edit; not committed.)
4. **AC4 — `probe_verdict` is unit-tested**, covering: both-hold → `Ok`;
   `junk != base` → `Err(admits-junk)`; `rs == base` → `Err(drops-source)`.
   Tests are pure (string inputs), host-side, deterministic.
5. **AC5 — no real-tree mutation, even on failure.** After a run — a passing
   run, _and_ a run forced to fail mid-probe (e.g. via a temporary filter edit
   per AC2/AC3) — `git status` in the invoking worktree is unchanged and no
   `probe.txt` / `__drift_probe.rs` / stray `git worktree list` entry remains.
   (The panic path is covered by construction via the `WorktreeGuard` `Drop`;
   the induced failure is the observable proxy for it.)
6. **AC6 — CI runs it.** `.github/workflows/ci.yml`'s `validate-no-e2e` job
   includes a step invoking `cargo xtask coverage probe-source` (with
   `if: always()`), so drift fails a PR.
7. **AC7 — not per-commit.** `cargo xtask check` and `cargo xtask validate` do
   **not** invoke the probe (grep of their step lists confirms absence).
8. **AC8 — documented.** The on-demand probe and its CI role are described in
   `CONTRIBUTING.md` or the coverage doc.

## Out of scope

- Any change to the `src` filter's actual behavior (this only guards it).
- Wiring the probe into per-commit `check`/`validate`.
- The #246 marker-matching hardening (separate issue).

## Testing / verification ladder

- `probe_verdict` unit tests (AC4) — run under the normal xtask/host test path.
- Manual/scripted demonstration of AC2/AC3 via a throwaway local filter edit
  (reverted), to prove the guard actually fires — recorded in the plan, not
  committed.
- `cargo xtask check` green (probe module compiles, clippy clean, unit tests
  pass) before ship.

## Appendix — empirical verification of the source-visibility model

Run against this repo's nix at `flake.nix` HEAD `ccb81d0e`, each an ephemeral
`git worktree add --detach HEAD` evaluating
`.#checks.x86_64-linux.coverage.drvPath`. This is what grounds the truth table
and the staging decision; it is the manual dry-run of the probe itself.

- **Exp 1 — untracked, dirty tree.** Tree dirtied via an unstaged `README.md`
  edit; untracked `probe.txt` and untracked `server/src/__drift_probe.rs` each
  added. **All three drvPaths identical** → untracked new files are ignored
  _even on a dirty tree_ (refutes "dirty ⇒ untracked included").
- **Exp 2 — staged / committed.** From clean HEAD: `git add probe.txt` → drvPath
  **unchanged** (filter drops it); `git add server/src/__drift_probe.rs` →
  drvPath **changed**; committing the `.rs` → same changed drvPath as staged.
  Confirms staging is the correct inclusion mechanism and that `junk == base`
  genuinely exercises the filter.
- **Exp 3 — tracked file edited.** An **unstaged** edit to an existing tracked
  `.rs` (`server/src/assets.rs`) → drvPath **changed**, identical to the staged
  edit. Confirms nix reads working-tree content for tracked paths regardless of
  staging; only _new_ files require `git add`.
