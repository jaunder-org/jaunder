# Plan — atompub: content-based ETag (#78)

**Spec:**
[`2026-07-11-issue-78-content-etag.md`](../specs/2026-07-11-issue-78-content-etag.md).
Behavior, hash domain, and AC1–AC9 live there; this plan is **how**.

## Review header

**Goal.** Make the AtomPub post ETag a content hash (`"sha256-<hex>"`) computed
on read from the post's content fields, and validate `If-Match` against it on
both PUT and DELETE. Server-side Rust only.

**Scope — in:** `server/src/atompub/posts.rs` (`etag_for`, `member_delete`),
`server/tests/atompub/atompub_posts.rs` (integration). **Out:** any DB migration
/ stored column; `If-None-Match`/304 on GET; client changes; back-compat shim
(one-time 412 re-sync is accepted, per spec Non-goals).

**Tasks (one commit each):**

1. **Content-hash `etag_for`** — replace the timestamp derivation with a sha256
   over a deterministic serialization of {title, stored body, format, summary,
   tag_displays, draft}; unit tests for the pure hash properties + HTTP property
   tests. PUT `If-Match` picks up the new value automatically.
2. **DELETE `If-Match`** — add a `HeaderMap` extractor + the same
   412-on-mismatch / unconditional-if-absent block to `member_delete`;
   dual-backend integration tests.

**Key risks / decisions:**

- **Tag hashing must use only `tag_display`** (ordered), never `PostTag`'s
  DB-assigned `post_id`/`tag_id` — else two identical-content posts get
  different ETags (AC2 fails). `PostTag`/`PostFormat` are **not** `Serialize`;
  reduce every field to a plain primitive.
- **Draft = `post.published_at.is_none()`** (the boolean `post_to_entry` uses) —
  a time-independent flag; never hash the `published_at` timestamp value.
- **Stored body is already canonical** (Org canonicalized on ingest in
  `post_service` create/update; other formats verbatim), so `etag_for` hashes
  `post.body` directly and stays a pure `fn(&PostRecord)` with no
  `render`/storage dependency.
- **`serde_json::to_vec` on the content struct is infallible** here (a fixed
  local struct of `Option<&str>`/`&str`/`String`/`Vec<&str>`/`bool`), but its
  error arm is uncovered whatever combinator is used. Use a non-panicking
  fallback in the request path — `.unwrap_or_else(|_| Vec::new())` (the shape
  projector/mod.rs:72 uses for its seed `unwrap_or_else(|_| "null")`); coverage
  stays green via the CRAP threshold for this tiny function (ADR-0050), not via
  any copied "no uncovered branch" precedent.

## Global constraints

- **Language:** complete Rust; follow `CONTRIBUTING.md` (backend parity,
  dialect-file rules, coverage policy). No `#[allow]`/`#[expect]` without
  explicit approval.
- **Per-commit gate:** `cargo xtask check` (fmt + clippy + Nix coverage/tests),
  git-enforced (`jaunder-commit`). Run it clean before each commit.
- **Tests:** integration in `server/tests/atompub/atompub_posts.rs` use
  `rstest_reuse` `#[apply(backends)]` + `#[tokio::test]`,
  `backend.setup().await → TestEnv { state, base }`, `make_app(state, &base)`,
  driven via `tower::ServiceExt::oneshot`. Unit tests live in a `#[cfg(test)]`
  module in `posts.rs`.
- **No placeholders.**

---

## Task 1 — content-hash `etag_for` (Commit 1)

### Files / interfaces

**`server/src/atompub/posts.rs`** — replace `etag_for`:

```rust
use serde::Serialize;
use sha2::{Digest, Sha256};

/// A strong content-hash `ETag` for a post: `"sha256-<hex>"` over the post's
/// content fields (never a timestamp), so identical content ⇒ identical ETag
/// and an idempotent re-publish does not change it (#78).
pub(crate) fn etag_for(post: &PostRecord) -> String {
    #[derive(Serialize)]
    struct EtagContent<'a> {
        title: Option<&'a str>,
        body: &'a str,
        format: String,
        summary: Option<&'a str>,
        tags: Vec<&'a str>,
        draft: bool,
    }
    let content = EtagContent {
        title: post.title.as_deref(),
        body: &post.body,
        format: post.format.to_string(), // PostFormat: Display = "org"/"markdown"/"html"
        summary: post.summary.as_deref(),
        tags: post.tags.iter().map(|t| t.tag_display.as_str()).collect(),
        draft: post.published_at.is_none(),
    };
    let bytes = serde_json::to_vec(&content).unwrap_or_else(|_| Vec::new());
    format!("\"sha256-{:x}\"", Sha256::digest(&bytes))
}
```

At implementation: (a) `PostFormat: Display` yields `"org"/"markdown"/"html"`
(`common/src/render.rs:27`) — use it; note the private `mapping::format_to_wire`
returns _content-types_ (`"text/org"`, …), so it is **not** the fallback to
reach for. (b) the non-panicking `unwrap_or_else` above keeps the handler
panic-free; coverage is fine by the CRAP threshold (per Key-risks). (c)
`PostRecord` derives `Clone` (confirmed, `storage/src/posts.rs`), all fields
`pub`.

**No change to `member_put`** — it already calls `etag_for(&current)` for
`If-Match` and `etag_for(&post)` for the response header, so AC6 holds
automatically.

### Test

**Unit (`#[cfg(test)] mod` in `posts.rs`)** — build a base `PostRecord` (all
fields `pub`; reuse the `.parse()` + `storage::PostTag { … }` construction
pattern already in `mapping.rs`'s test module `make_post`,
`server/src/atompub/mapping.rs:486`), then clone-and-flip:

- `etag_for_is_deterministic`: same record twice → equal (AC2).
- `etag_for_ignores_identity_and_timestamps` (AC2/AC5): clones differing only in
  `post_id`, `user_id`, `slug`, `created_at`, `updated_at`, `rendered_html`,
  each `PostTag`'s `tag_id`/`post_id`, **and `published_at` advanced from
  `Some(t1)` to `Some(t2)` (still non-draft)** → **equal** ETag (nails AC5's
  timestamp-value independence, not just the draft boolean).
- `etag_for_changes_on_each_content_field` (AC4): from a base, independently
  mutating `title`, `body`, `summary`, `format`, the tag **display** set, and
  `published_at` None↔Some (the draft flip) → each **differs**.
- `etag_for_format` (AC1): matches `^"sha256-[0-9a-f]{64}"$`.

Run: `cargo nextest run -p server etag_for` — **FAIL** before (timestamp ETag
ignores content / wrong format), **PASS** after.

**Integration (`atompub_posts.rs`, `#[apply(backends)]`)** — HTTP-level
regression:

- `etag_is_content_hash_format` (AC1): create → `ETag` header matches
  `"sha256-<64hex>"`.
- `identical_posts_share_etag` (AC2): create two posts with identical
  title/body/tags/ summary/format/draft → equal `ETag` (proves
  `post_id`/`tag_id` excluded end-to-end).
- `idempotent_reput_keeps_etag` (AC3 + AC5): create, capture ETag, PUT
  byte-identical content, assert the response `ETag` is **unchanged** (the
  timestamp ETag would have bumped).
- Confirm the existing `update_with_stale_if_match_returns_412` and
  `update_with_matching_if_match_succeeds` still pass (spec §Testing verified
  they hold).

Run: `cargo nextest run -p server atompub` (targeted names) — new HTTP tests
PASS after the impl; existing If-Match tests remain green.

### Commit

`cargo xtask check` clean → commit `posts.rs` (+ `mapping.rs` only if
`format_to_wire` was exposed): `atompub: content-hash ETag for posts (#78)`.

---

## Task 2 — DELETE `If-Match` (Commit 2)

### Files / interfaces

**`server/src/atompub/posts.rs` `member_delete`** — add the header extractor and
the validation block after the owned-post load, before the soft-delete:

```rust
pub(crate) async fn member_delete(
    // …existing extractors…,
    headers: HeaderMap,        // NEW (as member_put has)
) -> Result<Response, HandlerError> {
    // …load `post` via owned_post (unchanged)…
    if let Some(if_match) = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) {
        if if_match != "*" && if_match != etag_for(&post) {
            return Err(HandlerError::PreconditionFailed);
        }
    }
    // …existing soft-delete…
}
```

Semantics identical to PUT: absent/non-UTF-8 `If-Match` → unconditional delete;
`*` or exact content-hash match → delete; mismatch → `412`. Match the existing
`member_put` block (posts.rs) verbatim so the two read the same.

### Test

**Integration (`atompub_posts.rs`, `#[apply(backends)]`)** (AC7):

- `delete_with_stale_if_match_returns_412`: create, DELETE with
  `If-Match: "\"0\""` → `412`; a follow-up GET still `200` (post not deleted).
- `delete_with_matching_if_match_succeeds`: create, read its `ETag`, DELETE with
  that as `If-Match` → **`204 No Content`** (the status the existing
  `delete_then_get_is_404` asserts); GET → `404`.
- `delete_without_if_match_succeeds`: create, DELETE with no `If-Match` → `204`
  (unconditional, unchanged default).

Run: `cargo nextest run -p server atompub` (delete names) — **FAIL** before
(DELETE ignores `If-Match`), **PASS** after.

### Commit

`cargo xtask check` clean → commit `posts.rs` + `atompub_posts.rs`:
`atompub: validate If-Match on post DELETE against the content ETag (#78)`.

## Self-review

- Every AC maps to a test: AC1 unit `etag_for_format` + HTTP format test; AC2
  unit identity-ignore + HTTP `identical_posts_share_etag`; AC3/AC5 HTTP
  `idempotent_reput_keeps_etag` (+ unit timestamp-ignore); AC4 unit per-field;
  AC6 existing PUT If-Match tests; AC7 Task 2 delete tests; AC8 existing elisp
  integration test (unchanged); AC9 `#[apply(backends)]` on every HTTP test.
- Two commits, each independently `cargo xtask check`-green; Task 1 is
  self-contained (ETag value), Task 2 adds the DELETE guard.
- No separable follow-on surfaced (a conditional-DELETE-for-others or
  If-None-Match GET would be their own issues; not in scope here).
