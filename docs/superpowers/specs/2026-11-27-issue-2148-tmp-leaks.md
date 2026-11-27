# Issue #2148 — stop tests and e2e-local leaking /tmp directories

## Outcome

Gate, test, coverage, mutant and `dev e2e-local` runs stop leaving directories
in `/tmp`. A direnv shell (and every agent session started from one) gets the
same checkout-local `TMPDIR=<checkout>/.xtask/tmp` that `nix develop` already
gives, so host temp files go away with the worktree.

## Approach and tradeoffs

Four independent fixes, one per leak named in the issue:

1. **Fake `nix build` (e2e-workflow tests).** `FAKE_NIX` in
   `tools/dev/tests/test_e2e_workflow.py` removes its `mktemp -d` staging
   directory with `rm -r` once `nix store add-path` has copied it into the
   store.
2. **`ThemeManager` test helper.** `manager_with_changed_snapshot()` in
   `storage/src/theme_manager.rs` returns its lock-root `TempDir` with the
   manager; its three callers (`replace_role`, `replace_header_pool`,
   `remove_theme` tests) hold it for the test. `keep()` goes away, matching the
   file's other five lock roots.
3. **Host collector in `dev e2e-local`.** `_start_collector` launches
   `otelcol-contrib` with `TMPDIR` set to its capture source directory, which
   the run already deletes (or retains as evidence) afterwards. The Snowflake
   driver's empty `gosnowflake-cgo*` directory then lands inside that source and
   is removed with it. Accepted cost: a retained capture may contain that one
   empty directory. We are not patching or wrapping the upstream binary.
4. **direnv `TMPDIR`.** nix-direnv restores the pre-entry `TMPDIR` after running
   the shellHook, so the routing never survives. The TMPDIR block (resolve
   checkout root, create and probe `.xtask/tmp`, export, or `exit 1`) moves into
   one shell file, `nix/checkout-tmpdir.sh`. Both shellHook sites in
   `nix/dev-shells.nix` (`shellEnv` and `ci-nix`, today two copies) inline it
   with `builtins.readFile`, and `.envrc` sources the same file after
   `use flake`, with its own `watch_file nix/checkout-tmpdir.sh` (the existing
   `nix/*.nix` glob does not match it). The file keeps the "exit, not return"
   comment: `exit` is also what aborts a direnv load. One definition, three
   consumers, and the existing duplicate between the two hooks disappears. Under
   direnv the block runs twice (nix-direnv discards the first export); that is
   cheap and accepted.

   Simplest alternative: a second copy of the export line in `.envrc`. Rejected
   because the issue asks for a single definition and the two copies already in
   `dev-shells.nix` show how they would drift. A failure in the sourced file
   makes the direnv load fail visibly, like shell entry.

## Acceptance

Proof to execute (recorded in the PR, not committed):

- `direnv exec . printenv TMPDIR` prints `<checkout>/.xtask/tmp`.
- With `TMPDIR` pointed at a fresh empty directory, running the dev-suite
  e2e-workflow tests and the `storage` unit tests leaves that directory empty.
  (Leak 3 is proved by the committed collector test below; the fake collector
  never creates a `gosnowflake-cgo*` directory.)

Committed tests:

- `tools/dev/tests/test_e2e_local.py`: the fake collector records its `TMPDIR`
  and a test asserts it equals the capture source directory
  (`JAUNDER_CAPTURE_DIR`). The existing `readonly-tmp` fake, which reaches the
  run's `TMPDIR` through the collector's environment, instead locks the capture
  source's parent (the run's `TMPDIR`), keeping that failure case.
- `tools/dev/tests/test_dev_shells.py`: the existing hook tests keep passing
  against the shared file; a new test copies `.envrc` and
  `nix/checkout-tmpdir.sh` into a git-initialised fixture, sources `.envrc` with
  stub `watch_file` and a stub `use` that resets `TMPDIR` (as nix-direnv does),
  and asserts the result is the fixture checkout's `.xtask/tmp`. It skips via
  `require(...)` like the module's other tests where the filtered
  `checks.dev-python` source lacks those files.
- `CONTRIBUTING.md`'s shell-entry paragraph says direnv gets the same binding.

## Boundaries

- No change to Claude Code scratchpads or `ux-sandbox-` storage (issue's out of
  scope).
- No sweeping of already-leaked `/tmp` entries; no new cleanup tooling.
- No change to NixOS VM collectors or Nix build sandboxes.
