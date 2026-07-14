# Spec — #298: collapse the six server integration-test crates into one binary

**Issue:** [#298](https://github.com/jaunder-org/jaunder/issues/298) —
Restructure `server/tests` to drop compat re-exports and the `#[path]`-clone
pattern. **Milestone:** Code quality ratchet. **Date:** 2026-07-13.
**Incorporates #358** (landed 2026-07-13; consolidated the `post_form*` family
into `helpers/mod.rs`). This branch is rebased onto it, so `post_form` et al.
are now among the local helpers kept in `crate::helpers`.

## Problem / root cause

`server/tests/` holds six integration-test crates — `atompub`, `feed`, `misc`,
`projector`, `storage`, `web` — each a `tests/<x>/main.rs` cargo auto-discovers
as its own test-target crate. Shared helpers live in
`server/tests/helpers/mod.rs`, pulled into each crate with
`#[path = "../helpers/mod.rs"] mod helpers;`. Because that file is **recompiled
independently into all six binaries** and each uses a different subset:

- `helpers/mod.rs` carries `#![allow(dead_code)]` + two
  `#[allow(unused_imports)]` — the union is compiled everywhere but only partly
  used per-binary, and `#[expect]` can't be used (it would be "unfulfilled" in
  the binary that _does_ use a given item). These are the **3 `#[allow]`s** the
  issue targets.
- Each of the six `tests/<x>/main.rs` carries a crate-level
  `#![expect(clippy::unwrap_used, clippy::expect_used)]` (workspace lints deny
  both) — needed because clippy's `allow-*-in-tests` only exempts `#[test]`
  bodies, not the crates' many local non-`#[test]` helper fns (`post_form`,
  `make_app`, `seed_*`, `assert_*`, …). **6 `#![expect]`s.**

> **Correction to the issue comment.** The comment claims promoting the three
> shared helpers would let all six `#![expect]`s be removed "because their
> `#[test]` bodies are covered by clippy.toml." That is false: every one of the
> six crates has its _own_ local non-`#[test]` helpers using unwrap/expect
> (verified — ~40 across the files), so the suppression is still required
> per-crate. The six can only be reduced by changing the crate structure, not by
> relocating three functions.

## Decision — one integration-test binary

Collapse the six test crates into a **single** integration-test target. Chosen
over the alternative (a new `server-test-support` lib crate) after measuring the
cost:

- **Time-neutral.** Measured, warm deps: inner-loop relink after a one-file edit
  is **~2.7 s** with the current split; a combined binary relinks the _same_
  ~300 MB shared dependency blob (which dominates link time) so it is ~2.7–3.5 s
  — unchanged. Full test build is ~38 s either way (the split links that blob 6×
  redundantly; the combined binary links it once). One-time cold dep build
  (~1m23s) is irrelevant to the choice.
- **Disk win.** Six ~330 MB binaries (~2 GB, ~90 % duplicated deps) → one ~380
  MB binary, saving ~1.5 GB.
- **Folds suppressions.** One crate ⇒ one crate-level `#![expect]` (6 → 1). The
  `dead_code` allow drops because the shared `mod helpers;` is compiled once and
  every helper is used _somewhere_ in the single crate. The two `unused_imports`
  allows drop because **step 4 removes the blanket re-export** — four
  re-exported names (`postgres_only`, `sqlite_only`, `CloseablePool`,
  `PG_URL_FILE`) are unused suite-wide, so the collapse alone would _not_
  silence them (a `pub use` in a binary/test crate still warns on unused names);
  dropping the re-export and importing directly does.
- **No coverage/CRAP shift.** Helpers stay under `server/tests/` (CRAP-excluded
  via `**/tests/**`), unlike the lib-crate alternative which would move them
  into CRAP scope.

**Accepted cost:** loss of per-subsystem build isolation — a compile error in
one subsystem's tests now fails the whole integration binary, not just that
subsystem's.

## Scope of change

1. **Single target** in `server/Cargo.toml`: set `autotests = false` and declare
   `[[test]] name = "integration"`, `path = "tests/main.rs"` (explicit name
   avoids the auto-derived `main`).
2. **New `server/tests/main.rs`** (the one crate root):
   `#![expect(clippy::unwrap_used, clippy::expect_used)]`, then `mod helpers;`
   and `mod atompub; mod feed; mod misc; mod projector; mod storage; mod web;`.
3. **Rename** each `tests/<x>/main.rs` → `tests/<x>/mod.rs`; delete its
   crate-level `#![expect]` and its `#[path] mod helpers;` line; keep its
   `mod <submodule>;` declarations (they still resolve within the subdir).
4. **`tests/helpers/mod.rs`:** delete `#![allow(dead_code)]` and both
   `#[allow(unused_imports)]`; **drop the `pub use storage::test_support::{…}`
   blanket re-export**; keep the local helpers (`ensure_server_fns_registered`,
   `test_options`, `tmp_storage_path`) and `mod websub_capturing;` with its
   `pub use CapturingWebSubClient` (now used by the `feed` module in-crate → no
   allow needed).
5. **Convert import sites** in the ~27 test files: split each
   `use crate::helpers::{…}` into `use storage::test_support::{…}` (harness
   names: `Backend`, `TestEnv`, `TestBase`, `backends`, `backends_matrix`,
   `noop_mailer`, `seed_posts`, `*_postgres_url`, `PostgresDbGuard`,
   `sqlite_only`/`postgres_only`, …) plus a `use crate::helpers::{…}` keeping
   only the names `helpers` itself **defines**: the `post_form*` family
   (`post_form`, `post_form_with_mailer`, `post_form_with_secure_flag`,
   `post_form_with_ua`, `post_form_with_bearer` — from #358),
   `ensure_server_fns_registered`, `test_options`, `tmp_storage_path`,
   `CapturingWebSubClient`. Rewrite inline fully-qualified calls the same way
   (`crate::helpers::noop_mailer()` → `storage::test_support::noop_mailer()`;
   `crate::helpers::tmp_storage_path()` stays).
6. **Fix the two intra-`misc` cross-refs** (`crate::backup_fixture` →
   `crate::misc::backup_fixture` or `super::backup_fixture`) at
   `misc/backup_interop.rs:8` and `misc/commands.rs:23`.
7. **Resolve `websub_capturing` dead-code.** The removed `#![allow(dead_code)]`
   also blanketed `mod websub_capturing`, where `CapturedPing.sent_at`
   (`websub_capturing.rs:39`) is written but never read (`feed_worker.rs` reads
   only `.hub_url`/`.feed_url`; `#[derive(Debug)]` no longer counts as a read
   for the "field is never read" lint). Once the allow is gone this warns.
   Resolve by **removing the `sent_at` field** (and its `Utc::now()` write + the
   now-unused `chrono` import) — not by re-adding a suppression. (If a future
   test needs the timestamp, it adds an assertion that reads it; today none
   does.)
8. **ADR** recording the decision (see below).

## Acceptance criteria (observable)

- `server/tests/helpers/mod.rs` contains **zero** `#[allow]`/`#[expect]`
  attributes.
- Exactly **one** crate-level
  `#![expect(clippy::unwrap_used, clippy::expect_used)]` exists under
  `server/tests/`, in `tests/main.rs` (was 6). Net attributes: −3 `#[allow]`, −5
  `#[expect]`; **no new suppressions of any kind** introduced anywhere (in
  particular the `websub_capturing` dead-code is fixed by removing the unread
  field, per step 7 — not by a replacement allow).
- A short **ADR is recorded** (draft under `docs/adr/drafts/`) capturing the
  one-binary decision, its tradeoff, and the relationship to ADR-0033 /
  ADR-0046.
- **No `#[path]`** attribute remains anywhere under `server/tests/`.
- `helpers/mod.rs` has **no** `pub use storage::test_support::…` re-export;
  harness names are imported from `storage::test_support` at each use site.
- `cargo xtask validate` is green (static + clippy + coverage + all four e2e
  combos).
- The integration suite builds as a **single** target and every
  previously-passing test still passes (same test count; nextest paths gain a
  `<subsystem>::` prefix).

## Risks / verification

- **External refs to the old target names.** Grep of `.github/`, `flake.nix`,
  `xtask/`, nextest config found **none** — the six names aren't hardcoded.
  Re-verify at implementation; confirm the coverage classifier
  (`tools/devtool/src/coverage/emit.rs` `classify_nextest_output`) still
  classifies correctly when the binary is `integration` and test paths carry the
  `<subsystem>::` prefix.
- **Cross-crate `#[apply(...)]` templates.** After dropping the re-export, files
  do `use storage::test_support::backends;` + `#[apply(backends)]` — the same
  crate boundary the current re-export already crosses (`atompub_rsd.rs`
  documents that no `#[apply(path::…)]` / `pub use` is needed). Verify a
  backend-parametric test still expands to
  `::case_1_sqlite`/`::case_2_postgres`.
- **CONTRIBUTING references** to `server/tests/helpers/mod.rs` (as where
  `Backend::setup` lives) become stale-adjacent; update the prose to reflect
  that the harness is `storage::test_support` imported directly.
- **#358 incorporated.** #358 (landed) consolidated the `post_form*` family into
  `helpers/mod.rs`; this branch is rebased onto it, so those are now local
  helpers kept via `crate::helpers` (step 5). Their unwrap/expect is covered by
  the single crate-level `#![expect]`. The web files no longer define a local
  `post_form` — verify none remains.

## Out of scope — follow-ups (filed)

The collapse surfaces further copy-pasted local helpers now trivially
consolidatable into `helpers/`. These are entropy-fighting cleanups like #358 —
**not** required to remove any suppression — and are filed as separate issues,
both **blocked-by #298**:

- **#429** — HTTP request/response plumbing: `make_app` (×6), `body_string`
  (×4), `post_json` (×3), `get_asset`.
- **#430** — auth/session/user fixtures: `create_session_cookie` (×3),
  `make_user` (×2), `cookie_for` (×2).

They are **not** part of #298's scope; #298 does not touch these helpers beyond
the mechanical import-conversion its collapse requires.

## Decision record

Write a short ADR (draft via `jaunder-adr`): _server integration tests are one
binary_ — why (retires the `#[path]`-clone that forced the suppressions; folds 6
`#![expect]` → 1), the accepted tradeoff (lost per-subsystem build isolation),
and the relationship to the in-process `storage::test_support` harness
(ADR-0033) and the out-of-process `test-support` seed binary (ADR-0046).
