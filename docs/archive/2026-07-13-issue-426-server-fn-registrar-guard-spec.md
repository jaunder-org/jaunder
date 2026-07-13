# Spec — guard that every `web` `#[server]` fn is in the test registrar (#426)

Status: awaiting approval · Issue: #426 · Milestone: Test infrastructure & E2E

## Problem

The integration-test suite routes `web` server functions only if their generated
types are named in a **hand-maintained** registrar
(`server_fn::axum::register_explicit::<web::…>()`), because the integration-test
binary links `jaunder`/`web` as rlibs and dead-code elimination drops each
`#[server]` macro's auto-registration unless the type is referenced explicitly.
The hand list rots: a new `#[server]` fn compiles and passes its own crate's
tests, but its integration route silently 404s until someone remembers to add it
(surfaced in #358 — `web_media` had quietly patched in three fns missing from
the shared registrar).

There are **two** such lists today:

- `server/tests/helpers/mod.rs::ensure_server_fns_registered()` — 44 entries,
  `OnceLock`-guarded; meant to be the complete set.
- `server/src/lib.rs`'s `#[cfg(test)] mod tests::ensure_server_fns_registered()`
  — a 6-entry subset, only what the in-file router tests need.

**The drift is already real.** 10 `web` `#[server]` fns are defined but absent
from the integration registrar (so any test routing them 404s now): the whole
`media` module (`ListMyMedia`, `MediaUsage`, `DeleteMedia`),
`posts::{DeletePost, UnpublishPost, DefaultAudienceSelection, PostAudienceSelection}`,
`profile::{GetDefaultPostFormat, SetDefaultPostFormat}`,
`sessions::CreateAppPassword`.

## Decision

Chosen: **Approach C (consolidate to one registrar) + mandatory (no per-fn
opt-out) + a `syn`-based `xtask` gate.** Rejected: A (keep two lists, guard
them) and B (`linkme`/wrapper-macro auto-registration). Recorded in an ADR draft
(see §4).

- **Why C over A:** collapsing to one list removes the duplication that caused
  the second list's own latent rot and gives the gate a single unambiguous
  target.
- **Why not B:** B is the "make illegal states unrepresentable" ideal, but no
  `inventory`/`linkme` exists in the repo today, the cross-rlib linkage that
  forced `register_explicit` in the first place makes a `linkme` slice's
  survival uncertain, and it would touch every `#[server]` call site plus the
  coverage-measured `macros` crate. Not worth it for a gate-caught guarantee.
- **Why mandatory:** the strongest guarantee and the issue's intent;
  registration is harmless (it only makes a route available), so the 10 current
  gaps are simply registered rather than exempted.

## Design

### 1. One canonical registrar

- `server/tests/helpers/mod.rs::ensure_server_fns_registered()` becomes the sole
  registrar list.
- Delete `server/src/lib.rs`'s `#[cfg(test)]` `ensure_server_fns_registered()`
  (and its `register_explicit` calls).
- Relocate the `lib.rs` router unit-tests that depend on server-fn registration
  — `home_route_returns_ok`,
  `spa_fallback_serves_embedded_shell_without_disk_index_html`,
  `current_user_api_route_returns_ok` (and `home_response_contains_app_content`
  for cohesion) — into an integration test under `server/tests/` that calls
  `helpers::ensure_server_fns_registered()`. They exercise only the **public**
  `jaunder::create_router` (+ `AppState` via `storage::open_database`,
  `common::mailer::NoopMailSender`), which is exactly the idiom ~20 existing
  `server/tests/**` files already use (`ensure_server_fns_registered()` +
  `jaunder::create_router(...)` + `oneshot`), so no private access is needed.
  The `lib.rs`-private test shims (`test_state`, `test_storage_path`,
  `test_mailer`, `test_options`) are trivial public-API wrappers and must be
  re-created in the integration test — `helpers` already exposes `test_options`
  and `noop_mailer`; the `sqlite::memory:` state and temp-path shims are a few
  lines over `storage::open_database` / `common::mailer::NoopMailSender`.
- **Why relocation, not a shared `server` fn behind a `test-support` feature:**
  `server_fn` is a **dev-dependency** of `server`, which is why the registrar
  lives in test contexts today. Hosting a shared `pub fn register_all()` in
  `server/src` would require promoting `server_fn` to an optional dependency, a
  new `test-support` feature, and a self dev-dependency to enable it for the
  integration tests. Relocation reuses the existing integration idiom with zero
  Cargo/feature surface, and touches the `helpers` function (which #358 is
  editing concurrently) less than a full rewrite of its body would.

### 2. Reconcile the current drift

Add every `web` `#[server]` fn still missing from
`ensure_server_fns_registered()` after rebasing onto #358 (#358 folds the
`media` trio; do not double-add). Mandatory — no exemptions.

### 3. The gate — a `server-fn-registrar` `xtask` check

New `xtask/src/steps/server_fn_registrar_check.rs`, a sibling of
`test_pattern_check.rs` / `sequence_check.rs`:

- **Enumerate** every `#[server]` fn in `web/src/**/*.rs` with `syn`
  (`parse_file` + `syn::visit::Visit`, exactly as `xtask/src/coverage/exempt.rs`
  does — it already has a `does_not_exempt_server_fn` fixture showing a
  `#[server]` fn to the visitor). Collect free `ItemFn`s whose attributes
  include a `server` path; the generated type is `PascalCase(fn ident)`. This
  repo uses only the `#[server(endpoint = "…")]` form (no positional type
  rename), so the PascalCase mapping is exact; the check treats an unexpected
  positional-rename form as a hard error rather than silently mis-naming.
- **Parse** `server/tests/helpers/mod.rs` for the registered leaf type names in
  `register_explicit::<web::…::LEAF>()`.
- **Match by leaf type name**, not module path: re-exports
  (`web/src/posts/mod.rs` does `mod listing; pub use listing::*;`) make the
  registrar path (`web::posts::ListLocalTimeline`) differ from the source path
  (`web::posts::listing::…`). Accepted caveat (documented in the ADR): two
  same-named `#[server]` fns in different modules collapse to one leaf; this is
  benign because they would also collide at the `endpoint` level. **Precondition
  audit:** at implementation, scan the enumerated `web` `#[server]` fns for a
  cross-module same-name collision and assert none exists, so the gate is not
  shipped already blind to a real pair (none was found during design, but the
  tree must confirm it, not assume it).
- **Only the missing direction is checked.** A stale registrar entry (a type
  that no longer exists) already fails to compile, so the gate need not check
  it.
- **Failure detail** names each unregistered fn's `file:line` plus a `recovery:`
  line pointing at `server/tests/helpers/mod.rs`, mirroring the sibling checks.
- **Pure core:** `problems(web_fns, registrar_src) -> Option<String>` (or
  equivalent pure signature over already-read inputs), unit-tested directly with
  string fixtures; the `run(result: &mut CommandResult)` wrapper does the I/O
  and pushes exactly one `StepResult`.
- **Wire in:** add `pub mod server_fn_registrar_check;` under `mod steps` and a
  `steps::server_fn_registrar_check::run(&mut result);` call in **both** the
  `check` (Fix) and `validate` (Check) command sequences in `xtask/src/lib.rs`,
  positioned next to `test_pattern_check`.

### 4. ADR

An ADR draft (`docs/adr/drafts/`, numberless per ADR-0048; numbered at ship by
`cargo xtask adr promote`) records: the chosen approach (C + mandatory + syn
gate), the rejected A and B with reasons, the leaf-name-matching caveat, and the
`server_fn`-is-a-dev-dep rationale for relocation over a `test-support` feature.

## Acceptance criteria (observable)

1. **The gate catches an omission.** Adding a `#[server]` fn to `web` without
   registering it makes `cargo xtask check` (and `validate`) fail with a message
   that names the fn and its `web/src/...:line`. Verify by adding a throwaway
   `#[server]` fn, running `check` (red), then removing it.
2. **Exactly one registrar list remains.** `rg 'register_explicit'` over
   `server/` matches only under `server/tests/`; `server/src/lib.rs` no longer
   defines `ensure_server_fns_registered` or any `register_explicit` call.
3. **The reconciled tree is green.** With every `web` `#[server]` fn registered,
   `cargo xtask check` passes (the gate is satisfied).
4. **The relocated router tests still pass and still assert their behavior:**
   200 on `/`, the embedded shell (`init("/pkg/jaunder.wasm")`) served on the
   SPA fallback (`/login`), `Jaunder` present in the home HTML, and 200 on
   `POST /api/current_user`.
5. **The check is unit-tested directly:** a fixture whose `web` set contains an
   unregistered fn yields `Some(detail)` naming that fn; a fixture whose
   registrar covers every `web` fn yields `None`; and a fixture using the
   unsupported `#[server(Name)]` positional-rename form triggers the hard error
   (so that guard cannot silently regress).
6. **The ADR draft exists** and records the chosen approach plus rejected A and
   B with reasons.

## Out of scope / separable

- A `linkme`/wrapper-macro auto-registration prototype (Approach B) — not
  pursued; recorded as rejected. Any future revisit is its own issue.
- No behavioral change to any server fn.
- No change to how the production server registers server fns (it uses the
  `#[server]` macro's own auto-registration; only the test path is hand-listed).

## Coordination with #358

#358 is editing `helpers::ensure_server_fns_registered()` concurrently (folding
the `media` trio). Expect to **rebase onto #358 before ship**; reconcile the
registrar additions so nothing is double-added. Once either PR merges, the gate
enforces completeness regardless of which PR contributed a given entry.
