# Plan — #649 auth: LogoutPage "You have been logged out." is never seen

Spec:
[`2026-07-24-issue-649-logout-message-race.md`](../specs/2026-07-24-issue-649-logout-message-race.md).
For agentic workers: `jaunder-iterate` (delegate a task via `jaunder-dispatch`
if useful).

## Review header

**Goal.** Make `LogoutPage` honest: delete the `Ok(())` → "You have been logged
out." render branch (unreachable — `leptos_router`'s redirect→pushState
navigates to `/` on the same resolution), keeping only the perceivable "Logging
out…" transient and the `Err` branch. Verify in-browser and document.

**Scope**

- In: `web/src/auth/component.rs` `LogoutPage` render only; a `#649` intent
  comment on the existing logout e2e; browser verification + PR documentation.
- Out (all working, per spec): the `logout` server fn, `leptos_router`'s
  redirect→pushState, `clear_session()`, and the `clear_session`-on-`Ok`
  `Effect`.

**Tasks**

1. `LogoutPage`: drop the dead `Ok(())` branch → error-only resolution block via
   `.and_then(Result::err)`; add CSR-reason comments. Pin the observable
   behavior with a `#649` comment on the existing `"logout page logs out"` e2e
   (no new racy assertion).
2. Verify in a browser (`cargo xtask e2e-local auth`) and record what the user
   sees for the PR; then full `cargo xtask validate`.

**Key risks / decisions**

- **No new e2e test.** The three existing logout tests in `auth.spec.ts` already
  drive `/logout` → redirect `/` → signed-out; they're the regression pin. A
  `goto('/logout')` test or a "message never appears" assertion would be
  **flaky** — the redirect-wins-race is the very thing those tests' "avoid
  Firefox navigation abort races" comment guards against. Documented here so the
  absence is a decision, not an oversight.
- **No host unit test.** The change is a wasm-only render simplification (a
  2-arm `match` → a std `Option::and_then(Result::err)` map); there's no domain
  logic worth extracting to a host-tested helper. Verification is code review +
  existing e2e + the browser drive.

## Global constraints

- Rust; `web` crate; the file is wasm-only
  (`#[cfg(target_arch = "wasm32")] mod component;`), so run **wasm-clippy**
  before committing (host `cargo check` won't compile it). No `Co-Authored-By`
  trailer.
- Pre-commit hook runs full `cargo xtask check`; run it first so it passes clean
  (`jaunder-commit`). leptosfmt may reflow the `view!`; re-stage if so.

---

## Task 1 — Make `LogoutPage` honest + pin it

**File:** `web/src/auth/component.rs` (`LogoutPage`, ~lines 110–128).

Replace the placeholder + resolution block:

```rust
        // The honest transient a user sees during the logout round-trip. On success
        // leptos_router's redirect->pushState navigates to "/" on the same resolution
        // that fills the action value (the SSR-era full reload is gone, #591), so this
        // placeholder and the failure branch below are all that can actually paint.
        <p class="j-loading">"Logging out\u{2026}"</p>
        {move || {
            // Only a logout *failure* has no redirect, so it is the sole case that paints
            // here; on success the navigation unmounts this page before it could (#649).
            logout_action
                .value()
                .get()
                .and_then(Result::err)
                .map(|e| view! { <p class="error">{e.to_string()}</p> })
        }}
```

Notes:

- `logout_action.value().get()` is `Option<Result<(), WebError>>`;
  `.and_then(Result::err)` yields `Option<WebError>`; the `.map(...)` yields
  `Option<_: IntoView>` (Leptos renders `None` as nothing). The two
  `.into_any()` calls drop away — a single view type now.
- The `Ok(())` arm and its "You have been logged out." string are deleted
  entirely.
- Leave the two `Effect`s (dispatch on mount; `clear_session()` on
  `Some(Ok(()))`) and the `Topbar` untouched.

Then add a `#649` intent comment to the existing e2e in
`end2end/tests/auth.spec.ts` (`"logout page logs out"`, ~line 142), so a future
reader knows `/logout` is a pure redirect trigger and the success message was
intentionally removed:

```ts
// #649: /logout is a pure redirect trigger — leptos_router's redirect->pushState
// navigates to "/" on the same resolution that would render a success message, so
// there is no perceivable "You have been logged out." page. This test pins that the
// flow ends signed-out at "/"; the LogoutPage render carries no success branch.
```

**Verify (compile-driven, no unit test):**

- `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations`
  — expected clean (this file only compiles on wasm; catches e.g. an unused
  `WebError` import if it became orphaned — verify it's still used elsewhere in
  the file, it is).
- `cargo xtask check` — fmt + host clippy + coverage; expected clean (the
  wasm-only render change leaves host coverage unmoved).

**Commit** (after `cargo xtask check` passes clean):
`fix(web/auth): drop unreachable LogoutPage success branch (#649)`.

---

## Task 2 — Verify in a browser + full gate

- `cargo xtask e2e-local auth` — drives the real logout flow (the three existing
  logout tests). Expected: green. This is the browser verification the issue
  requires; from the run, record for the PR body **what the user actually sees
  on logout** (a brief "Logging out…", then the redirect to `/` with the sidebar
  reverted to signed-out; the "You have been logged out." message never
  appears).
- `cargo xtask validate` (foreground, `timeout: 600000`) — the full local gate
  incl. the e2e matrix. Expected: green.
- `git status --porcelain` after green — confirm no stray leptosfmt reflow left
  unstaged.

No follow-up issues: scope is a single dead-branch removal. (The `pages/mod.rs`
SSR-era rationale comment noted during the interview is a separate doc nit —
file only if the maintainer asks.)
