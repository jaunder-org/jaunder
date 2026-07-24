# Plan — #430: converge server-test fixture helpers onto the shared API

Spec:
[`2026-07-24-issue-430-server-fixture-helpers.md`](../specs/2026-07-24-issue-430-server-fixture-helpers.md)

**For agentic workers:** execute with `jaunder-iterate` (delegate a task via
`jaunder-dispatch` if useful). Tick checkboxes in real time.

---

## Review header

**Goal.** Delete the two duplicated local test aliases `make_user` and
`cookie_for` from `server/tests/web/audiences.rs` and
`server/tests/web/web_subscriptions.rs`, routing every call site onto the shared
fixtures (`SeedUser`, `create_session_for`, `create_user_and_session`) per the
spec's 4 conversion rules. No new shared helper; no product code.

**Scope.** _In:_ the two named test files + the #430 issue body. _Out:_
`helpers/mod.rs` (untouched), the near-variants, any product code.

**Tasks.**

- [x] 1. Correct the #430 issue body (record the drift + converge decision).
- [x] 2. Convert `audiences.rs` — delete both aliases; convert all 17
     `make_user` + 10 `cookie_for` sites per the rules; fix imports.
- [x] 3. Convert `web_subscriptions.rs` — delete both aliases; convert its
     sites; fix imports.
- [x] 4. Verify — `cargo xtask check` dual-backend green; acceptance-criteria
     sweep.

**Key risks / decisions.**

- Behavior preservation hinges on `create_user_and_session` ≡ `make_user` +
  `cookie_for` on one user (spec §Decision; review-confirmed: one user, one
  session, no id-ordering/literal-username assertions). Compiler enforces the
  rest — a dropped id binding still referenced fails to compile.
- Non-adjacent `bob` cases use Rule 1 + Rule 4 (keep the seed in place), not the
  `create_user_and_session` collapse — avoids relocating a seed past another
  author's setup.
- Test-only change → the gate is `cargo xtask check` (runs the server suite on
  both backends via coverage). Two file commits (one per file) for clean
  history.
- No separable follow-on concerns → no issue-filing task beyond the body update.

---

## Global constraints

- Touch only the two test files (tasks 2–3) + the issue body (task 1). Zero diff
  to `helpers/mod.rs`; no product code.
- No new `#[allow]`/`#[expect]`; no new shared helper.
- Commit via `jaunder-commit`: run `cargo xtask check` first (it runs the
  dual-backend server suite, so it verifies the converted tests pass) so the
  pre-commit hook passes clean. No `Co-Authored-By` trailer.
- After deleting the aliases, prune any now-unused imports (`UserId` may go
  unused; add `create_user_and_session` to the `use crate::helpers::{…}` line).
  Let clippy/fmt (in `check`) flag leftovers — finish the sweep, don't defer.

---

## Task 1 — Correct the #430 issue body

**No code.** Update the issue description (via `jaunder-issues` /
`gh issue edit`) to record that `create_session_cookie` was already retired
(#298/#429/#626/#635), that only `make_user`/`cookie_for` remained as local
aliases, and that the chosen approach is convergence onto the shared API (delete
the aliases), not lifting them into `helpers/`. Keep the original tasks list but
mark the reality.

**Done when:** the #430 body reflects the actual scope and the converge
decision.

---

## Task 2 — Convert `audiences.rs`

**Files:** `server/tests/web/audiences.rs`.

**Change.** Delete `fn make_user` (`:13-15`) and `fn cookie_for` (`:17-19`).
Convert **every** call site by mechanically applying the spec's 4 rules — the
per-site rule is determined by whether the seeded id and/or a cookie is used
downstream. The counts are 17 `make_user` + 10 `cookie_for`; the T4 `rg` sweep
guarantees none is missed. Representative sites per rule (not exhaustive):

- **Rule 1 — standalone seed** (id used, no cookie for that user): the
  `subscriber`/`alice`/`bob` target seeds, e.g. `:183`, `:227`, `:318`
  (`alice`), `:320` (`subscriber`), `:447`, `:449`, `:498`, `:499` →
  `SeedUser::new().seed(&state).await.user_id`.
- **Rule 3 — author seed + cookie, id also used** (`:31-32` [`author` used at
  `:82`], `:123-124`, `:151-152`, `:182-184`, `:226-228`, `:380-382`) → bind
  `let author = create_user_and_session(&state).await;` then use
  `author.user_id` where the id was consumed and `author.cookie()` where the
  cookie was.
- **Rule 2 — seed + cookie, id NOT used elsewhere** (e.g.
  `duplicate_audience_name` `author` at `:91-92`, used only for the cookie) →
  `let cookie = create_user_and_session(&state).await.cookie();` (drop the id
  binding).
- **Rule 4 — non-adjacent `bob`** (seed at `:319`/`:448`/`:499`, cookie at
  `:338`/`:462`/`:505`, with another author's setup in between): keep
  `let bob = SeedUser::new().seed(&state).await.user_id;` in place (Rule 1) and
  `let bob_cookie = create_session_for(&state, bob).await.cookie();` at the
  cookie site.

Verify each converted site against how the binding is used (the compiler
backstops a wrong choice — a dropped id still referenced fails to build). Update
imports: add `create_user_and_session` to `use crate::helpers::{…}`; drop
`common::ids::UserId` (unused once the alias signatures are gone) and any other
now-unused import.

**Check:** `cargo xtask check` — dual-backend server suite green (the converted
`audiences.rs` tests actually run), fmt/clippy clean (expected PASS).

**Commit:**
`test(server): converge audiences.rs fixtures onto shared helpers (#430)` via
`jaunder-commit`.

**Done when:** no `make_user`/`cookie_for` in `audiences.rs`; every site on a
shared fixture; `cargo xtask check` green.

---

## Task 3 — Convert `web_subscriptions.rs`

**Files:** `server/tests/web/web_subscriptions.rs`.

**Change.** Delete `fn make_user` (`:13-15`) and `fn cookie_for` (`:17-19`).
Convert its sites:

- `subscribe_then_unsubscribe` (`:27-28`, `subscriber` id used) → Rule 3:
  `let subscriber = create_user_and_session(&state).await;` use
  `subscriber.user_id` / `subscriber.cookie()`. (`author` there is already a
  direct `SeedUser::new().seed(&state).await` — leave it.)
- `self_subscribe_is_rejected` (`:72-73`, `me` seeded directly) → Rule 4:
  `create_session_for(&state, me.user_id).await.cookie()`.
- `is_subscribed_to_reports_state` (`:117-118`, `subscriber` feeds only the
  cookie) → Rule 2:
  `let cookie = create_user_and_session(&state).await.cookie();` (confirm
  `subscriber` isn't referenced after `:118`; else Rule 3).

Update imports as in Task 2.

**Check:** `cargo xtask check` — expected PASS.

**Commit:**
`test(server): converge web_subscriptions.rs fixtures onto shared helpers (#430)`
via `jaunder-commit`.

**Done when:** no `make_user`/`cookie_for` in `web_subscriptions.rs`; every site
on a shared fixture; `cargo xtask check` green.

---

## Task 4 — Verify acceptance

**No file change.** Confirm the spec's acceptance criteria:

- `rg 'fn make_user|fn cookie_for' server/tests` → no matches (AC1).
- `rg 'make_user|cookie_for' server/tests` → no matches (AC2 — no residual
  calls).
- `git diff wt-base-issue-430...HEAD -- server/tests/helpers/mod.rs` → empty
  (AC3).
- Diff confined to the two test files + issue body (AC4).
- `cargo xtask check` green, no new suppressions (AC5) — already proven by tasks
  2–3's commit gates; re-confirm on the final tip.

**Done when:** all five criteria hold.

---

## Self-review

- Every spec AC maps to a task: AC1/AC2 → T2+T3 (+T4 sweep), AC3/AC4 → T2+T3
  scope
  - T4, AC5 → T2+T3 commit gates + T4. Issue correction → T1.
- No task touches `helpers/mod.rs` or product code. No separable concern to
  file.
- Each task is independently verifiable (per-file `rg` + `cargo xtask check`).
