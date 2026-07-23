# Collapse `PostRow`/`PostRecordParts` into one `FromRow` struct — Plan

> **For agentic workers:** Execute with jaunder-iterate; tick `- [ ]` → `- [x]`
> in real time.

**Goal:** Replace the two identical 14-tuple aliases (`PostRow`,
`PostRecordParts`) in `storage/src/helpers.rs` with one
`#[derive(sqlx::FromRow)]` struct, decoding by column name — the entire diff in
one file.

**Spec:** `docs/superpowers/specs/2026-07-23-issue-616-postrow-dedup.md`.

## Global Constraints

- **Single-file diff.** Keep the names `PostRow` and `post_record_from_row`, so
  the 21 `query_as::<_, PostRow>` sites (`posts.rs` + both backend files) are
  untouched. `git diff wt-base-issue-616 --stat` must list only
  `storage/src/helpers.rs`.
- **By-name decode is behaviour-preserving** only because field names match the
  SELECT columns exactly:
  `post_id, user_id, username, title, slug, body, format, rendered_html, created_at, updated_at, published_at, deleted_at, summary, tags`.
  Don't rename a field off a column.
- **Backend parity (ADR-0053):** the existing dual-backend post-CRUD tests are
  the regression net.
- **Commits:** `cargo xtask check` before commit (jaunder-commit); conventional
  message; **no `Co-Authored-By`**.

---

### Task 1: `PostRow` tuple → `FromRow` struct (storage/src/helpers.rs)

**Files:** Modify `storage/src/helpers.rs` only.

**Interfaces:**

- Produces: `pub(crate) struct PostRow` (fields below),
  `#[derive(sqlx::FromRow)]`.
  `build_post_record(row: PostRow) -> sqlx::Result<PostRecord>` and
  `post_record_from_row(row: PostRow) -> sqlx::Result<PostRecord>` keep their
  names/signatures-by-name (param type still spelled `PostRow`).
- `PostRecordParts` is **deleted**.

This is a refactor: the **existing** dual-backend post-CRUD tests + the
`build_post_record` unit tests (`helpers.rs:501,573,600,623,729`) are the
behaviour contract — they must stay green (with their row _constructions_
updated from tuple to struct). "Red" here is the compile error from the old
tuple constructions; "green" is after they're converted.

- [ ] **Step 1: Define the struct + rewrite `build_post_record`; delete
      `PostRecordParts`.**

Replace the `PostRecordParts` tuple alias (`helpers.rs:125-146`, comment +
alias) and the `PostRow` tuple alias (`helpers.rs:276-294`) with a single struct
at the `PostRow` location, and delete the `PostRecordParts` alias entirely:

```rust
/// One row of the post read model, decoded by column name from every post SELECT.
///
/// Field names are the SELECT **column** names (`username` from `u.username`, `tags`
/// from the `… AS tags` JSON aggregate) — `#[derive(FromRow)]` binds by name. The
/// newtype columns decode via the sqlx bridge (#438; `format` via its text-enum
/// bridge, #572). `rendered_html` decodes as `String` (its bridge is write-only,
/// #502) and is rebuilt via the gated `from_trusted` in `build_post_record`.
#[derive(sqlx::FromRow)]
pub(crate) struct PostRow {
    post_id: i64,
    user_id: i64,
    username: Username,
    title: Option<PostTitle>,
    slug: Slug,
    body: PostBody,
    format: PostFormat,
    rendered_html: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
    summary: Option<PostSummary>,
    tags: String,
}
```

Rewrite `build_post_record` (`helpers.rs:177-220`) to take the struct and access
named fields (behaviour identical — same `PostRecord`, same `from_trusted`
rebuild, same `parse_post_tags_json` fallibility):

```rust
pub(crate) fn build_post_record(row: PostRow) -> sqlx::Result<PostRecord> {
    // author_username/title/slug/body/format decoded via the sqlx bridge; tags parse here.
    let post_id = PostId::from(row.post_id);
    let tags = parse_post_tags_json(&row.tags, post_id)?;
    Ok(PostRecord {
        post_id,
        user_id: UserId::from(row.user_id),
        author_username: row.username,
        title: row.title,
        slug: row.slug,
        body: row.body,
        format: row.format,
        // Trusted rebuild: this column only ever holds prior `render()` output.
        rendered_html: RenderedHtml::from_trusted(row.rendered_html),
        created_at: row.created_at,
        updated_at: row.updated_at,
        published_at: row.published_at,
        deleted_at: row.deleted_at,
        summary: row.summary,
        tags,
    })
}
```

Leave `post_record_from_row` (`helpers.rs:296-298`) exactly as-is —
`fn post_record_from_row(row: PostRow) -> sqlx::Result<PostRecord> { build_post_record(row) }`
now takes the struct, still delegates.

- [ ] **Step 2: Run tests, verify they FAIL (compile error).**

Run: `cargo nextest run -p storage helpers` Expected: FAIL — the `#[cfg(test)]`
constructions still pass tuples (`build_post_record((…))` /
`let row: PostRow = (…)`), which no longer match the struct.

- [ ] **Step 3: Convert the five test constructions** to struct literals
      (`helpers.rs` test module):
  - `build_post_record((…))` at `:503`, `:576`, `:602`, `:627` →
    `build_post_record(PostRow { post_id: 10, user_id: 20, username: parse_username("alice"), title: …, slug: parse_slug(…), body: "Body".into(), format: PostFormat::Markdown, rendered_html: "<p>Body</p>".to_string(), created_at: now, updated_at: now, published_at: …, deleted_at: None, summary: None, tags: <the same tags string> })`
    (map each positional value to its named field, preserving the per-test
    values — e.g. the tags-JSON tests keep their JSON string in `tags`).
  - `let row: PostRow = (…)` at `:731` → `let row = PostRow { … }` with the same
    field values.

- [ ] **Step 4: Run tests, verify they PASS.**

Run: `cargo nextest run -p storage helpers posts` Expected: PASS —
`build_post_record` unit tests + the dual-backend
post-CRUD/round-trip/malformed-column tests green (sqlite locally; postgres via
the commit gate). The malformed-column tests still match
`sqlx::Error::ColumnDecode { .. }` (wildcard, unaffected by the index→name
change).

- [ ] **Step 5: Confirm single-file diff + gate.**

Run: `git diff wt-base-issue-616 --stat` → only `storage/src/helpers.rs`. Run:
`rg -n 'PostRecordParts' storage/src` → empty. Run:
`cargo xtask check --no-test` → PASS (fmt + clippy + workspace build, incl. the
21 `PostRow` consumers unchanged).

- [ ] **Step 6: Commit.**

```bash
git add storage/src/helpers.rs
git commit -m "refactor(storage): collapse PostRow/PostRecordParts into one FromRow struct (#616)"
```

Run `cargo xtask check` first (jaunder-commit); the pre-commit gate runs
dual-backend coverage. No `Co-Authored-By`.
