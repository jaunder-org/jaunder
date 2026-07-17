# Plan — Issue #487: `EditPostPage` route sentinel → `Option<PostId>`

Spec:
[`2026-07-17-issue-487-editpost-option-route.md`](../specs/2026-07-17-issue-487-editpost-option-route.md)

## Review header

**Goal.** Remove the `PostId::from(-1)` route sentinel in `EditPostPage`; derive
`Option<PostId>` and short-circuit absence to the same client-side "Post not
found" without a wasted lookup. See spec § Approach.

**Scope.**

- In: `web/src/pages/posts.rs` (`EditPostPage` only); one new e2e in
  `end2end/tests/posts.spec.ts`.
- Out: server fns / wire / schema (unchanged, still take `PostId`); any other
  `EditPostPage` behavior.

**Tasks.**

1. Rewrite `post_id_param` → `Option<PostId>` and the two resource fetchers to
   short-circuit `None` → `WebError::not_found("Post")`.
2. Add an e2e regression guard: invalid edit route renders "Post not found".
3. Gate: `cargo xtask validate --no-e2e`; run the edit e2e.

**Key risks / decisions.**

- Message parity hinges on `WebError::not_found("Post")` equalling the server's
  not-found (`NotFound { "Post not found" }`) — verified identical; the existing
  `<Suspense>` `Err` arm renders it, so no view-arm change.
- `EditPostPage` is `#[component]`, which is coverage-exempt (ADR-0050), so the
  new `None` match arms carry no host-coverage obligation; the e2e is the
  behavioral guard.
- The new e2e passes **before and after** (today's `-1` lookup also yields "Post
  not found") — it is a parity/regression guard, not a red-green driver. Called
  out so its green-on-baseline is not mistaken for a no-op.

**For agentic workers.** Execute with `jaunder-iterate` (delegate a task via
`jaunder-dispatch` if useful). Small enough to run inline.

## Global constraints

- Rust; `web` is a dual-target crate (host + wasm). No `Co-Authored-By` trailer.
- No new `-1`/magic-number `PostId` sentinel may remain in `web/`.

---

## Task 1 — `post_id_param` → `Option<PostId>`, fetchers short-circuit `None`

**Files.** `web/src/pages/posts.rs` (`EditPostPage`, around lines 664–672).

**Change.** Drop the `.unwrap_or(PostId::from(-1))`; wrap each server fn in a
fetcher closure that only dispatches for `Some`:

```rust
let post_id_param = move || {
    params
        .get()
        .get("post_id")
        .and_then(|v| v.parse::<PostId>().ok())
};
let post = crate::server_resource(post_id_param, |maybe_id| async move {
    match maybe_id {
        Some(id) => get_post_preview(id).await,
        None => Err(WebError::not_found("Post")),
    }
});
let current_audience = crate::server_resource(post_id_param, |maybe_id| async move {
    match maybe_id {
        Some(id) => post_audience_selection(id).await,
        None => Err(WebError::not_found("Post")),
    }
});
```

Notes:

- `WebError` is already in scope in this module (used at the file's error arms);
  add the import if not (`crate::error::WebError`). Confirm during edit.
- `post_id_param` is `Copy` (a plain `move ||` closure over `params`), so
  passing it to both `server_resource` calls stays valid — same as today.
- No change to the `<Suspense>` body: the `Ok`/`Err` arms already handle both
  outcomes; `None` now resolves through the existing
  `Err(err) => <p class="error">` arm.

**Check.** `cargo check -p web --all-features --all-targets` compiles clean.

**Commit.** Run `cargo xtask check` first (fmt + clippy + Nix), then commit
(`jaunder-commit`). Message e.g.
`refactor(web): model EditPostPage route id as Option, drop -1 sentinel (#487)`.

## Task 2 — e2e regression guard for the invalid edit route

**Files.** `end2end/tests/posts.spec.ts` (new `test(...)` near the other edit
tests).

**Test.** Navigate to an edit route whose `post_id` does not resolve to a
viewable post and assert the not-found error surfaces:

```ts
test("editing a nonexistent post shows not-found", async ({
  page /* + auth fixture as siblings use */,
}) => {
  // authenticate the same way the sibling edit tests do (reuse their fixture/helper)
  await goto(page, `/posts/999999999/edit`);
  await expect(page.locator(SEL.error)).toContainText("Post not found");
});
```

Match the surrounding tests' auth setup and helpers (`goto`, `SEL.error`)
exactly — mirror the "authenticated user can edit a draft post" test's
scaffolding. Use a large numeric id (parses as `PostId`, no such post → server
not-found) so the test also exercises the `Some(id)` not-found path; the
`None`/unparseable path renders the identical message via the same arm.

**Check.** Run just this spec against a host build (see Task 3 command).
Expected PASS both on baseline and after Task 1 (parity guard).

**Commit.** `cargo xtask check` clean, then commit
(`test(e2e): guard EditPostPage not-found route (#487)`).

## Task 3 — full gate

- `cargo xtask validate --no-e2e` — clean (fmt, clippy, coverage).
- Run the edit-page e2e (at least the new test + "authenticated user can edit a
  draft post") on one backend/browser combo to confirm parity, e.g.
  `cargo xtask e2e sqlite chromium` or the project's targeted e2e invocation.
- Confirm `rg 'PostId::from\(-1\)|unwrap_or\(PostId' web/` returns nothing.
