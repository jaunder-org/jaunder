# Spec — atompub: content-based ETag (#78)

**Issue:** #78 (milestone #4). Make the AtomPub post ETag a **content hash**
instead of the last-update timestamp, so an identical-content re-publish yields
an identical ETag — removing the time-based false-positive that would otherwise
show as "diverged" in the reconcile 2×2 (#75, Unit D). Independent follow-on;
#71 (Unit B) prerequisite is landed.

## Background (current behavior)

- `server/src/atompub/posts.rs::etag_for(post) -> String` returns
  `format!("\"{}\"", post.updated_at.timestamp_millis())` — a **strong** quoted
  ETag from the update time. Emitted on GET member, POST create, PUT update.
- `If-Match` is validated **only on PUT** (`member_put`): if present and not `*`
  and not equal to `etag_for(&current)` → `412 Precondition Failed`;
  missing/invalid → the update proceeds unconditionally. **DELETE does not check
  `If-Match`.**
- The emacs client stores the ETag verbatim as the `JAUNDER_SYNCED` org property
  and echoes it back as `If-Match` on PUT. It treats the ETag as an **opaque
  quoted strong token** (`elisp/jaunder-transport.el` even guards its
  round-trip). No web/CSR consumer reads post ETags.
- Repo already has a content-hash ETag convention: the projector uses
  `format!("\"sha256-{:x}\"", Sha256::digest(...))` → `"sha256-<hex>"`. `sha2`
  is a workspace dep. `common/src/render.rs::canonicalize_org_body()` (#71 /
  ADR-0024) is a byte-deterministic, idempotent Org canonicalizer.

## Design (resolved)

### The ETag value

`etag_for` returns `format!("\"sha256-{:x}\"", Sha256::digest(bytes))` — a
strong, quoted `"sha256-<64-lowercase-hex>"` token (matching the projector/media
convention), where `bytes` is a **deterministic serialization of the post's
content fields**.

### Hash domain — full entry content, time-independent

The hash covers exactly the fields the client round-trips as post content. Each
is reduced to a plain, `Serialize`-able primitive (`PostFormat` and `PostTag`
are **not** `Serialize`, so they are never hashed directly):

| Field   | Hash input                              | Notes                                                                                                                                                                                                                |
| ------- | --------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| title   | `post.title.as_deref(): Option<&str>`   | `None` distinct from `Some("")`                                                                                                                                                                                      |
| body    | `post.body: &str` **as stored**         | the stored body is already canonicalized for Org on ingest (`post_service` create/update) and verbatim for other formats, so `etag_for` hashes it directly — no `canonicalize_org_body` call, no `render` dependency |
| format  | `post.format.to_string(): String`       | `PostFormat`'s `Display` — `"org"`/`"markdown"`/`"html"`                                                                                                                                                             |
| summary | `post.summary.as_deref(): Option<&str>` | `None` distinct from `Some("")`                                                                                                                                                                                      |
| tags    | `post.tags.iter().map(                  | t                                                                                                                                                                                                                    | t.tag_display.as_str()): Vec<&str>` | **only `tag_display`**, ordered as stored — never `post_id`/`tag_id` (DB-assigned; differ between identical-content posts, which would break AC2/AC3) |
| draft   | `post.published_at.is_none(): bool`     | the exact boolean `post_to_entry` uses for `app:draft` (`mapping.rs`) — a **time-independent** flag, never the `published_at` timestamp value                                                                        |

**Excluded on purpose:** `updated_at`, `created_at`, `published_at` (any
timestamp _value_), `post_id`, `tag_id`, `slug`, `rendered_html`. Including a
timestamp would re-introduce the exact time drift #78 removes;
`slug`/`rendered_html` are server-derived.

**Determinism:** hash a private local `#[derive(Serialize)]` struct holding
those borrowed primitives (`Option<&str>` title/summary, `&str` body, `String`
format, `Vec<&str>` tag_displays, `bool` draft) via `serde_json::to_vec` (struct
field order is fixed; no maps/floats), then sha256 the bytes — the same
serialize-then-hash pattern the projector already uses
(`server/src/projector/mod.rs`). Equivalent logical content ⇒ identical bytes ⇒
identical ETag.

### Compute-on-read

`etag_for(post: &PostRecord)` recomputes the hash from the record each time it
is emitted or compared, staying a **pure function of `PostRecord`** (no
`render`/storage dependency, since the stored body is already canonical). **No
schema migration, no stored column, no backfill, no drift.** The post is already
re-fetched after each mutation, so the record carries every field. Cost is one
sha256 over a small text post per request — negligible.

### If-Match — PUT and DELETE

Both `member_put` and `member_delete` validate `If-Match` against the (new)
content-hash `etag_for(&current)`, with identical semantics:

- present and `== "*"` → match (wildcard), proceed;
- present and `== etag_for(&current)` → match, proceed;
- present and different → `412 Precondition Failed`;
- absent or non-UTF-8 → unconditional (proceed).

PUT keeps its current shape (only the compared value changes). `member_delete`
today takes no headers, so it gains a `headers: HeaderMap` extractor (as
`member_put` has) plus the same validation block; it already loads `current` via
`owned_post`.

## Acceptance criteria

- **AC1 — format:** every emitted ETag (GET member, POST create, PUT update) is
  a strong quoted `"sha256-<64 lowercase hex>"` token.
- **AC2 — deterministic content hash:** two posts with identical {title, body
  (canonical), format, summary, tags, draft} produce the **same** ETag; the
  value is independent of `created_at`/`updated_at`/`published_at`.
- **AC3 — stable across idempotent re-PUT (the core property):** POST a post,
  then PUT byte-identical content → the response ETag is **unchanged**
  (contrast: the old timestamp ETag bumped on every write).
- **AC4 — changes on any covered-field edit:** starting from a fixed post,
  independently editing **body**, **title**, **summary**, the **tag set**,
  **format**, or the **draft flag** each changes the ETag.
- **AC5 — time-independence:** an operation that only advances
  `updated_at`/`published_at` without changing covered content does **not**
  change the ETag (this is the false-positive removal #78 targets).
- **AC6 — PUT If-Match:** stale `If-Match` → 412; matching or `*` → 200; absent
  → unconditional 200. (Semantics unchanged; value is the content hash.)
- **AC7 — DELETE If-Match:** stale `If-Match` → 412 (post not deleted); matching
  or `*` → deletes; absent → deletes unconditionally.
- **AC8 — client contract preserved:** the ETag remains a single quoted strong
  token under the `ETag` header; the emacs client round-trips it as `If-Match`
  with **no client change**. The existing
  `elisp/test/jaunder-publish-integration.el`
  `jaunder-publish-stale-if-match-surfaces-412` still passes.
- **AC9 — backend parity:** all handler tests run against both SQLite and
  Postgres (`#[apply(backends)]`).

## Non-goals

- No `If-None-Match` / `304 Not Modified` handling added to the AtomPub member
  GET (only the ETag _value_ changes).
- No stored `content_hash` column or DB migration (compute-on-read).
- No data migration and no backward compatibility shim: after deploy, a client
  still holding a time-based `JAUNDER_SYNCED` will get a one-time `412` on its
  next PUT and re-sync. Acceptable for the pre-release single-user beta; call it
  out in the PR.
- Tag ordering is hashed as stored (not set-normalized); reordering tags is
  treated as a content change. (Revisit only if the tracker shows a tag-reorder
  churn problem.)

## Testing

- **Integration (`server/tests/atompub/atompub_posts.rs`,
  `#[apply(backends)]`):**
  - Keep the existing `update_with_stale_if_match_returns_412` (a bogus
    `"\"0\""` still mismatches) and `update_with_matching_if_match_succeeds`
    (create-ETag round-trip still holds).
  - Add: **idempotent re-PUT keeps ETag** (AC3); **each covered-field edit
    changes ETag** (AC4, at least body + title + tags cases); **DELETE stale
    If-Match → 412** and **DELETE matching/absent → deletes** (AC7); ETag format
    assertion (AC1).
- **Unit (in `server/src/atompub/posts.rs` or a `mapping`-level test):**
  `etag_for` determinism and field-sensitivity on constructed `PostRecord`s
  (fast coverage of AC2/AC4 including format + draft + summary flips without
  HTTP), and time-independence (AC5: same content, different `updated_at` → same
  ETag).
- **Client:** the elisp integration test above is the AC8 backstop; no new elisp
  needed.

Follow `CONTRIBUTING.md` backend-parity + dual-backend storage-test conventions;
the gate is `cargo xtask check` (fmt + clippy + Nix coverage/tests), `validate`
at ship.
