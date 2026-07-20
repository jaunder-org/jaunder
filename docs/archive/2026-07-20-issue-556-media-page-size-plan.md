# Bounded media page size — Implementation Plan (issue #556)

> **For agentic workers:** Execute with **jaunder-iterate**. One task.

**Goal:** Adopt `common::pagination::PageSize` (`1..=50`) at `list_my_media`'s
`limit`, mirroring #537, so the media page size is bounded and defined by the
type.

**Architecture:** Type the `#[server]` `limit` arg as `Option<PageSize>`, unwrap
via `.unwrap_or_default().value()` at the storage boundary; the sole caller
passes `Some(PageSize::default())`. `offset` stays `Option<u32>`; storage
`list_media` keeps `u32`.

Spec: `docs/superpowers/specs/2026-07-20-issue-556-media-page-size.md`
(AC1–AC5).

## Global Constraints

- No `Co-Authored-By` trailer. Accessor is `.value()`. Reuse `PageSize` — no new
  newtype.
- Web has host + wasm + server-feature targets; verify with
  `cargo check -p web --all-features --all-targets`.
- Per-commit gate `cargo xtask check`; final gate
  `cargo xtask validate --no-e2e`.

---

## Task 1: Adopt `PageSize` at `list_my_media`

**Files:**

- Modify: `web/src/media/api.rs` (`list_my_media` arg + body; import).
- Modify: `web/src/media/component.rs:205` (caller; import).
- Test: `server/tests/web/web_media.rs` (add out-of-range rejection regression).

**Interfaces:**

- Consumes: `common::pagination::PageSize` (`value()`, `default()`, serde
  bridge).
- Produces:
  `list_my_media(source: Option<String>, limit: Option<PageSize>, offset: Option<u32>)`.

- [ ] **Step 1: Write the failing regression test** in
      `server/tests/web/web_media.rs` (mirror
      `list_my_media_returns_empty_for_new_user`'s setup — create a user +
      session + cookie), then POST an out-of-range limit and assert the request
      is rejected (per ADR-0065, assert non-OK, not a message):

```rust
#[apply(backends)]
#[tokio::test]
async fn list_my_media_rejects_out_of_range_limit(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"erin".parse().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("create_user failed");
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .expect("create_session failed");
    let cookie = session_cookie(&token);

    // `limit=999` is outside PageSize's `1..=50`; the typed wire arg rejects it on
    // deserialization instead of fetching an unbounded page.
    let (status, _body) =
        post_form(Arc::clone(&state), "/api/list_my_media", "limit=999", Some(&cookie)).await;

    assert_ne!(status, StatusCode::OK, "out-of-range media limit must be rejected");
}
```

- [ ] **Step 2: Run it, verify it fails (red).** Run:
      `cargo nextest run -p jaunder --test integration -E 'test(list_my_media_rejects_out_of_range_limit) and test(sqlite)'`
      Expected: FAIL — today `limit: Option<u32>` accepts `999` and returns
      `OK`.

- [ ] **Step 3: Adopt `PageSize`.**
  - `web/src/media/api.rs`: add `use common::pagination::PageSize;`; change the
    arg to `limit: Option<PageSize>`; in the body replace `limit.unwrap_or(50)`
    with `limit.unwrap_or_default().value()`. Leave `offset.unwrap_or(0)`
    unchanged.
  - `web/src/media/component.rs`: add `use common::pagination::PageSize;` (with
    the other `common::` imports); change the caller at `:205` `Some(50)` →
    `Some(PageSize::default())`.

- [ ] **Step 4: Compile web (server-gated) + run the media suite.** Run:
      `cargo check -p web --all-features --all-targets` Then:
      `cargo nextest run -p jaunder --test integration -E 'test(list_my_media) and test(sqlite)'`
      Expected: PASS — the new regression is green and the existing empty-body
      media tests (which send no `limit`) still return `OK`.

- [ ] **Step 5: Commit.** Run `cargo xtask check` first.
  ```bash
  git add web/src/media/api.rs web/src/media/component.rs server/tests/web/web_media.rs
  git commit -m "refactor(media): bound list_my_media page size with PageSize (#556)"
  ```

---

## Final gate

- [ ] `cargo xtask validate --no-e2e` (AC5) → PASS, then hand off to
      **jaunder-ship**.

## Self-review

AC1→Step 3 (api.rs); AC2→Step 3 (component.rs); AC3→unchanged storage (Step 3
uses `.value()`); AC4→Step 1 regression + existing empty-body tests; AC5→Final
gate. No new types; `offset` untouched. Type names (`PageSize::default`/`value`)
match #537's shipped API.
