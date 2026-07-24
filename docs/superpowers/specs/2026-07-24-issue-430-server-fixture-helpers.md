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
these behavior-preserving rules:

1. **Standalone seed** (a `make_user` whose `UserId` is used, no cookie for that
   user) → `SeedUser::new().seed(&state).await.user_id`.
2. **Seed + cookie for the same user, id not used elsewhere** →
   `create_user_and_session(&state).await.cookie()` (drop the now-unused id
   binding).
3. **Seed + cookie for the same user, id also used** →
   `let s = create_user_and_session(&state).await;` then `s.user_id` /
   `s.cookie()`.
4. **Cookie for a separately-seeded / pre-existing id** →
   `create_session_for(&state, id).await.cookie()`.

Rule 2 assumes the `make_user` and `cookie_for` calls are adjacent. In the
cross-author cases (`audiences.rs` `bob`), the seed and the cookie call are
**non-adjacent** — the seed precedes another author's setup, the cookie follows
it. For those, prefer keeping the seed in place via Rule 1
(`let bob = SeedUser::new().seed(&state).await.user_id;`) and taking the cookie
at its original site via Rule 4
(`create_session_for(&state, bob).await.cookie()`), rather than collapsing to
`create_user_and_session` at the cookie site (which would relocate the seed
later). Both are behavior-neutral — no assertion depends on user-id ordering —
but Rule 1 + Rule 4 preserves source locality; pick whichever reads cleaner per
site.

Each rule maps one alias call onto the canonical shared helper it already
wrapped; `create_user_and_session` = seed a fresh `SeedUser` + issue one
session, which is exactly `make_user` followed by `cookie_for` on that user. No
test semantics change (same seeding, same single session per user). The compiler
enforces correctness: a dropped id binding that is still referenced fails to
compile.

No new helper is added to `helpers/mod.rs` — the shared surface (`SeedUser`,
`create_session_for`, `create_user_and_session`, `SeededSession::cookie`)
already covers every case.

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
   call site references `make_user`/`cookie_for`; each uses `SeedUser`,
   `create_session_for`, or `create_user_and_session` directly.
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
