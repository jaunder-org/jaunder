# Issue #1989 — platform contract evaluates the Git source, not the directory

## Outcome

`just _platform` (in `validate-no-e2e` and `ci-validate-host`) stops copying the
whole checkout into the Nix store. It evaluates the flake from the files Git
tracks, including uncommitted edits to them, and no longer copies `.git/`,
`target/`, `tools/target/` or other ignored or untracked files. The platform
contract's assertions do not change.

## Approach and tradeoffs

Today the recipe calls `builtins.getFlake (toString ./.)`. A bare path makes Nix
fetch a `path:` flake, which copies the entire directory. In `ci-validate-host`
that step runs after the Clippy units, so it copies several GB of fresh build
output; the issue records 120–170 s per run and a `No space left on device`
failure when a lane runs twice on one runner.

Change the reference to `builtins.getFlake ("git+file://" + toString ./.)`.
Nix's Git fetcher then copies only tracked files and reads them from the working
tree, so uncommitted edits to tracked files are still evaluated. This is the
same Git fetcher `nix develop .#ci` already uses in the same `Validation host`
job, on the same shallow, detached `actions/checkout` clone, so CI compatibility
is already shown. `tools/repository-policy/tests/native.rs` already uses this
exact expression.

Probe (Nix 2.34.8, linked worktree with a dirty tracked `flake.nix`, an
untracked `untracked.nix` and an ignored `target/` file):

| Reference   | Sees dirty tracked edit | Store copy contains                                          |
| ----------- | ----------------------- | ------------------------------------------------------------ |
| `path:`     | yes                     | `.git`, `.gitignore`, `flake.nix`, `target`, `untracked.nix` |
| `git+file:` | yes                     | `.gitignore`, `flake.nix`                                    |

Accepted limitation: a new Nix file that is not yet `git add`ed is invisible to
the contract and to `just perf`. The gates already run `_clean-tree` first, so
they never see one. `perf` has no such guard, but `.#` commands already behave
this way. A developer adds the file first.

`just perf` has the same defect: it evaluates `selectPerformanceProducers` with
`getFlake (toString ./.)` on developer machines, where `target/` is largest. It
gets the same one-expression fix. Nothing pins its expression, its producer
installables are `drvPath^out` (so `nix build` is unaffected), and it does not
use `self.rev`.

Edits:

- the `_platform` and `perf` recipes in `Justfile`;
- the pinned expectation in `tools/host-proof/tests/just_workflows.rs`
  (`platform()`, the regression test that locks the command);
- the run-comments in `nix/platform-contract.nix` and
  `nix/performance-contract.nix`;
- the manual command in `CONTRIBUTING.md`.

## Acceptance

- `cargo xtask test-local -- -p host-proof` passes with the updated `platform()`
  expectation.
- `just _platform` succeeds locally and in CI `Validation host`.
- The command in the `nix/performance-contract.nix` comment evaluates
  successfully.
- Proof goes in the PR and is not committed:
  - The store path `getFlake` fetches for a built checkout contains only tracked
    files: no `.git`, `target` or `tools/target`.
  - Disk use, measured locally on the main checkout with `target/` built:
    - before: `du -sb` of the directory, which is what `path:` copies, measured
      without performing the copy;
    - after: `nix path-info -S` of the `git+file:` `outPath`.
  - CI `_platform` step time:
    - before: 120–170 s, from the issue's evidence (run 43972357405);
    - after: from this PR's `Validation host` log.
- `nix/platform-contract.nix` assertions are byte-for-byte unchanged.

## Boundaries

- No change to what the contract asserts, to `_clean-tree`, or to CI topology.
- Qualification's locked `git+file:…?rev=` sources are already correct and are
  untouched.
- `perf` changes only its source reference; its selection, measurement and
  validation logic are untouched.
