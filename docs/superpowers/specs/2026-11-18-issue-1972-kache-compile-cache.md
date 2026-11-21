# Issue #1972: kache replaces sccache as the host compile cache

## Outcome

Host Cargo compilation uses kache instead of sccache, with one configuration
locally and in CI: kache's defaults, except that executables are not cached
(`KACHE_CACHE_EXECUTABLES=0`). Locally, one per-user kache store serves every
worktree. In CI, the `validate-host` job's store persists between runs through
the GitHub Actions cache: `main` saves it, and pull requests and the merge queue
restore it. Dev and test builds emit line-table debug info only. What Clippy and
the other checks verify does not change.

## Evidence

The measurements comment on #1972 has the run IDs, method and raw numbers. All
runs used kache 2.1.1 against the post-#1965 lane. The timing runs used
`[profile.dev] debug = "line-tables-only"`. The disk runs used `main` as it
stands, with full debug info.

- **Local, three repetitions, second worktree of the same commit.**
  `check-no-test` took 267–268 s with kache and 669–682 s with sccache, even
  with both roots in `SCCACHE_BASEDIRS`. sccache misses about a quarter of Rust
  compiles across checkouts, mostly build-script crates and their dependents.
  With executables uncached, kache took 284–285 s. Cold builds took 601–614 s
  with kache and 721–747 s with sccache.
- **CI, paired.** Four jobs each ran every tool back to back on one runner, in
  rotated order, cold and then warm after five changes (none, `common`,
  `storage`, `server`, `tools`). Median lane seconds:

  | Change  | sccache | persisted `target/` | kache, no executables | kache, defaults |
  | ------- | ------- | ------------------- | --------------------- | --------------- |
  | cold    | 763     | 751                 | 694                   | 709             |
  | none    | 369     | 434                 | 289                   | 366             |
  | common  | 500     | 434                 | 407                   | 446             |
  | storage | 460     | 437                 | 372                   | 392             |
  | server  | 369     | 432                 | 292                   | 370             |
  | tools   | 362     | 432                 | 290                   | 374             |

  kache without executables beat sccache in every job and every change. With
  executables cached, the store reached 9.0 GiB against the runner's computed 7
  GiB cap (5 % of a 144 GiB disk), and pruning during the lane cost hits. After
  one cold lane, the store without executables was 4.8 GB raw and 1.2 GB
  compressed; sccache's was 0.9 GB. An earlier replay of successive merges
  (kache 2.0.1, 8 GiB cap, no executables) grew it to about 3.1 GB compressed.

- **Local disk, two worktrees two days apart on `main`, full debug info.** On
  ext4, kache copies every restore into the second worktree, so two worktrees
  and the store took 143 GB. On btrfs, restores are reflinks: the store cost
  almost nothing beyond the first worktree's 43 GB, and the second worktree
  added 31 GB without cached executables (26 GB with them). Plain Cargo took 44
  GB per worktree; with line tables only, one worktree of the slightly older
  experiment commit took 22 GB.
- **Shared `target/` across worktrees, rejected.** It took 60 GB for the two
  worktrees, but Cargo's mtime freshness then skipped Clippy on three changed
  `web` files in the second worktree. Within one checkout, sharing between the
  product and tools workspaces saved about 1 GB.

## Approach

**Wrapper.** `dev` owns the compile-cache environment
(`compile_cache_environment()` in `tools/dev/dev/checkout.py`), and every `dev`
host step applies it. That stays the single owner, and is what makes the local
and CI configurations the same. The values change:

- `RUSTC_WRAPPER=sccache` becomes `RUSTC_WRAPPER=kache`.
- `KACHE_CACHE_EXECUTABLES=0` is added. Test, tool and build-script executables
  are the bulk of the store. Uncached, they are relinked or, for build scripts,
  recompiled; the compiles kache still serves outweigh that.
- `CARGO_INCREMENTAL=0` stays. kache's documentation asks for it on persisted
  target directories, and it keeps kache's adaptive incremental mode off.
- The `git worktree list` discovery that computes `SCCACHE_BASEDIRS` is deleted,
  not ported. kache makes paths portable without it. With it go the
  `worktree list --porcelain` branch of the fake git in `conftest.py`, the shim
  in `test_pr_frontdoors.py`, and `CONTRIBUTING.md`'s discovery-failure
  sentence; `test_gates.py`'s `..._across_worktree_roots` test is renamed for
  what it still checks.
- The `tools/dev` tests that assert the environment expect the new values and no
  `SCCACHE_BASEDIRS`. They are the fake-cargo capture in `conftest.py`,
  `test_gates.py`, `test_static.py` and `test_test_local.py`.
- Nothing runs `kache init`. Nothing writes kache configuration, Cargo
  configuration or shell configuration into users' files.

**Debug info.** `[profile.dev] debug = "line-tables-only"` is set in
`Cargo.toml` and `tools/Cargo.toml`, and the test profile inherits it.
Backtraces keep file and line numbers. This restores the decision recorded on
#1972 on 2026-11-17; it halves the per-worktree `target/` and keeps the CI store
within the runner's default cap. The Nix derivations that already set
`CARGO_PROFILE_*_DEBUG=0` are unchanged. The few dev-profile Nix builds without
that override (the debug `jaunder` binaries, `debugHostArgs`) also get line
tables only.

**Pin.** The dev shells' `ciInputs` provide kache in place of `pkgs.sccache`,
from a second nixpkgs input, `nixpkgs-kache`, locked to
`04906337b6d53e8bb0eaf0470ae9f070a4d4eb55`, the revision the measurements used,
which carries kache 2.1.1. Nothing else reads that input.

- The locked `nixos-unstable` revision (2026-09-08) has no kache.
- Bumping the main input instead was the approved plan, but at that revision,
  and at `nixos-unstable` on 2026-11-20, `otelcol-contrib` 0.155.0 fails to
  build under Go 1.27 and is not substituted. It builds with Go 1.26, but it is
  also the product NixOS module's and the VM checks' collector, so working
  around it would mean an overlay on every `pkgs`. The human chose the second
  input on 2026-11-20.
- The input is dropped once the main nixpkgs carries kache and builds
  `otelcol-contrib`.

**Off switch.** `KACHE_DISABLED=1` keeps the wrapper in place but sends every
compile straight to `rustc`, without starting the daemon. `dev` passes it
through because it sets only its own variables. `CONTRIBUTING.md` documents the
switch for use when a stale-artifact bug is suspected.

**Local.** Beyond `dev`'s environment, kache runs on its defaults: one store in
the user cache directory shared by all worktrees, a daemon started on demand,
and a cap of 5 % of the store's volume (5 GiB floor, 100 GiB ceiling) with
automatic GC. `CONTRIBUTING.md` states:

- that kache shares disk across worktrees only on a reflink filesystem (btrfs,
  or XFS with reflink); on ext4 each worktree's restores are private copies
- that `KACHE_CACHE_DIR` moves the store, which must sit on the same filesystem
  as the worktrees for reflinks to apply
- the per-worktree disk cost, measured with line tables after this change lands

**CI.** Only `validate-host` persists a store. It is still the one job that
compiles the product workspace on the host (`dev ci-validate-host`). The other
jobs that enter `dev` (`validate-hermetic`, `validate-test-checks`,
`validate-coverage`) also get the wrapper, through `dev`, for their `cargo run`
tool builds; they run with an empty, unpersisted default store, and keep the
`tools/target` cache unchanged.

| Step      | When                                                                                             | What                                                                                                                                                              |
| --------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Configure | always                                                                                           | `KACHE_RUNTIME_DIR=~/.cache/kache-run`                                                                                                                            |
| Restore   | always                                                                                           | `actions/cache/restore` of `~/.cache/kache`, newest entry with the `kache-${{ runner.os }}-` prefix                                                               |
| Stats     | `always()`, after the lane                                                                       | `kache stats`                                                                                                                                                     |
| Save      | `github.event_name == 'push' && github.ref == 'refs/heads/main'` (with the implicit `success()`) | remove the product `target/`; `kache gc`; `kache daemon stop`; `actions/cache/save` under `kache-${{ runner.os }}-${{ github.run_id }}-${{ github.run_attempt }}` |

- **Steps.** All four are steps of `validate-host`: configure and restore right
  after `setup-ci`, stats and save at the end, so the store's path and key live
  in one job. `setup-ci` is unchanged. kache is only in the dev shell, so the
  kache commands run through `nix develop .#ci -c kache …`.
- **Same configuration.** CI sets only where the daemon's runtime files live, so
  its socket and locks stay out of the archive. The store is kache's default
  location, already outside the checkout, so it never shows up in `clean_tree`
  or in the platform flake's source. The cap is kache's default for the runner's
  disk.
- **`target/` before GC.** kache keeps a blob that a live target still links, so
  removing the product `target/` first lets GC bring the saved store within its
  cap.
- **Daemon stop.** Stopping the daemon first means the archive captures an index
  the daemon is not writing. `kache gc` is the documented compatibility command
  for evicting to the cap now. It is hidden from `--help`.
- **Run-stamped key.** It follows `mutants.yml`, because Actions cache entries
  are immutable. Superseded `kache-` entries are left to the Actions cache's
  least-recently-used eviction; no job needs new permissions.
- **Out-of-order saves.** Concurrent `main` pushes can finish out of order and
  leave a slightly older store. That is accepted: the next save replaces it.

**Decision record.** A tracked `jaunder-adr` draft records the switch to kache,
the shared configuration with executables uncached, line-table debug info, and
the CI store. It amends the sccache clauses of ADR-0146 and ADR-0237. ADR-0251
only places the compile-cache environment in `dev`, which stays true, so it is
not amended. The same commit projects the new decision into
`docs/ARCHITECTURE.md` and updates the remaining sccache mentions:

- `CONTRIBUTING.md`: the policy statement, and the `test-local`, gate and
  PostgreSQL sections
- `docs/ARCHITECTURE.md`: the three policy passages
- the comments in `nix/dev-shells.nix`, `tools/dev/dev/checkout.py` and
  `tools/dev/dev/gates.py`

## Alternatives rejected

- **kache with cached executables.** About 5 GB less per extra local worktree on
  a reflink filesystem, but it overflowed the CI runner's default cap and lost
  hits to pruning (measured). Raising the cap in CI only would split the
  configuration.
- **Persisted sccache everywhere.** Slower in every measured CI change and much
  slower across local worktrees, and it keeps the base-directory discovery.
- **A persisted `target/` in CI instead of a compile cache.** Slower than kache
  without executables on every warm change (measured).
- **One `target/` shared across worktrees.** Cargo's mtime freshness gives stale
  results across checkouts (measured), and the build lock would make worktrees
  wait on each other.
- **A kache store restored in every job, replacing the `tools/target` cache.**
  Every e2e and coverage job would unpack the store for a handful of tool
  crates.
- **`kunobi-ninja/kache-action`.** It installs kache outside Nix.
- **Deleting superseded store entries.** It needs `actions: write` on
  `validate-host` to save at most one store's worth of space.

## Accepted costs

- `validate-host` downloads the store on every run: about 1.2 GB after one lane,
  growing toward its cap over successive merges (about 3 GB compressed in the
  earlier replay).
- The repository's Actions cache is already at its 10 GB limit. Each `main` save
  evicts the least recently used entries, which are mostly pull-request and
  merge-queue `tools-` entries and any mutants `--iterate` state.
- Locally on ext4, kache's store is extra disk on top of every worktree's full
  `target/`. Reflink filesystems are recommended, not enforced.
- A second nixpkgs input for one tool, until the main input carries it.
- Full debug info is no longer the default for dev and test builds.

## Acceptance

Proof to execute:

1. No host Cargo path runs sccache.
   - `git grep -i sccache` matches only historical ADR text, the new ADR draft
     and this spec.
   - The `tools/dev` environment tests assert exactly `RUSTC_WRAPPER=kache`,
     `KACHE_CACHE_EXECUTABLES=0` and `CARGO_INCREMENTAL=0`. Nothing sets
     `SCCACHE_BASEDIRS`; the grep above proves its absence, so the tests do not
     name it.
2. Cross-worktree hits, with an isolated `KACHE_CACHE_DIR` and
   `KACHE_RUNTIME_DIR`: after `dev check-no-test` in one worktree, the same
   commit in a second worktree reports hits in `kache stats`.
3. The off switch works. With the same isolated store and `target/` removed,
   `KACHE_DISABLED=1 dev check-no-test` succeeds, and `kache stats` hit and miss
   counts are the same before and after.
4. `nix develop .#ci -c rustc -V` reports 1.97.1, and
   `nix develop .#ci -c kache --version` reports 2.1.1.
5. CI saves and restores the store:
   - A `main` push's `validate-host` saves one `kache-` entry.
   - A following PR run restores it, and its `kache stats` step reports hits.
   - No PR or merge-queue run creates a `kache-` entry.
   - No job other than `validate-host` restores the store.
6. The PR records a warm `validate-host` run's duration, including the restore,
   and the saved entry's size, next to recent `main` runs. They are reported,
   not thresholds.
7. The Clippy and deny command definitions are unchanged in the diff.

Deliverables to commit:

- this spec
- the `checkout.py` change and its tests
- the profile change
- the flake input, dev-shell and lock changes
- the `validate-host` steps
- the ADR draft
- the `CONTRIBUTING.md` and `ARCHITECTURE.md` updates

## Boundaries

- Hermetic Nix builds (crane) are untouched: no wrapper runs inside the sandbox.
- Other CI jobs change only in running the wrapper with an empty store; the
  `tools/target` cache is unchanged.
- The mutants shell keeps no wrapper. It gets kache on `PATH` in place of
  sccache, and nothing sets `RUSTC_WRAPPER` there.
- No remote kache backend (S3), and no kache test runner or other kache feature
  beyond the rustc wrapper.
- Follow-ups, not here:
  - Host tools built by Nix and served from Cachix, instead of `cargo run` from
    the checkout, which retires CI's stale `tools/target` cache: #1968 (it
    absorbs #2027 and supersedes #2026).
- No resource limits are added to any lane.
