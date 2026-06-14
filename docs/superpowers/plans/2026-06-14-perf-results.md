# Performance results — PostgreSQL parallel tests / host coverage / VM consolidation

Tracking epic **jaunder-k4tb**. Methodology and per-phase steps: see
`2026-06-14-postgres-parallel-tests-and-host-coverage.md`.

## Environment
- Host: 16 cores, 30 GiB RAM (~20 GiB available at capture)
- Timing: bash builtin `time` (real/wall clock). GNU `/usr/bin/time` is not
  available on host or in the devShell, so peak RSS is noted qualitatively
  (CPU saturation / OOM behaviour) rather than measured precisely.
- Caches: cargo `target/` warmed once (`cargo build --workspace --tests`)
  before host-step timings, so those reflect test execution, not first compile.
  Nix VM rows are forced **cold** (build → `nix store delete` → rebuild) to
  reflect the cost paid after a source change, not a cache hit.
- Sizing context: 23 PostgreSQL integration binaries each run in their own
  NixOS VM at `memorySize = 4096`. At Nix's default parallelism (~cores) that
  is far more than 30 GiB of RAM, so the baseline either serialises VM builds
  or approaches OOM — the motivating problem.
- Date captured: 2026-06-14

## Results (wall clock, real time; host rows = median of 3, min–max in parens)

| Metric | Baseline | After Phase 1 | After Phase 3 | Final |
|--------|----------|---------------|---------------|-------|
| `scripts/verify --fast` | 2.0s (warm; first run 49.7s incl. clippy-profile compile) | — | — | |
| `scripts/check-coverage` (host; SQLite only at baseline) | 270.7s | | — | |
| PostgreSQL integration run — serial (`--test-threads=1`) | 228s exec (643 tests; 448s wall on first run incl. test compile) | | — | — |
| PostgreSQL integration run — parallel (default threads) | — | | — | |
| One postgres VM check (cold, `postgres-commands`) | 42.8s | — | n/a | n/a |
| Postgres integration VMs total (`postgres-integration-checks`, 23 VMs cold) | 222.1s | — | | |
| `nix build .#nix-only-checks` (cold) | see Notes (composite) | — | | |
| Full `scripts/verify` (default tier) | see Notes | | | |
| Full `scripts/verify --full` (with VM) | n/a (no `--full` flag at baseline) | | | |
| Peak memory / OOM notes | no OOM — `max-jobs=4` serialises VMs into batches | | | |

## Notes

### Baseline (2026-06-14, captured before any code change)

Decision-relevant, directly-measured numbers:

- **Serial PostgreSQL run: 228s execution** (643 tests, 25 binaries, all pass;
  `--test-threads=1`, current shared-DB `reset_postgres_schema` model). This is
  what Phase 1 (per-test DBs + parallel) targets. First-run wall was 448s
  because nextest recompiled the test profile; the 228s is nextest's own
  execution summary and the fair figure for the parallel comparison.
- **23 PostgreSQL VMs (`postgres-integration-checks`) cold: 222.1s** for one
  `nix build` invocation that re-ran all 23 `vm-test-run` derivations (verified
  count). This is what Phase 3 (consolidate to one VM) targets.
- **One PostgreSQL VM cold: 42.8s** (`postgres-commands` via `--rebuild`).
- `check-coverage` (host, SQLite only): 270.7s — always re-runs the suite under
  llvm-cov instrumentation (not cached). Phase 2 adds a host-PG pass on top.

Supporting / context:

- **e2e-sqlite VM cold (from clean): 530.9s** — dominated by the one-time
  `jaunderBin` (release) + wasm bundle + Playwright dependency compile, which is
  shared with `e2e-postgres`. Not a per-run cost once those deps are cached.
- **`nix build .#nix-only-checks` (cold), composite:** the full cold cost is
  the one-time compile of `cargoArtifacts`/`jaunderBin`/wasm/integration
  binaries (≈ the 531s seen on the first e2e build, mostly compile) plus the
  ~25 VM *runs* (23 PG ≈ 222s + 2 e2e VM runs), overlapping under `max-jobs=4`.
  Measured directly only in parts (see above); a single forced-cold run was not
  repeated because it re-incurs the multi-minute release compile already
  measured. When VM *runs* are forced cold but compiles stay cached, the phase
  is dominated by the 222s PG-VM batch.
- **`scripts/verify` full (baseline):** the current script runs everything
  (fast → clippy → coverage → `nix-only-checks`). Felt cost ≈ clippy (~50s cold,
  ~2s warm) + coverage (271s, always) + the VM phase above. Cold ≈ ~16 min;
  warm (VMs cached, only coverage re-runs) ≈ ~5 min, coverage-dominated.

### Resource behaviour

`max-jobs=4`, `cores=8` (16 physical cores, 30 GiB RAM). VM tests at
`memorySize=4096` run **4 at a time** (~16–20 GiB), so the 23 PG VMs serialise
into ~6 batches rather than over-committing memory. The constraint manifests as
**serialised wall-clock**, not an OOM crash, at this setting. Consolidating to
one VM (Phase 3) removes ~22 redundant boots and the batch serialisation;
moving the PG suite to the host (Phases 1–2) removes the VM boots entirely for
the local inner loop.
