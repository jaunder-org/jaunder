# Design: Speed up the non-e2e test/coverage gate

**Date:** 2026-06-21
**Status:** Draft — awaiting review
**Driver:** The non-e2e CI gate (`cargo xtask validate --no-e2e`) takes ~12 min. We want it materially faster without weakening coverage or the backend-parity / coverage-ratchet invariants.

## Background & measurement

We profiled the coverage step (the dominant non-e2e cost) faithfully — same commands the Nix `coverage` check runs (`scripts/check-coverage`: instrumented build, then the full suite under an ephemeral PostgreSQL via `scripts/with-ephemeral-postgres`). Measured on the dev host with deps cached:

| Component | Time | Notes |
|---|---|---|
| Instrumented **build/link** | **~250s** (deps cached); ~440–530s fully cold | Compiling + linking the workspace + **25 separate integration-test binaries** under `-Cinstrument-coverage` |
| Test **execution** | **~76s** | 1530 tests, both backends, all passing |
| …of which a handful of slow tests | ~35–40s | see below |

Key conclusion: **the cost is compilation/linking (~3× execution), not test count.** Execution itself is concentrated in a few slow tests, not spread across the 1530.

Execution long-poles (from the nextest SLOW report, `> 5s` / `> 10s`):

- **4 `web_posts` pagination tests > 10s each** — `get_post_finds_author_draft_across_multiple_pages`, `list_home_feed_returns_authenticated_users_published_posts_only`, `list_local_timeline_returns_published_posts_with_cursor_pagination`, `list_user_posts_returns_published_posts_with_cursor_pagination` (sqlite case). Each seeds 26–55 posts through the full web handler.
- **3 `storage` auth tests > 5s** — `set_password_authenticate_with_old_returns_invalid_and_new_succeeds`, `use_invite_with_valid_code_marks_it_used`, `use_password_reset_already_used_returns_already_used`. Argon2 KDF cost, amplified by instrumentation.
- `backup_worker_executes_scheduled_backup`, 2 `media_manager` upload tests > 5s.

### Measurement caveat recorded for posterity

Running the instrumented suite under context-mode's sandbox sets `TMPDIR` to an ephemeral `/tmp/.ctx-mode-*` dir that is torn down when the launching call returns; this makes every `tempfile`-based test panic (false failures) and breaks later cargo phases. Profiling must export a stable `TMPDIR`. With a stable `TMPDIR`, all 1530 tests pass.

## Goal

Cut the non-e2e gate wall-clock, primarily by attacking instrumented build/link time, secondarily by fixing the few genuinely slow tests.

## Non-goals (explicitly out of scope)

- **Unit ↔ integration overlap dedup.** The original hypothesis was that storage-unit vs server-integration overlap (208 vs 140 tests) drives the cost. Measurement disproves this for speed: those tests are individually fast (0.01–0.07s); deleting 100 saves ~1–2s. Not pursued here. (May still be worth doing later on maintenance grounds, separately.)
- **Instrumenting the e2e (Playwright) suite for coverage.** Large, separate effort; not needed for this goal.
- **Lowering the coverage baseline or the CRAP ratchet.** Invariant preserved.

## Workstreams

### 1. Consolidate integration-test binaries (25 → 5) — biggest, lowest-risk

**Why:** Cargo compiles each `.rs` directly under `server/tests/` as its own binary (25 today), each separately linking the `server` rlib + harness. Instrumented linking is the slow part; 25 links dominate the build. Files in a *subdirectory* are not auto-compiled as binaries — they become modules of a single `main.rs`. `server/Cargo.toml` has **no `[[test]]` entries and no `harness=false`**, so this is pure auto-discovery with zero Cargo.toml test migration.

**Grouping (judgment call, approved as a starting point):**

| Binary | Files |
|---|---|
| `storage` | `storage.rs` alone (6,635 lines / 148 tests — heaviest; isolating it bounds its relink) |
| `web` | the 12 `web_*` files |
| `atompub` | `atompub_posts/media/rsd/service` |
| `feed` | `feed_worker/events_hook/handlers/regenerate` |
| `misc` | `commands`, `backup_interop`, `media_handlers`, `static_assets` |

**Layout per group:** `server/tests/<group>/main.rs` containing only:
```rust
#[path = "../helpers/mod.rs"]
mod helpers;
mod web_posts;
mod web_auth;
// … one line per file in the group
```
`git mv` each file into its group dir. Shared `helpers/` stays at `server/tests/helpers/`.

**The one mechanical edit per file:** today each root-level file declares `mod helpers;` (resolving to `tests/helpers/mod.rs`). As a submodule that path is wrong, and helpers must be declared once per group binary. So: drop the per-file `mod helpers;`, declare it once in the group `main.rs` (via `#[path]`), and rewrite in-file `helpers::X` references to `crate::helpers::X` (or add `use crate::helpers;`).

**Collision check (done):** top-level helper names are reused across files (`post_form` in 11 files, `make_app` in 6, `body_string`, `seed_alice`, `make_user`, `cookie_for`, …). These collide **only if files are flattened into one module**. Keeping each file as its own `mod` makes them distinct paths (`web_posts::post_form` ≠ `web_account::post_form`) — **no collisions**. This is why we do not use a single flat file.

**Risk:** Low — pure file reorg + mechanical `helpers` rewiring; no test logic changes. Trade-off: editing one test now relinks its whole domain binary (not just that file), but cuts CI cold links 25→5. The 5-way split bounds the local-incremental cost.

### 2. Seed-direct helper for pagination tests — cheap, safe

**Why:** The 4 slow `web_posts` tests seed posts via `create_post_json` → HTTP POST `/api/create_post` → full server-fn (markdown render → slug → storage insert), ×26–55 sequentially. The tests assert on the **list/pagination** endpoint; creation is just setup.

**Handling:** Add a `seed_posts(state, author_id, n)` test helper that calls the storage create directly with a stub `rendered_html`, bypassing the HTTP layer and markdown render (the server-fn already threads `rendered_html` into storage — `web/src/posts/server.rs` shows the seam). Replace the loops with one helper call. Same DB rows and pagination behavior under test.

**Open detail (pin at implementation):** the exact storage entry point (`post_service` / `state.posts` create method) — the quick scan didn't surface the method name.

**Risk:** Low — test-helper only; the list/pagination path under test is unchanged.

### 3. KDF Path A — test-only cheap Argon2, with a hard production lock

**Why:** `common/src/password.rs` hardcodes `Argon2::default()` (m=19 MB, t=2) in `hash()`; no knob. Every `create_user`/auth test pays full cost, amplified by instrumentation. Useful fact: `verify()` reads cost from the stored PHC hash string, so a cheap-param hash verifies cheaply automatically — only `hash()` and the timing-equalization dummy need a cheap variant.

**Mechanism (rides the existing, already-audited test-only boundary):**

- The workspace uses **`resolver = "2"`**, so features enabled via `[dev-dependencies]` are **not unified into normal builds**.
- `common`/`storage` already expose a `test-utils` feature enabled *only* through dev-deps (e.g. `server` prod dep = `common/metrics`; dev-dep = `common/test-utils`). Production never sees it.
- Add a `cheap-kdf` feature to `common`, **not in `default`**; set `test-utils = ["cheap-kdf"]` so existing dev-dep wiring picks it up with no new plumbing.
- Cheap params live behind `#[cfg(feature = "cheap-kdf")]` in `password.rs`. Feature off (every production build) ⇒ the code does not exist; `hash()` is unconditionally `Argon2::default()`.

**Production "no chance" lock (two independent gates):** A naive `#[cfg(feature = "cheap-kdf")] compile_error!(…)` does **not** work: the `jaunder` binary is compiled during `cargo test` with the feature unified on, so it would break the test build. The distinguishing signal between a test build and a production build is `debug_assertions` (on in dev/test, off in `--release`). So:

1. **Compile-time tripwire** in the server `[[bin]]` (`main.rs`):
   ```rust
   #[cfg(all(feature = "cheap-kdf", not(debug_assertions)))]
   compile_error!("cheap-kdf must never be compiled into a release jaunder binary");
   ```
   A release build (production, `debug_assertions` off) with the feature on **fails to compile**. Test builds (`debug_assertions` on) are unaffected.
2. **Runtime fail-closed guard** at the top of `main()` as the catch-all for any build profile (e.g. a debug production binary):
   ```rust
   #[cfg(feature = "cheap-kdf")]
   { eprintln!("FATAL: cheap-kdf compiled into the server binary"); std::process::exit(1); }
   ```
   `main()` is not executed by the integration tests, so this never fires during tests; a production binary that somehow carried the feature would refuse to start rather than hash cheaply.

Together: production is protected by feature *absence* (resolver-v2 dev-dep isolation), a release build can't even compile with the feature, and any residual case fails closed at startup.

**Preserve real-KDF coverage:** Because the whole-workspace coverage run unifies `test-utils` on, `common`'s own password tests would otherwise all go cheap, leaving nothing exercising production-strength Argon2. Keep one explicit full-params correctness test (e.g. a `hash_with_params(PRODUCTION_PARAMS)` seam) independent of the feature.

**Timing-parity ripple (§2.1):** The dummy Argon2 hash in `storage/src/helpers.rs` (used to equalize not-found auth timing against the username-enumeration oracle) must switch to the cheap-param variant under the same feature, so real and dummy hashes keep matching params and the timing-parity tests stay coherent.

**Pre-check:** Scan auth tests for any asserting on literal default params (e.g. expecting `m=19456` in a hash string); adjust those.

**Risk:** Medium — touches a security-sensitive path. Mitigated by: compile-time absence in prod, the `compile_error!` lock, preserved full-params test, and resolver-v2 isolation.

## Invariants preserved

- **Coverage ratchet** (`coverage-baseline.json`) and **CRAP** (`crap-manifest.json`): no source lines removed; test reorg doesn't change `server/src` coverage. Run `cargo xtask check` after, expect clean/auto-heal only.
- **Backend parity:** no storage trait/behavior changes; both backends still run in the same nextest pass.
- **Security:** production hashing strength unchanged and locked at compile time.

## Verification

- Re-profile the coverage step before/after each workstream using a **stable `TMPDIR`**; record build vs execution split.
- Targets: workstream 1 cuts build (expect the ~250s build to drop substantially); workstreams 2–3 cut execution (expect the ~76s, and specifically the named SLOW tests, to drop).
- Gate: `cargo xtask validate` (or `--no-e2e` while iterating) stays green; coverage/CRAP ratchet clean.

### Measured results (2026-06-21)

Rigorous before/after: `main` (`87e04b9`) vs `ci-test-speed` (`64c9620`), identical harness — whole-workspace instrumented coverage suite, both backends, full cold build each, stable `TMPDIR`. Build derived as `run_total − execution` (execution = nextest `Summary` bracket).

| Metric | BEFORE (`main`) | AFTER (branch) | Δ |
|---|---|---|---|
| Coverage step total (build + exec) | 296s | 199s | **−97s (−33%)** |
| Instrumented build | ~229s | ~157s | **−72s (−31%)** |
| Test execution | 67.2s | 41.7s | **−25.5s (−38%)** |
| Tests | 1434 | 1435 (+1 prod-params test) | — |

- **Build (−31%)** is the deterministic headline — 23→5 instrumented test-binary links. Confirms the profiling thesis that build dominates.
- **Execution (−38%)** is real (the total bracket is robust) and is mostly seed-direct: the 4 `web_posts` pagination tests were each **>10s** on `main` and are gone from the slow list on the branch. cheap-kdf additionally dropped `set_password` below the 5s threshold.
- **Caveat — per-test SLOW flags are contention-noisy:** AFTER reported 12 slow `storage::use_*` token tests that BEFORE did not, yet AFTER's total execution is 25s *faster* — scheduling contention near the 5s line, not a regression. Trust the total bracket, not the slow list.
- **Correction to the plan's KDF attribution:** cheap-kdf is active (it sped up `set_password`) but did **not** speed up the storage token tests — those are database-setup-bound, not Argon2-bound. The build consolidation and seed-direct carried the win; KDF's payoff is smaller and more diffuse than assumed.

`cargo xtask validate` passes (static + clippy + coverage clean + e2e on both backends).

## Sequencing

1. **Workstream 1 (consolidation)** — independent, biggest payoff, lowest risk. Land first.
2. **Workstream 2 (seed-direct)** — independent, small, safe.
3. **Workstream 3 (KDF)** — independent; do last as the security-sensitive item, with its own focused review.

Each is independently shippable and independently verifiable.

## Deferred / future

- ~~The double-compilation question (clippy and coverage each do a full separate workspace build) — plausibly as large a lever as consolidation.~~ **Investigated + measured 2026-06-21 — hypothesis refuted.** CI's only uncached full workspace build is the host `cargo clippy --all-targets` (the `actions/cache` covers `~/.cargo` + `xtask/target`, not the main `target/`). But clippy is a *check*-mode build (`.rmeta` only, no codegen/linking), so a full cold run is just **~56s** on a many-core box (~2–3× on a smaller CI runner), not the minutes assumed. The dominant CI costs are the *codegen* builds — the instrumented coverage build and e2e — both already cachix-cached. The flake already has an unused `clippy` crane check; routing `validate`'s clippy to it would recover most of the ~56s (deps from cachix) but is a modest, low-priority cleanup, not a major lever. Tracked as bead `jaunder-b2i1`.
- Unit↔integration overlap cleanup on maintenance grounds (not speed).
- Mutation-testing round (noted: the prior round surfaced missing tests, not redundant ones).
