# Spec — Issue #487: replace the `PostId::from(-1)` route sentinel in `EditPostPage` with `Option`

## Problem

`EditPostPage` (`web/src/pages/posts.rs`) derives its target from the `post_id`
route param and, when the param is missing or unparseable, falls back to a
`PostId::from(-1)` sentinel:

```rust
let post_id_param = move || {
    params
        .get()
        .get("post_id")
        .and_then(|v| v.parse::<PostId>().ok())
        .unwrap_or(PostId::from(-1))
};
let post = crate::server_resource(post_id_param, get_post_preview);
let current_audience = crate::server_resource(post_id_param, post_audience_selection);
```

`-1` is a guaranteed-nonexistent id minted **inside the typed domain** — the
exact bare-`i64` smell the `PostId` newtype (#472) exists to eliminate, just
relocated into the newtype. Today the `-1` reaches the server, which does a real
lookup that returns `WebError::NotFound { message: "Post not found" }`; the
`<Suspense>` `Err` arm renders `<p class="error">Post not found</p>`. So absence
costs a wasted round-trip that only ever yields not-found.

## Approach

Model absence honestly. Make the derived value `Option<PostId>` (`None` when the
param is missing/invalid), and have each resource's fetcher **short-circuit
`None` client-side** to the same not-found error — no lookup dispatched for a
nonexistent id.

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

- `server_resource`'s source type becomes `Option<PostId>`
  (`PartialEq + Clone + Send + Sync` — all satisfied by `PostId`).
- The server fns (`get_post_preview`, `post_audience_selection`) are
  **unchanged** — they still take `PostId`. The `Option` is unwrapped in the
  fetcher; the server is called only for `Some`.
- `WebError::not_found("Post")` yields `NotFound { message: "Post not found" }`
  — byte-identical to today's server response, so the `<Suspense>` `Err` arm
  renders the same message. `WebError::not_found` is not `cfg(server)`-gated, so
  it is constructible in the wasm/client build.

### UX parity

- **Missing/unparseable param** (`None`): today → server round-trip → "Post not
  found". After → same "Post not found", rendered client-side with **no wasted
  round-trip**. Strictly at least as good.
- **Valid id for a nonexistent/foreign/deleted post** (`Some`): unchanged — the
  server still returns not-found.
- The `current_audience` seeding `Effect` already acts only on `Ok(selection)`;
  on `None`→`Err` the audience picker stays at its `public` default, exactly as
  today (where the `-1` audience lookup also errored and the `Effect` no-op'd).

## Scope / non-goals

- **Web-only.** No wire, schema, server-fn signature, or storage change.
- No new `-1` (or other magic-number) `PostId` sentinel anywhere in `web/`.

## Acceptance

- `post_id_param` is `impl Fn() -> Option<PostId>`; no `PostId::from(-1)` (or
  other sentinel) remains in `web/`.
- The two resources short-circuit `None` client-side to
  `WebError::not_found("Post")`; no lookup is dispatched for absence.
- The invalid/nonexistent edit-route renders "Post not found" (parity with
  today).
- New e2e: navigating to the edit route with an unparseable/nonexistent
  `post_id` shows the "Post not found" error. Existing edit-page e2e still
  passes.
- `cargo xtask validate --no-e2e` clean.
