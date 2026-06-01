# AtomPub Interface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a standards-based AtomPub (RFC 5023) publishing interface to Jaunder — Service Document, per-user Collection, entry CRUD, and media upload — with full MarsEdit compatibility.

**Architecture:** A new authenticated AtomPub surface under `/atompub`, wired as raw Axum routes (like the media routes) rather than Leptos server functions. Pure AtomPub serialization/parsing lives in `common/src/atompub/` with **no Jaunder type coupling** (so it can later be extracted upstream); the Post↔Entry mapping lives at a boundary in `server/src/atompub/`. The public M8 Syndication Feed (HTML) and the AtomPub Collection (native source form) are deliberately separate serializers (ADR-0015). Authentication is via app-specific passwords carried over HTTP Basic, validated through the existing session-token path (ADR-0014).

**Tech Stack:** Rust, Axum, `atom_syndication` (entries, reused from M8), `quick-xml` (service/categories/RSD docs), `sqlx` (SQLite + Postgres), Leptos (web UI surfaces), Playwright (e2e via API request context).

**Prerequisite:** M8 (Published Feeds) merged into this branch. Migration numbers below assume the next free numbers after M8's `0014`/`0015` are `0016`+. Verify with `ls storage/migrations/sqlite/ | sort | tail -3` before creating migrations and renumber if needed.

**Conventions (from CLAUDE.md):**
- Never `.unwrap()`/`.expect()` in production code; parse-at-the-boundary into infallible types.
- Run `scripts/verify` after every step; request review before each commit; commit refactors before the change that uses them.
- Unit tests live in the same file as the code under test; DB tests use `sqlite::memory:` with `sqlx::migrate!("../storage/migrations/sqlite")`.
- Never update `.coverage-manifest.json` without explicit user approval.

---

## File Structure

**Phase 0 — prerequisite refactors (each independently shippable):**
- `storage/migrations/{sqlite,postgres}/0016_add_post_summary.sql` — nullable `summary` column on `posts`.
- `storage/migrations/{sqlite,postgres}/0017_sessions_label_not_null.sql` — backfill + `label NOT NULL`.
- `storage/src/posts.rs` — `PostFormat::Html`; `summary` on records/inputs.
- `storage/src/render.rs` — `render()` identity arm for `Html`.
- `storage/src/sessions.rs` — `SessionRecord.label: String`; `create_session` requires a label.
- `storage/src/user_config.rs` (existing) — `default_post_format` get/set helpers.
- `common/src/feed/metadata.rs` (M8) — prefer real `summary` over derived label.
- `web/src/posts/*`, `web/src/pages/posts.rs` — summary input + display; default-format wiring.
- `web/src/auth/*` — login generates a session label.

**Phase 1 — AtomPub core (pure, extraction-ready):**
- `common/src/atompub/mod.rs` — module root + shared types (`MediaType`, errors).
- `common/src/atompub/entry.rs` — Atom entry parse/serialize via `atom_syndication`, incl. `app:draft`/`app:edited` extension handling.
- `common/src/atompub/service.rs` — Service Document serializer (quick-xml).
- `common/src/atompub/categories.rs` — inline Categories Document serializer.
- `common/src/atompub/rsd.rs` — RSD document serializer.

**Phase 2 — AtomPub server surface:**
- `web/src/auth/*` — `AuthUser` extractor reads `Authorization: Basic`.
- `server/src/atompub/mod.rs` — router builder, shared extractors/helpers.
- `server/src/atompub/mapping.rs` — Post↔Entry / Media↔media-link-entry mapping (boundary).
- `server/src/atompub/service.rs` — `GET /atompub/service`.
- `server/src/atompub/posts.rs` — collection `GET`/`POST`, member `GET`/`PUT`/`DELETE`.
- `server/src/atompub/media.rs` — media collection `POST`, media member `GET`/`DELETE`.
- `server/src/media_manager.rs` — transport-agnostic `upload` core.
- `server/src/lib.rs` — mount `/atompub`; RSD `<link>` injection point.
- `storage/src/posts.rs` / backend impls — `list_collection_by_user`.

**Phase 3 — discovery, app-password UI, tests:**
- `web/src/pages/*` — App Password minting UI (Sessions page); RSD link on `/~username`.
- `server/src/atompub/rsd.rs` (handler) — `GET /~{username}/rsd.xml`.
- `server/tests/atompub_*.rs` — Rust integration tests per endpoint.
- `end2end/tests/atompub.spec.ts` — Playwright API-request e2e.
- `docs/atompub-marsedit-acceptance.md` — manual MarsEdit checklist.

---

## Phase 0 — Prerequisite Refactors

### Task 1: Add `summary` column to posts (storage)

**Files:**
- Create: `storage/migrations/sqlite/0016_add_post_summary.sql`
- Create: `storage/migrations/postgres/0016_add_post_summary.sql`
- Modify: `storage/src/posts.rs` (`PostRecord`, `CreatePostInput`, `UpdatePostInput`)
- Modify: backend impls under `storage/src/sqlite/`, `storage/src/postgres/`

- [ ] **Step 1: Write the failing test** in `storage/src/posts.rs` `#[cfg(test)]`:

```rust
#[tokio::test]
async fn create_post_persists_summary() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::migrate!("../storage/migrations/sqlite").run(&pool).await.unwrap();
    let posts = crate::SqlitePostStorage::new(pool);
    let rec = posts
        .create_post(&CreatePostInput {
            user_id: 1,
            title: Some("T".into()),
            slug: "t".parse().unwrap(),
            body: "b".into(),
            format: PostFormat::Markdown,
            rendered_html: "<p>b</p>".into(),
            summary: Some("the summary".into()),
            published_at: None,
        })
        .await
        .unwrap();
    assert_eq!(rec.summary.as_deref(), Some("the summary"));
}
```

- [ ] **Step 2: Run it, expect failure** — `cargo nextest run -E 'test(create_post_persists_summary)'`. Expected: compile error (`summary` field missing).

- [ ] **Step 3: Create the migrations.**

`storage/migrations/sqlite/0016_add_post_summary.sql`:
```sql
ALTER TABLE posts ADD COLUMN summary TEXT;
```
`storage/migrations/postgres/0016_add_post_summary.sql`:
```sql
ALTER TABLE posts ADD COLUMN summary TEXT;
```

- [ ] **Step 4: Add `summary: Option<String>`** to `PostRecord`, `CreatePostInput`, and `UpdatePostInput` in `storage/src/posts.rs`. Update every `SELECT`/`INSERT`/`UPDATE` and row-mapping in `storage/src/sqlite/posts.rs` and `storage/src/postgres/posts.rs` to read/write `summary` (add it to the column lists and the `PostRecord { .. }` construction). Update `render.rs`'s orchestration (`perform_post_creation`/`perform_post_update`) to thread `summary` through; default to `None` when not supplied.

- [ ] **Step 5: Run tests** — `cargo nextest run -E 'test(create_post_persists_summary)'`. Expected: PASS. Then `cargo nextest run -p storage`. Fix any other call sites that construct these structs (compiler will list them).

- [ ] **Step 6: `scripts/verify`, request review, commit.**

```bash
git add storage/migrations storage/src/posts.rs storage/src/sqlite/posts.rs storage/src/postgres/posts.rs storage/src/render.rs
git commit -m "feat(storage): add nullable summary field to posts"
```

### Task 2: Surface `summary` in web create/update + feed reconciliation

**Files:**
- Modify: `web/src/posts/mod.rs` (`create_post`/`update_post` server fns, `PostResponse`)
- Modify: `web/src/posts/server.rs` (`post_response`)
- Modify: `web/src/pages/posts.rs` (composer form + display)
- Modify: `common/src/feed/metadata.rs` / M8 feed item construction (prefer real summary)

- [ ] **Step 1: Write the failing test** in `web/src/posts/mod.rs` tests (SSR): assert `post_response` carries `summary` through. Mirror the existing `post_response_marks_draft_state_from_published_at` test, adding `summary: Some("s".into())` to the `PostRecord` and asserting `result.summary.as_deref() == Some("s")`.

- [ ] **Step 2: Run, expect fail** — `cargo nextest run -E 'test(post_response)' -p web`. Expected: field missing.

- [ ] **Step 3: Add `summary: Option<String>`** to `PostResponse` and `CreatePostResult`/`UpdatePostResult` as appropriate; populate in `post_response`. Add a `summary: Option<String>` parameter to the `create_post`/`update_post` server fns and pass into `CreatePostInput`/`UpdatePostInput`.

- [ ] **Step 4: Add the composer field** in `web/src/pages/posts.rs` — a `<textarea>`/`<input>` bound to a `summary` signal, submitted with the form; render the summary where the post is displayed.

- [ ] **Step 5: Feed reconciliation** — in `server/src/feed/regenerate.rs:153`, change the hardcoded `summary: None,` to `summary: p.summary.clone(),` (M8 never derived a feed summary, so this is a clean one-liner — no fallback expression needed). `p` is a `&PostRecord`, which carries `summary` after Task 1. The Atom/JSON serializers already emit `<summary>`/`"summary"` when the field is `Some` (`common/src/feed/atom.rs:55`, `common/src/feed/json.rs:20`).

- [ ] **Step 6: Run** `cargo nextest run -p web -p common`. Expected: PASS.

- [ ] **Step 7: `scripts/verify`, review, commit.**

```bash
git commit -m "feat(web): expose post summary in composer and feed"
```

### Task 3: Add `PostFormat::Html`

**Files:**
- Modify: `storage/src/posts.rs:15` (enum + `Display` + `FromStr`)
- Modify: `storage/src/render.rs` (`render` match)

- [ ] **Step 1: Write failing tests** in `storage/src/posts.rs` tests:

```rust
#[test]
fn post_format_html_roundtrips() {
    assert_eq!("html".parse::<PostFormat>().unwrap(), PostFormat::Html);
    assert_eq!(PostFormat::Html.to_string(), "html");
}
```
and in `storage/src/render.rs` tests:
```rust
#[test]
fn render_html_is_identity() {
    let body = "<p>hi <b>there</b></p>";
    assert_eq!(render(body, &PostFormat::Html).unwrap(), body);
}
```

- [ ] **Step 2: Run, expect fail** — `cargo nextest run -E 'test(post_format_html_roundtrips) + test(render_html_is_identity)'`.

- [ ] **Step 3: Implement.** Add `Html` variant; `Display` → `"html"`; `FromStr` arm `"html" => Ok(PostFormat::Html)`; in `render.rs` add `PostFormat::Html => Ok(body.to_string())`. Also update `derive_post_metadata`'s `match format` to add an `Html` arm: extract a title from the first `<h1>…</h1>` if trivially present, else `None` (keep it simple — `PostFormat::Html => None` for title extraction is acceptable; the `fallback_label` still applies).

- [ ] **Step 4: Run** the two tests, expect PASS; then `cargo nextest run -p storage` to catch non-exhaustive matches.

- [ ] **Step 5: `scripts/verify`, review, commit** — `git commit -m "feat(storage): add Html post format with identity render"`.

### Task 4: `default_post_format` user preference

**Files:**
- Modify: `storage/src/user_config.rs` (constant key + typed get/set)
- Test: same file

- [ ] **Step 1: Write failing test** in `storage/src/user_config.rs` tests: set `default_post_format` to `"org"` for a user, read it back as `PostFormat::Org`; default (unset) returns `PostFormat::Html`.

- [ ] **Step 2: Run, expect fail.**

- [ ] **Step 3: Implement** `pub const DEFAULT_POST_FORMAT_KEY: &str = "posts.default_format";` and helpers `get_default_post_format(user_id) -> PostFormat` (parse stored string, fall back to `PostFormat::Html` on missing/invalid) and `set_default_post_format(user_id, PostFormat)`. Reuse the existing user_config get/set primitives.

- [ ] **Step 4: Run, expect PASS.**

- [ ] **Step 5: Wire the web composer default** — in `web/src/pages/posts.rs`, initialize the format selector from `get_default_post_format` for the current user. Add a preferences control to set it (Sessions/account page or composer).

- [ ] **Step 6: `scripts/verify`, review, commit** — `git commit -m "feat: per-user default post format preference"`.

### Task 5: Make `sessions.label` non-optional

**Files:**
- Create: `storage/migrations/{sqlite,postgres}/0017_sessions_label_not_null.sql`
- Modify: `storage/src/sessions.rs` (`SessionRecord.label: String`, `create_session(user_id, label: &str)`)
- Modify: `storage/src/{sqlite,postgres}/sessions.rs`
- Modify: `web/src/auth/*` (login supplies a label)

- [ ] **Step 1: Write failing test** in `storage/src/sessions.rs` tests: `create_session(1, "Firefox on Linux")` then `list_sessions(1)` returns a record whose `label == "Firefox on Linux"` (note `label` is now `String`, not `Option`).

- [ ] **Step 2: Run, expect fail** (signature/type mismatch).

- [ ] **Step 3: Migrations.** SQLite (rebuild table, since SQLite can't add NOT NULL to existing nullable directly with a default backfill cleanly):
```sql
UPDATE sessions SET label = 'Unknown device' WHERE label IS NULL;
-- SQLite: recreate with NOT NULL
CREATE TABLE sessions_new (
    token_hash   TEXT PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES users(user_id),
    label        TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    last_used_at TEXT NOT NULL
);
INSERT INTO sessions_new SELECT token_hash, user_id, label, created_at, last_used_at FROM sessions;
DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;
```
Postgres:
```sql
UPDATE sessions SET label = 'Unknown device' WHERE label IS NULL;
ALTER TABLE sessions ALTER COLUMN label SET NOT NULL;
```

- [ ] **Step 4: Change types.** `SessionRecord.label: String`; `create_session(&self, user_id: i64, label: &str)`; update INSERT and row mapping in both backends.

- [ ] **Step 5: Update callers.** In `web/src/auth/server.rs` (login), derive a label from the `User-Agent` header (extract via `leptos_axum::extract` of `axum::http::HeaderMap`; fall back to `"Unknown device"`), pass to `create_session`. Update all other `create_session` callers (compiler lists them).

- [ ] **Step 6: Run** `cargo nextest run -p storage -p web`, expect PASS.

- [ ] **Step 7: `scripts/verify`, review, commit** — `git commit -m "refactor(storage): require session label; web login derives it"`.

### Task 6: `list_collection_by_user` storage method

**Files:**
- Modify: `storage/src/posts.rs` (trait method + cursor type)
- Modify: `storage/src/{sqlite,postgres}/posts.rs`

- [ ] **Step 1: Write failing test** in `storage/src/sqlite/posts.rs` tests: create 3 posts for a user (mixed draft/published, varying `updated_at`), call `list_collection_by_user(user_id, None, 10)`, assert all 3 returned **ordered by `updated_at DESC, post_id DESC`** and a soft-deleted post is excluded.

- [ ] **Step 2: Run, expect fail.**

- [ ] **Step 3: Implement.** Add to `PostStorage`:
```rust
async fn list_collection_by_user(
    &self,
    user_id: i64,
    cursor: Option<&PostCursor>,
    limit: u32,
) -> sqlx::Result<Vec<PostRecord>>;
```
SQL (both backends), keyset pagination on `(updated_at, post_id)`:
```sql
SELECT <all post columns incl. summary>
FROM posts
WHERE user_id = ? AND deleted_at IS NULL
  AND (?cursor_null OR (updated_at, post_id) < (?cur_updated, ?cur_id))
ORDER BY updated_at DESC, post_id DESC
LIMIT ?;
```
Reuse the existing `PostCursor`; if its field is `created_at`, add a parallel `CollectionCursor { updated_at, post_id }` rather than overloading. Keep tags hydrated the same way the other list methods do.

- [ ] **Step 4: Run, expect PASS;** then `cargo nextest run -p storage`.

- [ ] **Step 5: `scripts/verify`, review, commit** — `git commit -m "feat(storage): list_collection_by_user ordered by updated_at"`.

---

## Phase 1 — AtomPub Core (pure, extraction-ready)

> Rule for this phase: **no `storage`/`web` imports**. These modules model AtomPub generically and take/return plain data (strings, enums, small structs). Add `quick-xml` to `common/Cargo.toml` only if not already transitive; confirm with `cargo tree -p common | grep quick-xml`.

### Task 7: Entry parse/serialize with `app:` extensions

**Files:**
- Create: `common/src/atompub/mod.rs`
- Create: `common/src/atompub/entry.rs`
- Modify: `common/src/lib.rs` (`pub mod atompub;`)

- [ ] **Step 1: Define the generic types** in `entry.rs` (no Jaunder coupling):

```rust
/// Content as carried on the AtomPub wire, decoupled from any CMS model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryContent {
    Text(String),
    Html(String),
    Xhtml(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntry {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub content: Option<EntryContent>,
    pub categories: Vec<String>,
    pub is_draft: bool,
}
```

- [ ] **Step 2: Write the failing parse test:**

```rust
#[test]
fn parses_draft_html_entry_with_category() {
    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Hello</title>
  <summary>sum</summary>
  <content type="html">&lt;p&gt;hi&lt;/p&gt;</content>
  <category term="rust"/>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#;
    let parsed = parse_entry(xml).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("Hello"));
    assert_eq!(parsed.summary.as_deref(), Some("sum"));
    assert_eq!(parsed.content, Some(EntryContent::Html("<p>hi</p>".into())));
    assert_eq!(parsed.categories, vec!["rust".to_string()]);
    assert!(parsed.is_draft);
}
```

- [ ] **Step 3: Run, expect fail** — `cargo nextest run -E 'test(parses_draft_html_entry_with_category)' -p common`.

- [ ] **Step 4: Implement `parse_entry`** using `atom_syndication::Entry::read_from(BufReader::new(xml.as_bytes()))`. Map `entry.content()` `content_type` (`text`/`html`/`xhtml`) → `EntryContent`; map `entry.categories()` → terms; read `app:control/app:draft` from `entry.extensions()` (the `app` namespace map) — `is_draft = value.trim().eq_ignore_ascii_case("yes")`. Return a typed `AtomPubError::Malformed` (defined in `mod.rs`) on read failure — never panic, never `.unwrap()`.

- [ ] **Step 5: Write the failing serialize test:**

```rust
#[test]
fn serializes_text_entry_with_edit_links() {
    let out = serialize_entry(&EntrySerializeInput {
        id: "tag:example.com,2026:post/1".into(),
        title: Some("Hello".into()),
        summary: Some("sum".into()),
        content: EntryContent::Text("# md".into()),
        categories: vec!["rust".into()],
        edit_uri: "https://h/atompub/alice/posts/1".into(),
        alternate_uri: Some("https://h/~alice/2026/01/01/hello".into()),
        published_rfc3339: Some("2026-01-01T00:00:00Z".into()),
        updated_rfc3339: "2026-01-02T00:00:00Z".into(),
        is_draft: false,
    });
    assert!(out.contains("type=\"text\""));
    assert!(out.contains("rel=\"edit\""));
    assert!(out.contains("# md"));
}
```

- [ ] **Step 6: Run, expect fail.**

- [ ] **Step 7: Implement `serialize_entry`** building an `atom_syndication::Entry`: set `id`, `title`, `summary`, `content` (`Content { content_type: Some("text"|"html"), value: Some(..) }`), `categories`, `links` (`rel="edit"`, optional `rel="alternate"`), `published`/`updated`, and `app:edited` + `app:control/app:draft` via the extension API. Return `entry.to_string()`.

- [ ] **Step 8: Run both tests, expect PASS.**

- [ ] **Step 9: `scripts/verify`, review, commit** — `git commit -m "feat(common/atompub): generic entry parse/serialize with app: extensions"`.

### Task 8: Service, Categories, and RSD documents

**Files:**
- Create: `common/src/atompub/service.rs`, `categories.rs`, `rsd.rs`

- [ ] **Step 1: Write failing tests** (one per document) asserting the well-formed XML contains the required elements. Example for service:

```rust
#[test]
fn service_document_lists_two_collections() {
    let out = render_service_document(&ServiceDocument {
        workspace_title: "Alice".into(),
        posts_collection: CollectionDecl {
            href: "https://h/atompub/alice/posts".into(),
            title: "Posts".into(),
            accept: vec!["application/atom+xml;type=entry".into()],
            categories: vec!["rust".into(), "leptos".into()],
        },
        media_collection: CollectionDecl {
            href: "https://h/atompub/alice/media".into(),
            title: "Media".into(),
            accept: vec!["image/png".into(), "image/jpeg".into(),
                         "image/gif".into(), "image/webp".into()],
            categories: vec![],
        },
    });
    assert!(out.contains("app:service"));
    assert!(out.contains("https://h/atompub/alice/posts"));
    assert!(out.contains("type=entry"));
    assert!(out.contains("image/webp"));
    assert!(out.contains("app:categories"));
    assert!(out.contains("fixed=\"no\""));
}
```

- [ ] **Step 2: Run, expect fail.**

- [ ] **Step 3: Implement** with `quick-xml::writer::Writer`. `render_service_document` emits `app:service > app:workspace(atom:title) > app:collection*`, each collection with `app:accept` per media type and (when non-empty) `app:categories fixed="no"` with inline `atom:category term="..."`. `render_categories_document` emits a standalone `app:categories`. `render_rsd_document` emits the RSD 1.0 envelope:

```rust
pub fn render_rsd_document(service_url: &str, homepage_url: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<rsd version="1.0" xmlns="http://archipelago.phrasewise.com/rsd">
  <service>
    <engineName>Jaunder</engineName>
    <homePageLink>{homepage}</homePageLink>
    <apis>
      <api name="Atom" preferred="true" apiLink="{service}" blogID=""/>
    </apis>
  </service>
</rsd>"#,
        homepage = xml_escape(homepage_url),
        service = xml_escape(service_url),
    )
}
```
Provide a small `xml_escape` helper (or reuse `quick_xml::escape::escape`). No `.unwrap()` — build into a `String` buffer and return; on writer error return `AtomPubError`.

- [ ] **Step 4: Run all three tests, expect PASS.**

- [ ] **Step 5: `scripts/verify`, review, commit** — `git commit -m "feat(common/atompub): service, categories, and RSD serializers"`.

---

## Phase 2 — AtomPub Server Surface

### Task 9: HTTP Basic auth in `AuthUser`

**Files:**
- Modify: `web/src/auth/server.rs` (or wherever the `AuthUser` `FromRequestParts` impl lives)
- Test: same module

- [ ] **Step 1: Write failing test** — a unit test for the credential extraction helper: `parse_basic_auth("Basic YWxpY2U6dG9rMTIz")` returns `("alice", "tok123")`; malformed/non-Basic returns `None`.

- [ ] **Step 2: Run, expect fail.**

- [ ] **Step 3: Implement `parse_basic_auth(header: &str) -> Option<(String, String)>`** (base64-decode the credentials, split on first `:`). In the `AuthUser` extractor, after the existing cookie/bearer checks, if an `Authorization: Basic` header is present: parse it, call `SessionStorage::authenticate(password)`, and **verify the decoded username equals the session's `username`** — reject (`401`) on mismatch. Reuse the existing `AuthUser` construction.

- [ ] **Step 4: Run, expect PASS.**

- [ ] **Step 5: Integration test** in `server/tests/atompub_auth.rs`: build `create_router`, create a user + session via storage, then `GET /atompub/service` with `Authorization: Basic base64(user:token)` → expect `200`; wrong username → `401` with `WWW-Authenticate: Basic`. (This test will be completed once Task 11 lands the route; stub the assertion now as `#[ignore]` if route absent, un-ignore in Task 11.)

- [ ] **Step 6: `scripts/verify`, review, commit** — `git commit -m "feat(auth): accept app passwords via HTTP Basic"`.

### Task 10: Transport-agnostic media upload core

**Files:**
- Modify: `server/src/media_manager.rs`
- Modify: `server/src/media.rs` (multipart handler delegates)

- [ ] **Step 1: Write failing test** in `media_manager.rs` tests: call a new `upload_bytes(&auth_user, filename, content_type, bytes)` with a small PNG byte slice; assert it returns an `UploadResponse` with the expected sha256 and that a second identical call returns the existing record (dedup) without error.

- [ ] **Step 2: Run, expect fail.**

- [ ] **Step 3: Refactor.** Extract the storage/dedup/quota/DB logic from `upload(field)` into `upload_bytes(&self, auth_user, filename: &str, content_type: &str, reader: impl AsyncRead)` (or accept `bytes::Bytes`). Re-express the existing multipart `upload(field)` as: read filename/content_type from the field, then delegate to `upload_bytes`. Map `CreateMediaError::AlreadyExists` to returning the existing record (look it up via `get_media`) rather than erroring.

- [ ] **Step 4: Run, expect PASS;** then `cargo nextest run -p server -E 'test(upload)'` to confirm the multipart path still works.

- [ ] **Step 5: `scripts/verify`, review, commit** — `git commit -m "refactor(server): transport-agnostic media upload core"`.

### Task 11: Post↔Entry mapping + posts collection/member handlers

**Files:**
- Create: `server/src/atompub/mod.rs`, `mapping.rs`, `service.rs`, `posts.rs`
- Modify: `server/src/lib.rs` (mount routes), `server/src/lib.rs` module decl

- [ ] **Step 1: Write failing mapping unit tests** in `mapping.rs`: `parsed_entry_to_create_input(parsed, default_format)` maps `EntryContent::Html` → `(format=Html, body=html)`, `EntryContent::Text` → `(format=default_format, body=text)`, `is_draft=true` → `published_at=None`; and `post_to_entry(post, base_url)` produces an `EntrySerializeInput` whose `content` is `Text` for a Markdown post (native source) and `Html` for an Html post, with `edit_uri = {base}/atompub/{user}/posts/{id}`.

- [ ] **Step 2: Run, expect fail.**

- [ ] **Step 3: Implement `mapping.rs`** — pure functions converting between `common::atompub` types and storage inputs/records. This is the only place that couples AtomPub to Jaunder's `Post`. `post_to_entry` selects content form by `post.format` per ADR-0015; the public `alternate` link is set only when `published_at.is_some()`.

- [ ] **Step 4: Run mapping tests, expect PASS.**

- [ ] **Step 5: Implement handlers** in `posts.rs`:
  - `GET /atompub/{username}/posts` → `list_collection_by_user`, render an Atom feed of entries + RFC 5005 `rel=next/previous/first` paging links (cursor = `updated_at,post_id`; page size default 25, clamp 50); `Content-Type: application/atom+xml;type=feed`.
  - `POST /atompub/{username}/posts` → read body, `parse_entry`, map via `default_post_format`, `perform_post_creation`, apply categories as tags; respond `201` + `Location: {member edit uri}` + serialized entry.
  - `GET/PUT/DELETE /atompub/{username}/posts/{post_id}` → member fetch (404 if missing/foreign/deleted; emit `ETag` from `updated_at`), update (honor `If-Match` if present → `412`; `app:draft` yes→no publishes), and `soft_delete_post` (`204`).
  All handlers require `AuthUser` and enforce `auth.username == {username}` (else `403`). Implement `service.rs` `GET /atompub/service` using `render_service_document` with the user's tags from tag storage.

- [ ] **Step 6: Mount in `server/src/lib.rs`** — add the `/atompub/...` routes to the router (alongside the `/media` routes), passing `Extension(state)`. Add `pub mod atompub;` to `server/src/lib.rs`.

- [ ] **Step 7: Integration tests** `server/tests/atompub_posts.rs`: full create→get→list→edit→delete cycle over the router with Basic auth; assert status codes, `Location`, `ETag`, native-source round-trip (Markdown post returns `type="text"`). Un-ignore the Task 9 auth test.

- [ ] **Step 8: `scripts/verify`, review, commit** — `git commit -m "feat(server/atompub): service document and posts collection/member"`.

### Task 12: Media collection/member handlers

**Files:**
- Create: `server/src/atompub/media.rs`
- Modify: `server/src/atompub/mod.rs`, `server/src/lib.rs`

- [ ] **Step 1: Write failing integration test** `server/tests/atompub_media.rs`: `POST /atompub/{user}/media` with raw PNG bytes, `Content-Type: image/png`, `Slug: pic.png`, Basic auth → `201`, `Location` to `/atompub/{user}/media/{sha}/pic.png`, body is a media-link entry whose `<content src>` and `rel="edit-media"` are an **absolute** `/media/upload/...` URL. Re-POST identical → `200` with the same entry. `DELETE` the member → `204`.

- [ ] **Step 2: Run, expect fail.**

- [ ] **Step 3: Implement handlers.** `POST` reads the raw body + `Content-Type` + sanitized `Slug` filename, calls `MediaManager::upload_bytes`, builds a media-link entry (new `common::atompub` helper `serialize_media_link_entry(id, edit_uri, edit_media_uri, content_type, published, updated)`), responds `201`/`200`. `GET` returns the media-link entry; `DELETE` calls `delete_media` → `204`. Absolute URLs use the site base-URL config (same source M8 uses for `canonical_url`). Enforce `auth.username == {username}`.

- [ ] **Step 4: Run, expect PASS.**

- [ ] **Step 5: `scripts/verify`, review, commit** — `git commit -m "feat(server/atompub): media collection upload and member"`.

---

## Phase 3 — Discovery, App Passwords, Tests

### Task 13: RSD autodiscovery on `/~username`

**Files:**
- Create: `server/src/atompub/rsd.rs` (handler) or add to `service.rs`
- Modify: `server/src/lib.rs` (route `GET /~{username}/rsd.xml`)
- Modify: the `/~username` page component (`web/src/pages/*`) — inject `<link rel="EditURI">`

- [ ] **Step 1: Write failing integration test:** `GET /~alice/rsd.xml` → `200`, `Content-Type: application/rsd+xml`, body contains `apiLink="https://host/atompub/service"`.

- [ ] **Step 2: Run, expect fail.**

- [ ] **Step 3: Implement** the handler using `render_rsd_document(service_url, homepage_url)` with absolute URLs from site config. Add the route. In the user profile/blog page head, add `<link rel="EditURI" type="application/rsd+xml" href="/~{username}/rsd.xml"/>`.

- [ ] **Step 4: Run, expect PASS.**

- [ ] **Step 5: `scripts/verify`, review, commit** — `git commit -m "feat(atompub): RSD autodiscovery on user pages"`.

### Task 14: App Password minting UI

**Files:**
- Modify: `web/src/sessions/*` (server fn to mint), `web/src/pages/sessions.rs` (UI)

- [ ] **Step 1: Write failing test** for a `create_app_password(label)` server fn: returns the raw token once; the session then appears in `list_sessions` with that label.

- [ ] **Step 2: Run, expect fail.**

- [ ] **Step 3: Implement** `create_app_password` server fn (`require_auth`, then `create_session(user_id, &label)`), returning the raw token. Add UI on the Sessions page: a labeled "Create App Password" form that displays the token **once** with copy instructions, and lists existing sessions (app passwords + browser logins) with revoke buttons (existing `revoke_session`).

- [ ] **Step 4: Run, expect PASS.**

- [ ] **Step 5: `scripts/verify`, review, commit** — `git commit -m "feat(web): app password minting in sessions UI"`.

### Task 15: End-to-end (Playwright API request) + manual checklist

**Files:**
- Create: `end2end/tests/atompub.spec.ts`
- Create: `docs/atompub-marsedit-acceptance.md`

- [ ] **Step 1: Write the e2e spec** using the `request` fixture (no browser UI): register a user (existing helper), mint an app password (via the server fn or storage seed), then exercise the full AtomPub flow over HTTP against the VM server — service doc, create post (Basic auth), list collection (assert paging links), fetch member (assert native-source `type="text"` for a Markdown post created via web), edit, upload media (raw bytes + `Slug`), delete. Assert status codes and key XML fragments.

- [ ] **Step 2: Run** `nix build .#nix-only-checks` (or the project's e2e runner) — expect PASS. (Per CLAUDE.md, `scripts/verify` runs this.)

- [ ] **Step 3: Write `docs/atompub-marsedit-acceptance.md`** — a manual checklist: configure MarsEdit with the service URL + app password; verify connect, list, create (HTML and Markdown-as-text), edit (incl. title-only edit preserves source format), draft toggle, category autocomplete, image upload + embed, delete. Note the open `type="text"` round-trip question (ADR-0015) to verify here.

- [ ] **Step 4: `scripts/verify`, review, commit** — `git commit -m "test(atompub): e2e API-request suite and manual MarsEdit checklist"`.

---

## Self-Review Notes

- **Spec coverage:** auth (Task 9, ADR-0014), two-surface serialization (Tasks 7/11, ADR-0015), `Html` format (Task 3), `default_post_format` (Task 4), `summary` (Tasks 1–2), media (Tasks 10/12), collection listing+paging (Tasks 6/11), service doc (Task 11), categories autocomplete (Tasks 8/11), autodiscovery/RSD (Task 13), app passwords + non-null label (Tasks 5/14), deletes + idempotent re-upload (Tasks 11/12/10), concurrency ETag/If-Match (Task 11), wire content-types/status (Tasks 11/12), code placement + deps (Phases 1–2), tests (Task 15). All Q1–Q17 decisions mapped.
- **Authz invariant:** every handler enforces `auth.username == {username}` and the Basic username/token match (Task 9).
- **Migration numbers** are provisional (`0016`/`0017`) pending the post-M8 state — verify and renumber at execution time.
- **Open verification** (not a code task): MarsEdit `type="text"` round-trip behavior, resolved during Task 15 manual acceptance; fallback to last-write-wins conversion if needed (ADR-0015).
