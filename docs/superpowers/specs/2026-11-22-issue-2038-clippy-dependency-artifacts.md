# Issue #2038 — reuse cached dependency builds in the Nix Clippy leaves

## Outcome

On a pull-request run whose Nix Clippy verdict misses Cachix, the six Clippy
leaves stop recompiling third-party crates. Each leaf fetches a cached
dependency build from Cachix and lints only the workspace crates. The time CI
spends on Nix Clippy drops, measured on a real PR run.

## Evidence (where the time goes today)

From the hermetic lane of cache-miss PR runs 44837736998 and 45049636974 (both
about 695 s from the first leaf starting to the last leaf finishing):

- Every leaf is a plain `runCommand` that compiles its whole dependency graph
  from scratch. Across the six leaves Cargo compiled or checked 2583 dependency
  units, but only 690 of them were distinct.
- Dependencies dominate. On product-core, the slowest leaf, about 590 s pass
  before its last dependency starts. Linting adds roughly 100 s after that.
  Derivation setup is 1–3 s.
- Nix runs four leaves at once on the 4-vCPU runner (the web leaves wait for a
  slot), so the leaves compete for the same CPUs while redoing the same
  dependency work.
- Merging the leaves into one derivation that shares a target directory does not
  pay. The host lane in the same run did exactly that (six units in sequence,
  one `target/`) and took 617 s.

## Approach and tradeoffs

Each leaf becomes a crane derivation (`mkCargoDerivation`). Its `cargoArtifacts`
is a `buildDepsOnly` that takes the same arguments. Both run the same
`dev static <leaf> --sandbox`, so the cargo command, features, target, profile
and environment are identical by construction. The dependency build runs over
crane's dummy source, which replaces workspace Rust files with stubs. The
artifact's key is therefore:

- every manifest and both lockfiles;
- `.cargo` config;
- the vendored `sqlx-sqlite` patch source, which is copied in real;
- the vendor directories and offline Cargo homes;
- the toolchain, crane and the native inputs (openssl, sqlite, dav1d);
- `devBin`, which is built from `tools/dev/`.

The single definition of each Clippy command stays in `tools/dev/dev/static.py`
(ADR-0146).

Supporting details, all found while probing:

- **One artifact per leaf.** The six leaves resolve different feature and target
  sets, so one leaf's dependency build does not serve another. A single union
  artifact (the six commands run in sequence over one target dir) would roughly
  halve storage. In exchange, every leaf would fetch and decompress the whole
  union, and a miss would run the dependency work serially, as the host lane's
  617 s does, before any leaf could start. Per-leaf artifacts reuse the leaf's
  own arguments unchanged and keep a miss parallel. Storage is the cost accepted
  (see below).
- **tools-clippy:** crane's dummy source keeps only the root lockfile, so
  `tools/Cargo.lock` is copied in. `CARGO_TARGET_DIR = "target"` stops nested
  tools Cargo from writing to `tools/target`, which crane never archives. The
  existing tools derivations set the same thing.
- **No debug info.** The leaves use the existing `leanDevAndTestProfile`, which
  is the repository's policy for compile-only gates. It sets
  `CARGO_PROFILE_{DEV,TEST}_DEBUG=0` in the environment only. Debug assertions,
  overflow checks and lint arguments are unchanged, and no workspace build
  script reads `DEBUG`, so lint results match host Clippy (ADR-0255). This cuts
  the compressed product-core `target.tar.zst` from 645 MB to 468 MB.
- **No nonempty-archive guard** like the tool builds' `test -s`. An empty
  archive would only cost a full rebuild, never a wrong verdict, and the log
  acceptance below catches it.

Invariant: real workspace and path-dependency source is always linted; only
registry and git dependencies are reused. This relies on the same crane
freshness behaviour the product binaries already depend on. Those binaries are
built over crane dummy-source artifacts (`cargoArtifacts` in
`nix/packages.nix`), so a Cargo change that kept the dummy crates would also
ship stub binaries and fail e2e. ADR-0256's stale-Clippy failure came from host
mtimes that drift. Here the source and the archive are both pinned to the Nix
epoch timestamp. The probe confirmed that the dummy-built workspace crates are
rechecked against the real source. A deliberate `unwrap()` in `client` failed
`client-host-clippy` while its dependency artifact was reused, and reverting it
left that artifact unchanged.

Costs this accepts:

- Cachix stores a dependency artifact for each leaf, about 1.5 GB in total
  (product-core alone is 468 MB). Every key change stores a new set. At the last
  two weeks' rate that is about 15 sets from `main`, plus PR iterations that
  change the key, so roughly 20–30 GB per two weeks. The repo does not record
  the cache's quota or garbage-collection policy, so that needs confirming
  before shipping.
- The artifacts are fetched only when a leaf misses and its artifact hits. When
  the leaf verdict itself is substituted, nothing is downloaded. The fetch and
  decompress time counts in the CI measurement.
- Any change to a manifest, a lockfile, the toolchain or `tools/dev/` rebuilds
  the artifacts. That miss costs today's time plus compressing and pushing the
  artifacts. In the last two weeks, 20 of the 35 merges to main that touched
  Rust changed none of those inputs.
- Taking `dev` out of the dependency key would require repeating the six cargo
  command lines in Nix. That would gain 4 of those 35 merges, so it is rejected.
- PR wall clock does not change: the e2e chain bounds it. The saving is
  runner-minutes in the hermetic lane.

Local probe on a 16-core machine, all leaves linting only workspace crates:
client-host 7 s (42 s dependency build), tools 23 s (113 s), product-core 170 s
(288 s). The CI saving is expected to be several minutes of the roughly 11.5
min, but it is not established until measured on CI.

Rejected alternatives: one shared-target derivation (saves about 75 s and loses
leaf parallelism); closing the issue as won't-fix (the user chose to pursue the
cached build).

## Acceptance

- `nix build .#checks.x86_64-linux.clippy-aggregate` passes. In each leaf's log,
  every workspace member that leaf lints appears as `Checking` or `Compiling`,
  and no registry or git crate appears.
- Canary: a deliberate `unwrap()` added to non-test code in `client` makes
  `client-host-clippy` fail with its dependency artifact already built. Run it,
  then revert it; nothing is committed.
- Before and after on CI, reported in the PR; nothing is checked in:
  - **Miss run:** the PR's first CI run builds and pushes the artifacts.
  - **Hit run:** then push a second, Rust-only commit, so the leaves miss but
    the artifacts hit.
  - **Baseline:** run 45049636974's 695 s Nix Clippy span (first leaf start to
    last leaf end).
- **Ship rule:** the hit-run span is at most 350 s (about half the baseline),
  and the miss-run span is no more than 120 s above 695 s. Otherwise, revert,
  post the measurements to #2038 and close it as won't-fix.
- Docs:
  - Update the leaf descriptions in `CONTRIBUTING.md` (around lines 1192–1207
    and 2307–2315) and the matching `docs/ARCHITECTURE.md` passages.
  - Rewrite `docs/ARCHITECTURE.md`'s "not a production dev/all-targets clippy
    preparation layer" sentence (around line 4987), which this change makes
    untrue. "Release artifacts are not assumed to seed dev-profile clippy" stays
    true.
- No ADR: this applies the existing `buildDepsOnly` pattern. ADR-0252 already
  treats these artifacts as ordinary build inputs, and ADR-0255 deferred this
  cost work explicitly.

## Boundaries

- No change to which units are linted, their arguments or `-D warnings`, or the
  host Clippy path. Clippy is not moved back to the host lane (ADR-0255).
- No change to Cachix `pushFilter` or substitution policy (ADR-0252). The
  artifacts are ordinary build inputs.
- No narrowing of the dependency key per leaf, and no Darwin claim.
