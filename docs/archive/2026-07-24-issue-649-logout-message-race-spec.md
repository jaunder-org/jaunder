# Spec — #649 auth: LogoutPage "You have been logged out." is never seen (redirect races it)

- Issue: [#649](https://github.com/jaunder-org/jaunder/issues/649) (spun out of
  #643's SSR-remnant sweep)
- Milestone: Web: canonical Leptos CSR convergence (#11)
- Date: 2026-07-24
- Vertical: `auth` (`web/src/auth/component.rs`)

## Problem

`LogoutPage` (`web/src/auth/component.rs:92–128`) renders a success message that
a user can never actually see:

```rust
Ok(()) => view! { <p>"You have been logged out."</p> }.into_any(),
```

The `logout` server fn (`auth/api.rs:115`) clears the cookie, calls
`leptos_axum::redirect("/")`, and returns `Ok(())`. `leptos_router`'s
**built-in** redirect handling — the same-origin `use_navigate` hook the
`<Router>` registers in Leptos's first-caller-wins redirect `OnceLock`, which is
the default since #591 removed the SSR-era full-reload override
(`pages/mod.rs:69`) — turns that server-fn redirect into a client-side
**pushState** navigation to `/`. It fires on the **same** action resolution that
flips `logout_action.value()` to `Some(Ok(()))`. So the `Ok(())` render branch
and the navigation-away happen together — the navigation unmounts `LogoutPage`
before its success message can paint. (This is stock Leptos CSR behavior, not
jaunder machinery — nothing to change there; the page must adapt to it.)

### Diagnosis (grounded)

- **Code:** `logout()` always redirects on success; the second `Effect` already
  keys `clear_session()` off `Some(Ok(()))` — the same resolution that
  redirects.
- **Existing e2e:** all three logout tests in `auth.spec.ts`
  (`"logout page logs out"`, `"logout navigates client-side…"`,
  `"sidebar reverts to signed-out state…"`) click the sidebar link and
  `waitForURL("/")`. **None navigate to `/logout` to read its content; none
  assert "You have been logged out."** The suite already treats `/logout` as a
  pure redirect trigger — corroborating that the message is unreachable.

### What a user actually sees (to confirm in-browser during iterate)

- **"Logging out…"** — rendered on mount while `value()` is `None`; visible for
  the logout round-trip. _Perceivable, honest._
- **"You have been logged out."** (`Ok(())`) — paints only once
  `value() == Some(Ok(()))`, which is when the redirect navigates away. _Not
  perceivable._
- **`Err` branch** — logout failure returns `Err` with **no** redirect, so the
  error message would paint. _Perceivable only on failure._

## Goal

Make `LogoutPage` honest: it contains no render branch unreachable under the
#591 redirect.

## Scope — in

1. **Remove the dead `Ok(())` branch.** Restructure the resolution block to
   render only on failure (the sole case with no redirect):

   ```rust
   {move || {
       // On success the router's redirect->pushState navigates to "/" on the same
       // resolution that fills `value()`, so only a logout *failure* (no redirect) paints.
       logout_action.value().get().and_then(Result::err).map(|e| {
           view! { <p class="error">{e.to_string()}</p> }
       })
   }}
   ```

2. **Keep the "Logging out…" placeholder**, with a comment stating the real
   (CSR) reason: it is the honest transient a user sees during the logout
   round-trip before the redirect. (The issue invites reconsidering it; keeping
   it avoids a blank page during the round-trip and is genuinely perceivable —
   so keep, don't strip.)

3. **Pin the honest behavior with e2e.** Extend `auth.spec.ts`: navigate to
   `/logout` while authenticated (or click the sidebar link) and assert the flow
   ends at `/` signed-out, and that **"You have been logged out." is never the
   visible end state**. Document in the PR what the user sees (the "Logging
   out…" transient → redirect home).

## Scope — out

- The `logout` server fn, `leptos_router`'s redirect→pushState navigation (stock
  Leptos, the default since #591 removed the full-reload override), and
  `clear_session()` — unchanged.
- The second `Effect` (`clear_session()` on `Some(Ok(()))`) — unchanged; still
  needed.
- No other auth-page surface.

## Acceptance (from the issue)

- The rendered `LogoutPage` contains no branch unreachable under the #591
  redirect (the `Ok(())` "logged out" branch is gone).
- Behavior verified in a browser; what the user sees on logout is documented in
  the PR.
- The `auth` e2e spec is green (incl. the new pinning assertion).
- `cargo xtask validate` green.

## Risks

- Tiny, contained change in one wasm-only component. The only judgment call —
  keep vs strip the "Logging out…" placeholder — is resolved in favor of keeping
  it (perceivable + honest). If the in-browser drive shows the placeholder never
  paints either (redirect too fast to perceive), revisit stripping it; but a
  blank logout page is worse UX, so default to keep.
