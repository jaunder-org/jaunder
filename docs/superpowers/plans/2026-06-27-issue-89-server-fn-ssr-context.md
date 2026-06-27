# SSR Server-Fn Context Across Awaits — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Leptos context reliable across `.await` points inside `#[server]` functions during SSR, fixing the rare `expect_context::<UserStorage>` panic (#89), and retire the now-redundant per-fn workaround.

**Architecture:** `server_boundary` (the `boundary!` wrapper every server fn routes through) runs the body inside `reactive_graph`'s `ScopedFuture`, which holds a strong owner ref and re-applies it on each poll — keeping the owner (and its context map) alive across awaits. Guarded on `Owner::current().is_some()` so it never captures an empty default owner. With context now guaranteed, the defensive `use_context(...).ok_or_else(...)` workarounds revert to the uniform `expect_context` DI (ADR-0016).

**Tech Stack:** Rust, Leptos 0.8.19 / reactive_graph 0.2.14, `web` crate (`ssr` feature for server-fn bodies).

## Global Constraints

- Per-commit gate: `cargo xtask validate --no-e2e` (run via context-mode with `cd <worktree> &&`, or Bash). Full `cargo xtask validate` (with e2e) only at ship.
- NO `Co-Authored-By` trailers in commits. One clean commit per task.
- `server_boundary` is `#[cfg(feature = "ssr")]`; reference `Owner`/`ScopedFuture` by fully-qualified path inline (`leptos::reactive::owner::Owner`, `leptos::reactive::computed::ScopedFuture`) — do NOT add top-level `use` (would be unused on non-ssr builds).
- Use `ScopedFuture::new_untracked` (not `new`): server-fn bodies do no reactive reads; keep the owner, clear the observer.
- The `Owner::current().is_some()` guard is mandatory (an empty owner loses context deterministically — proven by `owner_lifetime::scoped_future_with_no_current_owner_sees_empty_context`).
- Do NOT reorder any `#[server]` body's statements. Do NOT touch `require_auth` (its `Parts`→extensions path is a separate mechanism).
- `expect_context::<T>()` is the DI per ADR-0016. A missing context is a bug, not a recoverable error.

## Pre-existing artifacts (already written during design; commit with this work)

- `docs/superpowers/specs/2026-06-27-issue-89-server-fn-ssr-context.md` — the spec.
- `docs/adr/0016-dependency-injection-and-appstate.md` — addendum "SSR context lifetime across `await`s (#89)".
- `web/src/error.rs` — module `owner_lifetime` with four deterministic, passing tests (bug repro, `ScopedFuture` fix, read-before-await pattern, empty-owner trap). These stay; they are the mechanism's regression suite. Committed **with Task 1** (alongside the fix), not here.

Commit the spec + plan + ADR addendum first, before Task 1:

```bash
git add docs/superpowers/specs/2026-06-27-issue-89-server-fn-ssr-context.md docs/superpowers/plans/2026-06-27-issue-89-server-fn-ssr-context.md docs/adr/0016-dependency-injection-and-appstate.md
git commit -m "docs(issue-89): add spec, plan, and ADR-0016 SSR-context addendum"
```

Separable concern (#93, e2e capture-always + zero-panic gate) is already filed (P1) — no filing task here.

---

### Task 1: Owner-scope the server-fn boundary

**Files:**
- Modify: `web/src/error.rs` — `server_boundary` (line ~371) and its test module.

**Interfaces:**
- Consumes: `leptos::reactive::owner::Owner::current() -> Option<Owner>`; `leptos::reactive::computed::ScopedFuture::new_untracked(fut) -> ScopedFuture<Fut>` (impls `Future<Output = Fut::Output>`, holds a strong `Owner`, re-applies it per poll).
- Produces: unchanged public signature `server_boundary(server_fn: &'static str, future: impl Future<Output = InternalResult<T>>) -> WebResult<T>`.

- [ ] **Step 1: Write the failing test.** In `web/src/error.rs`, inside the existing `mod owner_lifetime` (it already has `Marker`, `YieldOnce`, and `noop_waker`), add:

```rust
    /// The actual fix: `server_boundary` must keep context alive across an await,
    /// even when the caller's owner ref is dropped mid-suspension.
    #[cfg(feature = "ssr")]
    #[test]
    fn server_boundary_keeps_context_alive_across_await() {
        let owner = Owner::new();
        owner.set();
        provide_context(Marker(7));

        let mut fut = Box::pin(crate::error::server_boundary("spike_test", async {
            let _pre = use_context::<Marker>();
            YieldOnce(false).await;
            Ok::<Option<Marker>, crate::error::InternalError>(use_context::<Marker>())
        }));

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        drop(owner);
        let result = match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("server_boundary future did not complete"),
        };
        assert_eq!(
            result,
            Ok(Some(Marker(7))),
            "server_boundary must keep context alive across the await"
        );
    }
```

- [ ] **Step 2: Run it; verify it FAILS.**

Run: `cargo test -p web --features ssr owner_lifetime::server_boundary_keeps_context_alive_across_await`
Expected: FAIL — `assertion left == right` with `left: Ok(None)` (current `server_boundary` does not preserve the owner across the await).

- [ ] **Step 3: Implement the guarded `ScopedFuture` wrap.** Replace the body of `server_boundary` (`match future.await { ... }`) with:

```rust
    // #89: a server-fn body that reads Leptos context after an `.await` can panic
    // during SSR. The reactive "current owner" is a *weak* thread-local; if its last
    // strong ref is dropped while the future is suspended at an await, the owner's
    // context map is freed and the post-await `expect_context` finds nothing.
    // `ScopedFuture` captures a *strong* owner ref and re-applies it on every poll,
    // keeping context alive across awaits. Guard on a current owner: `ScopedFuture::new`
    // captures `Owner::current().unwrap_or_default()`, so wrapping with no owner would
    // capture an empty owner and lose context deterministically. See ADR-0016 addendum.
    let outcome = if leptos::reactive::owner::Owner::current().is_some() {
        leptos::reactive::computed::ScopedFuture::new_untracked(future).await
    } else {
        future.await
    };
    match outcome {
        Ok(value) => Ok(value),
        Err(error) => {
            log_boundary_failure(server_fn, &error);
            common::metrics::error(error.kind.as_metric_str(), error.class.as_metric_str());
            Err(error.into_public())
        }
    }
```

- [ ] **Step 4: Run the new test and the whole `owner_lifetime` suite; verify PASS.**

Run: `cargo test -p web --features ssr owner_lifetime`
Expected: PASS — all five tests (the four pre-existing + the new `server_boundary_*`).

- [ ] **Step 5: Gate and commit.**

Run (via context-mode, `cd` into the worktree, bare): `cargo xtask validate --no-e2e`
Expected: green.

```bash
git add web/src/error.rs
git commit -m "fix(web): owner-scope server_boundary so Leptos context survives awaits (#89)"
```

---

### Task 2: Retire the read-before-await workaround

**Files:**
- Modify: `web/src/audiences/mod.rs` (module `//!` note; `list_my_audiences`, `create_audience`)
- Modify: `web/src/subscriptions/mod.rs` (module `//!` note; `subscribe_to`, the `SubscribeButton` state server fn)
- Modify: `web/src/posts/mod.rs` (`default_audience_selection`, `post_audience_selection`, and the inline convention comments)

**Interfaces:**
- Consumes: `expect_context::<Arc<dyn FooStorage>>()` (panics if absent — now guaranteed present by Task 1).
- Produces: no signature changes; only the in-body storage lookup and comments change.

- [ ] **Step 1: Convert each defensive lookup to `expect_context`.** For every site currently of the form:

```rust
let foo = use_context::<Arc<dyn FooStorage>>()
    .ok_or_else(|| InternalError::server_message("FooStorage not in context"))?;
```

replace it with:

```rust
let foo = expect_context::<Arc<dyn FooStorage>>();
```

Apply to all six sites: `list_my_audiences` and `create_audience` (`AudienceStorage`) in `audiences/mod.rs`; `subscribe_to` and the `SubscribeButton` state fn (`SubscriptionStorage`) in `subscriptions/mod.rs`; `default_audience_selection` and `post_audience_selection` (`PostStorage`/`AudienceStorage`) in `posts/mod.rs`. Confirm the exact set first:

Run: `rg -n 'use_context::<Arc<dyn .*Storage>>\(\)\s*\n?\s*\.ok_or_else' -U web/src`
Expected: exactly the six sites above; convert each.

- [ ] **Step 2: Remove the now-false convention comments.** Delete the module-level `//!` SSR-context-loss notes at the top of `audiences/mod.rs` and `subscriptions/mod.rs`, and the per-fn inline comments in `posts/mod.rs`, `audiences/mod.rs` (`create_audience`), and `subscriptions/mod.rs` (`subscribe_to`, the state fn) that explain reading context "before the await". Where a comment is still useful (e.g. on `get_registration_policy`), rewrite it to reference the guarantee, e.g.:

```rust
// Storage handles come from Leptos context (ADR-0016). server_boundary owner-scopes
// the body so this resolves regardless of await ordering (ADR-0016 addendum, #89).
```

- [ ] **Step 3: Verify no convention residue and no dangling imports.**

Run: `rg -n 'before .*await|task-local|context first|after an await|not in context' web/src` (exclude `error.rs`'s `owner_lifetime` doc/tests, which legitimately describe the mechanism)
Expected: no remaining "read context before await" workaround comments in `audiences`/`subscriptions`/`posts`.

Run: `cargo xtask check --no-test` (clippy will flag any now-unused `InternalError`/`use_context` imports left behind by Step 1 — remove them).
Expected: green.

- [ ] **Step 4: Gate and commit.**

Run (via context-mode, `cd` into the worktree, bare): `cargo xtask validate --no-e2e`
Expected: green.

```bash
git add web/src/audiences/mod.rs web/src/subscriptions/mod.rs web/src/posts/mod.rs
git commit -m "refactor(web): retire read-before-await workaround now that context survives awaits (#89)"
```

---

## Acceptance (verified at ship, `jaunder-ship`)

- `cargo xtask validate` (full, with e2e) green; the captured e2e VM log shows **zero**
  `expect_context::<…UserStorage>` panics (grep the diagnostics, or rely on #93 once it lands).
- All five `owner_lifetime` tests green.
- `git grep` shows the "read context before await" convention is gone from `web/src` (outside the
  `owner_lifetime` mechanism docs).
