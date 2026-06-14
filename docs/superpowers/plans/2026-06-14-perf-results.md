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
| `scripts/verify --fast` | 2.0s (warm; first run 49.7s incl. clippy-profile compile) | 2.0s (unchanged) | — | |
| `scripts/check-coverage` (host; SQLite only at baseline) | 270.7s | 270.7s (still SQLite only) | 533s (Phase 2: +host-PG pass; ~733s in Nix sandbox) | |
| PostgreSQL integration run — serial (`--test-threads=1`) | 228s exec (643 tests; 448s wall on first run incl. test compile) | n/a (replaced by parallel) | — | — |
| PostgreSQL integration run — parallel (default threads) | — | **28.5s exec** (median of 3: 28.3/28.5/28.6s; ~35s wall), 643 tests, 3× clean | — | |
| One postgres VM check (cold, `postgres-commands`) | 42.8s | — | n/a | n/a |
| Postgres integration VMs total (`postgres-integration-checks`, 23 VMs cold) | 222.1s | — | **64.2s** (1 VM, all binaries, multithreaded, fsync off) | |
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

### After Phase 1 (per-test PostgreSQL databases, 2026-06-14)

- **PostgreSQL suite: 228s → 28.5s execution (~8× faster)** by dropping
  `--test-threads=1` and giving each test its own template-cloned database.
  643 tests, three consecutive clean runs (28.3 / 28.5 / 28.6s) — no flakes, no
  cluster-global collisions, so the planned nextest serial test-group was not
  needed (the role-provisioning tests already use unique suffixed names).
- `--fast` and `check-coverage` unchanged at this phase (coverage still
  SQLite-only until Phase 2 adds the host-PG pass).

### After Phase 2 (host-PostgreSQL coverage pass, 2026-06-14)

- **PostgreSQL storage coverage is now measured** (was a blanket 3–15% gap):
  `backup.rs` 13%→95%, `bootstrap.rs` 63%→97%, `feed_cache.rs` 23%→100%,
  `feed_events.rs` 14%→90%, `posts.rs` 2%→82%, plus smaller bumps elsewhere.
  Remaining low and now *visible* (real gaps, not measurement artifacts):
  `media.rs` ~5%, and partial `sessions`/`users`/`invites`.
- **Cost:** `check-coverage` 270.7s (SQLite only) → ~533s host (2-pass) /
  ~733s in the Nix `coverage` sandbox. The PG pass roughly doubles coverage
  time (re-runs the jaunder suite against PG under instrumentation).
- Postgres runs inside the Nix build sandbox (validated). The committed baseline
  is **generated in that sandbox**, because the sandbox has no network and a few
  files (`websub/http.rs`, `commands.rs`) report lower there than on a networked
  host; a host-generated baseline would exceed what CI can reproduce.

### After Phase 3 (consolidate 23 VMs → 1, 2026-06-14)

- **PostgreSQL VM phase: 222s (23 VMs) → 64.2s (1 VM), ~3.5×**, and one 4 GB VM
  instead of up to four concurrent 4 GB VMs — the resource/OOM win.
- Key enabler: per-test databases (Phase 1) let the binaries run with libtest's
  normal **in-process parallelism** (no `--test-threads=1`). The inherited
  single-threaded rule was verified stale — `web_posts` ran 3/3 and the whole
  suite 2/2 multithreaded against PG with no Leptos dispose panics. Upstream,
  the reactive arena (`reactive_graph` `owner/arena.rs`) is a process-global
  keyed `SlotMap`; the real hazard was async use-after-dispose (since fixed),
  not slot collision, and process-per-test (nextest) remains the bulletproof
  fallback if any flake ever appears.
- **Durability matters in the VM:** the first consolidated build was 489s
  because NixOS Postgres defaults to `fsync=on` and the suite creates ~643
  template-clone databases. Setting `fsync/synchronous_commit/full_page_writes`
  off (throwaway VM) dropped it to 64.2s.

### Resource behaviour

`max-jobs=4`, `cores=8` (16 physical cores, 30 GiB RAM). VM tests at
`memorySize=4096` run **4 at a time** (~16–20 GiB), so the 23 PG VMs serialise
into ~6 batches rather than over-committing memory. The constraint manifests as
**serialised wall-clock**, not an OOM crash, at this setting. Consolidating to
one VM (Phase 3) removes ~22 redundant boots and the batch serialisation;
moving the PG suite to the host (Phases 1–2) removes the VM boots entirely for
the local inner loop.
