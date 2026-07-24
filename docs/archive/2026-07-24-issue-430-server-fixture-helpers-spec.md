# Spec — #430: converge duplicated server-test fixture helpers onto the shared API

- Issue: [#430](https://github.com/jaunder-org/jaunder/issues/430)
- Milestone: Test infrastructure & E2E
- Date: 2026-07-24

## Problem

Issue #430 (filed post-#358) targeted three copy-pasted server-test fixtures —
`create_session_cookie` (×3), `make_user` (×2), `cookie_for` (×2). The tracker
counts are "as of filing"; the code has since drifted (via #298, #429, #626,
#635) and the premise is largely obsolete:

- **`create_session_cookie` — gone entirely.** No definitions, no call sites.
  Its role is the shared `helpers::session_cookie(&RawToken)` +
  `SeededSession::cookie()`.
- **`make_user` / `cookie_for` — two byte-identical local copies each remain**,
  in `server/tests/web/audiences.rs` (`:13`, `:17`) and
  `server/tests/web/web_subscriptions.rs` (`:13`, `:17`).

These two survivors are not missing shared helpers — they are **thin local
aliases that wrap fixtures already shared** in `server/tests/helpers/mod.rs`:

- `make_user(state) -> UserId` is exactly
  `SeedUser::new().seed(state).await.user_id` — and both files already call
  `SeedUser::new().seed(&state).await` directly in other tests
  (`audiences.rs:381`, `web_subscriptions.rs:26,72,99,116`).
- `cookie_for(state, id) -> String` is exactly
  `create_session_for(state, id).await.cookie()` — and `create_session_for` is
  already imported at the top of both files.
- The `make_user` + `cookie_for` **pair on the same user** reproduces the shared
  `create_user_and_session(state)`, which returns a `SeededSession` carrying
  both `.user_id` and `.cookie()`.

So the residual duplication is best removed by **converging call sites onto the
existing shared API** and deleting the two local aliases — not by promoting the
aliases to `helpers/` (which would enshrine near-duplicates of
`create_user_and_session`).

## Decision

Delete the local `make_user` and `cookie_for` definitions from `audiences.rs`
and `web_subscriptions.rs`, and rewrite every call site using the shared
fixtures already in `server/tests/helpers/mod.rs` / `storage::test_support`, per
these two behavior-preserving rules keyed on whether the user needs a session:

1. **Authed user** (needs a cookie/session — its `.user_id` and/or `.username`
   may also be used) → `let u = create_user_and_session(&state).await;` then
   `u.cookie()` / `u.user_id` / `u.username` (or `create_user_and_session(&state)
   .await.cookie()` inline when only the cookie is needed). This is the
   purpose-built "authed user" fixture: a fresh `SeedUser` seed + one session,
   returning a `SeededSession` with all three — exactly `make_user` followed by
   `cookie_for` on that user.
2. **Session-less target** (a user referenced only by `.user_id`/`.username`,
   never logged in — subscription targets like `subscriber`/`alice`) →
   `SeedUser::new().seed(&state).await.user_id` (or the full seed result when the
   `.username` is asserted).

`create_session_for` is **not** used: every user that needs a cookie is freshly
seeded for that purpose, so `create_user_and_session` (seed + session in one) is
the right fixture and avoids `create_session_for`'s redundant `get_user`
round-trip. The cross-author `bob` users are only ever used for their cookie, so
they collapse to `create_user_and_session(&state).await.cookie()` at the cookie
site (dropping the separate seed binding); relocating that seed a few lines later
is behavior-neutral (no assertion depends on user-id ordering).

No test semantics change (same seeding, same single session per user). The
compiler enforces correctness: a bare `SeededSession` used where a `UserId` is
expected fails to compile, forcing the `.user_id` projection.

No new helper is added to `helpers/mod.rs` — the shared surface (`SeedUser`,
`create_user_and_session`, `SeededSession`) already covers every case.

## Scope

**In scope:** `server/tests/web/audiences.rs` and
`server/tests/web/web_subscriptions.rs` (the only definers/callers of the two
aliases), plus a correcting update to the #430 issue body noting
`create_session_cookie` was already retired and the work is now the converge.

**Out of scope / untouched:**

- The shared `helpers/mod.rs` (no additions, no signature changes).
- The near-variants (`user_with_cookie`, `author_with_cookie`,
  `login_and_state`, `create_user_with_verified_email`) — they already delegate
  to shared fixtures; the issue says leave near-variants local unless they
  collapse cleanly, and these are already on the shared surface.
- Any product (non-test) code.

## Acceptance criteria

1. **Both local aliases are gone.** No `fn make_user` or `fn cookie_for` remains
   anywhere under `server/tests` (`rg 'fn make_user|fn cookie_for'` → no
   matches).
2. **Every former call site uses a shared fixture** per the rules above — no
   call site references `make_user`/`cookie_for`; each uses `SeedUser` or
   `create_user_and_session` directly.
3. **No new shared helper** was added to `helpers/mod.rs` (diff there is empty).
4. **No product code changed** — the diff is confined to the two test files (and
   the issue body).
5. **Dual-backend green.** `cargo xtask check` passes — the converged
   `audiences.rs` and `web_subscriptions.rs` tests pass on both SQLite and
   Postgres (they are `#[case] backend` rstest cases) — with no new lint
   suppressions.

## Verification

`cargo xtask check` is the gate: it runs the server integration suite under
coverage on both backends, so a green run proves the converted tests still pass
identically. This branch touches only server **test** files (no product, web, or
e2e surface), so the ship gate is `cargo xtask validate --no-e2e`.
