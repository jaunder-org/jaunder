# Spec — issue #616: collapse `PostRow`/`PostRecordParts` into one `FromRow` struct

/ Issue: jaunder-org/jaunder#616 · Labels: `dx` · Milestone: none / Worktree:
`.claude/worktrees/issue-616-postrow-dedup`

## Problem

`storage/src/helpers.rs` has **two byte-identical 14-tuple aliases** for the
same post row shape:

- `PostRecordParts` (`helpers.rs:131`) — the param type of `build_post_record`.
- `PostRow` (`helpers.rs:279`) — the `query_as::<_, PostRow>` decode target at
  **21** sites (19 SELECTs in `storage/src/posts.rs` + the two
  `UPDATE … RETURNING` paths in `storage/src/sqlite/posts.rs` and
  `storage/src/postgres/posts.rs`); each maps the row via
  `post_record_from_row(row: PostRow)`, which just delegates to
  `build_post_record(row)`.

A single change to the row shape forces the identical edit in both aliases
(_Duplicated Code_ + _Shotgun Surgery_ — #572 had to edit element 7 in both). A
positional 14-tuple is also hard to read and easy to misindex at the
`build_post_record((...))` sites.

## Change

Replace **both** tuple aliases with **one named struct** deriving
`sqlx::FromRow`, and merge the delegating helper into the builder.

```rust
// storage/src/helpers.rs
#[derive(sqlx::FromRow)]
pub(crate) struct PostRow {
    post_id: i64,
    user_id: i64,
    username: Username,          // column `username` (from `u.username`)
    title: Option<PostTitle>,
    slug: Slug,
    body: PostBody,
    format: PostFormat,          // decoded via its text-enum bridge (#572)
    rendered_html: String,       // write-only bridge (#502): decodes as String, rebuilt below
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
    summary: Option<PostSummary>,
    tags: String,                // column `tags` (JSON aggregate), parsed below
}
```

- `build_post_record(row: PostRow) -> sqlx::Result<PostRecord>` accesses named
  fields (`row.post_id`, `row.username` → `author_username`, `row.tags` →
  `parse_post_tags_json`, `RenderedHtml::from_trusted(row.rendered_html)`, …).
  Delete **only** `PostRecordParts`; `build_post_record` takes `PostRow`.
- **Keep `post_record_from_row(row: PostRow)`** as-is — it is the name the 21
  decode sites call (`posts.rs` ×19 + both backend files). Retaining it (and the
  `PostRow` type name) means **no call site or backend file changes** — the
  entire diff lands in `storage/src/helpers.rs`. Do NOT delete the delegator or
  rename to `build_post_record` at the call sites; that would needlessly churn
  `posts.rs` + both backend files.
- The 21 `query_as::<_, PostRow>(&sql)` sites and their SELECTs (incl. the two
  `… RETURNING` paths) are **unchanged**.

### The one behavioral point: decode mechanism switches positional → by-name

The tuple decoded **positionally** (sqlx's tuple `FromRow`, by column index);
the derived struct decodes **by name** (`try_get("field")`). This is sound here
because every SELECT already produces exactly these column names, in this set:
`post_id, user_id, username, title, slug, body, format, rendered_html, created_at, updated_at, published_at, deleted_at, summary, tags`
(verified: all 21 sites share this list, including the two `… RETURNING` paths,
whose `(SELECT username …) AS username` / `… AS tags` produce the same column
names; `helpers.rs`'s conceptual `author_username`/`tags_json` map to the DB
column names `username`/`tags`, so the struct fields are named for the
**columns**, not the record). No SELECT needs an alias. The risk a by-name
switch introduces — a field whose name matches no column silently fails to
decode — is caught at the query boundary as a `ColumnDecode` error by the
existing dual-backend post-CRUD tests, which exercise every SELECT path.

## Acceptance criteria (observable)

1. **One row type.** `PostRow` and `PostRecordParts` no longer both exist;
   exactly one post-row type in `storage/src/helpers.rs`. _Verify:_
   `rg -n 'PostRecordParts' storage/src` is empty; `PostRow` is a
   `#[derive(sqlx::FromRow)]` struct.
2. **A field-shape change edits one place.** `build_post_record` and the
   `query_as` target are the same type; no second parallel alias.
3. **Queries + call sites unchanged.** The 21 `query_as::<_, PostRow>` sites,
   their SELECTs, and `post_record_from_row` are untouched. _Verify:_
   `git diff wt-base-issue-616 -- storage/src/posts.rs storage/src/sqlite/posts.rs storage/src/postgres/posts.rs`
   is **empty**.
4. **Behaviour unchanged.** Post read paths return identical `PostRecord`s; the
   malformed-column decode still surfaces as `ColumnDecode`. _Verify:_ the
   dual-backend post-CRUD + `build_post_record` + malformed-column tests stay
   green (updated only to construct a `PostRow` struct instead of a tuple).
5. **No wire/DB/API change; minimal blast radius.** Storage-internal only; no
   query semantics, wire, or `PostRecord` change. Because `PostRow` and
   `post_record_from_row` keep their names, the entire diff lands in
   **`storage/src/helpers.rs`** alone. _Verify:_
   `git diff wt-base-issue-616 --stat` lists only `storage/src/helpers.rs`.
6. **Gate green.** `cargo xtask validate --no-e2e` (static + clippy +
   dual-backend coverage) passes.

## Out of scope / non-goals

- The **19 duplicated SELECT column lists** in `posts.rs` are a _separate_
  pre-existing duplication (a shared column-list const would collapse them). Not
  this issue — Option B leaves them untouched, and folding them in would be
  scope creep.
- No change to the `PostRecord` type, the query SQL, wire formats, or behaviour.
- No ADR — no novel architectural decision; this is a local `storage` cleanup.
