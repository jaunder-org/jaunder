# #694 Typed Post Seams Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Replace the remaining issue-listed primitive Post
title/body/summary/id/tag/slug helper seams with existing domain newtypes,
without changing behavior or wire/storage shapes.

**Architecture:** Keep parsing at real external boundaries and explicit
test-literal helpers. Interior service inputs, render views, ETag helpers, and
fixture builders carry `PostTitle`, `PostBody`, `PostSummary`, `PostId`,
`TagId`, `Slug`, `Tag`, and `TagLabel`. The change is a clean cutover: update
signatures first, migrate all call sites, then remove primitive helper seams and
stale transposition comments.

**Tech Stack:** Rust workspace crates `common`, `storage`, `server`, and `web`;
existing ADR-0063 newtype trailers; `cargo nextest`; `cargo xtask check` via
`devtool run`.

**Review header**

- **Scope in:** `common::render::derive_post_naming`, storage service input/test
  helpers, web `PostView` construction, AtomPub mapping/posts test helpers and
  ETag content, JSON Feed test helpers, all direct call sites.
- **Scope out:** new domain types, storage/API/wire-shape changes, new xtask
  adoption gates (#697), unrelated primitive hazards not named by #694.
- **Task list:** Task 1 common naming seam; Task 2 storage service inputs and
  helpers; Task 3 web render view; Task 4 AtomPub mapping/posts seams; Task 5
  JSON Feed helper cleanup and final gate.
- **Key risks/decisions:** blank explicit titles remain absence at boundaries;
  ETag JSON shape must stay stringly identical; no `From<&str>` shims; service
  `title` fields are borrowed typed titles (`Option<&PostTitle>`) and every
  caller must migrate cleanly.

**Global Constraints**

- Spec:
  `docs/superpowers/specs/2026-08-20-issue-694-post-title-body-summary.md`.
- Preserve title presence semantics: omitted/blank explicit title still behaves
  as absent.
- Preserve body canonicalization/title extraction rules from ADR-0105 and
  summary rules from ADR-0101.
- Preserve rendered HTML, AtomPub XML, JSON Feed output, ETag semantics, API
  payloads, and storage rows.
- Do not add primitive shims, deprecated aliases, or blanket `From<&str>`
  convenience paths.
- Use existing test parse helpers for literals: `parse_post_title`,
  `parse_post_body`, `parse_post_summary`, `parse_slug`, or local equivalents.
- Before each kept commit: tick the task checkbox, run
  `devtool run -- cargo xtask check`, inspect/stage any mechanical fixes, then
  commit with no `Co-Authored-By` trailer.

---

### Task 1: Type the common naming seam

**Files:**

- Modify: `common/src/render.rs:584-626`
- Test: `common/src/render.rs` in-file tests around `derive_post_naming`

**Interfaces:**

- Produces:
  `pub fn derive_post_naming(explicit_title: Option<&PostTitle>, body: &PostBody, format: &PostFormat) -> (Option<PostTitle>, Slug)`.
- Consumes: existing `PostTitle`, `PostBody`, `PostFormat`, and `Slug` newtypes.
- Later tasks rely on this exact signature from `storage/src/post_service.rs`.

- [x] **Step 1: Update the common tests to express the typed boundary**

  In `common/src/render.rs` tests, change the `naming` helper to parse an
  optional title literal before calling `derive_post_naming`:

  ```rust
  fn naming(
      explicit_title: Option<&str>,
      body: &str,
      format: PostFormat,
  ) -> (Option<PostTitle>, Slug) {
      let explicit_title = explicit_title.and_then(|title| title.parse::<PostTitle>().ok());
      derive_post_naming(
          explicit_title.as_ref(),
          &crate::test_support::parse_post_body(body),
          &format,
      )
  }
  ```

  Keep the existing test
  `derive_post_naming_treats_blank_explicit_title_as_absent` by making the
  helper own the parse-or-None boundary; this pins the unchanged external
  behavior while the production function receives only typed titles.

- [x] **Step 2: Run the common naming tests and verify the typed signature is
      not implemented yet**

  Run: `devtool run -- cargo nextest run -p common derive_post_naming`

  Expected: FAIL to compile with a type mismatch at the call that passes
  `explicit_title.as_ref()`, because the production signature still expects
  `Option<&str>`.

- [x] **Step 3: Implement the typed naming signature**

  Change `derive_post_naming` to take `explicit_title: Option<&PostTitle>`.
  Remove the explicit-title trim/filter/parse branch for the supplied title,
  because blank `PostTitle` is unrepresentable. Keep Markdown/Org
  extracted-title parsing inside the function.

  Required implementation shape:

  ```rust
  pub fn derive_post_naming(
      explicit_title: Option<&PostTitle>,
      body: &PostBody,
      format: &PostFormat,
  ) -> (Option<PostTitle>, Slug) {
      let trimmed = body.trim();
      let derived_title = || match format {
          PostFormat::Markdown => extract_markdown_title(trimmed).map(|(title, _)| title),
          PostFormat::Org => extract_org_title(trimmed).map(|(title, _)| title),
          PostFormat::Html => None,
      };
      let title = explicit_title.cloned().or_else(|| {
          derived_title().and_then(|title| title.parse::<PostTitle>().ok())
      });
      let seed = match title.as_ref() {
          Some(title) => title.to_string(),
          None => first_meaningful_line(body),
      };
      let Ok(slug) = slugify_title(&seed).parse::<Slug>() else {
          unreachable!("slugify_title's output always re-parses as a Slug")
      };
      (title, slug)
  }
  ```

  Preserve the existing `slugify_title` / `Slug` fallback code after `seed`; do
  not change slug behavior.

- [x] **Step 4: Run the common naming tests and verify they pass**

  Run: `devtool run -- cargo nextest run -p common derive_post_naming`

  Expected: PASS.

- [x] **Step 5: Commit Task 1**

  Run: `devtool run -- cargo xtask check`

  Stage `common/src/render.rs` and any formatter changes. Commit:

  devtool run -- git commit -m "types(post): type explicit title naming seam"

### Task 2: Type storage service inputs and service test helpers

**Files:**

- Modify: `storage/src/post_service.rs:200-284,364-479,1129-1261`
- Modify: `storage/src/test_support.rs:955-1024,1463-1525,1650-1666`
- Test: `storage/src/post_service.rs` in-file tests
- Test: `storage/src/test_support.rs` in-file tests

**Interfaces:**

- Consumes: Task 1
  `derive_post_naming(Option<&PostTitle>, &PostBody, &PostFormat)`.
- Produces: `PostUpdate<'a>` and `PostCreation<'a>` with
  `pub title: Option<&'a PostTitle>` and all non-title fields unchanged.
- Produces:
  `creation_with_key(user_id: UserId, body: PostBody, key: Option<&str>) -> PostCreation<'_>`
  in `post_service` tests.
- Produces: `SeedPost` with `title: Option<PostTitle>`, `body: PostBody`,
  unchanged `user_id` / `audiences`, plus
  `SeedPost::title(self, title: PostTitle) -> Self`, and `storage::test_support`
  service helpers accepting `PostBody` for body parameters.

- [x] **Step 1: Update storage tests and helpers to call typed service inputs**

  In `storage/src/post_service.rs`, change `creation_with_key` to:

  ```rust
  fn creation_with_key<'a>(
      user_id: UserId,
      body: PostBody,
      key: Option<&'a str>,
  ) -> PostCreation<'a> {
      PostCreation {
          user_id,
          body,
          title: None,
          format: PostFormat::Markdown,
          slug_override: None,
          published_at: Some(Utc::now()),
          max_attempts: 100,
          summary: None,
          audiences: vec![AudienceTarget::Public],
          idempotency_key: key,
      }
  }
  ```

  Update its callers to pass concrete parsed fixtures such as
  `parse_post_body("First body")` instead of a body `&str`.

  In `storage/src/test_support.rs`, change `SeedPost::title` to accept
  `PostTitle`, store `Option<PostTitle>`, and call `title: self.title.as_ref()`
  if `PostCreation` stays borrowed. Change `create_post_via_service`,
  `create_draft_via_service`, private `create_via_service`, and
  `update_post_body_via_service` to accept `PostBody` and remove their internal
  body parsing. Update direct test callers with concrete parsed fixtures such as
  `parse_post_body("Custom body text")`.

- [x] **Step 2: Run targeted storage tests and verify expected compile
      failures**

  Run both:
  - `devtool run -- cargo nextest run -p storage creation_with_key`
  - `devtool run -- cargo nextest run -p storage seed_post_builder_setters_apply`

  Expected: FAIL to compile until `PostCreation` / `PostUpdate` title signatures
  and all call sites are migrated.

- [x] **Step 3: Implement typed `PostCreation` and `PostUpdate`**

  Change `PostUpdate.title` and `PostCreation.title` to borrowed typed title
  references. Delete the doc comment sentences at `PostUpdate` and
  `PostCreation` that say struct naming mitigates the `title` / `slug_override`
  transposition. Update `perform_post_update` and `perform_post_creation` to
  pass typed titles into `derive_post_naming` and typed titles into
  `UpdatePostInput` / `RenderedPostContent`.

  If using borrowed titles, required forwarding shape:

  ```rust
  let (title, derived_slug) = derive_post_naming(title, &body, &format);
  let input = UpdatePostInput {
      title,
      slug,
      body,
      format,
      rendered,
      unpublish,
      explicit_published_at,
      summary,
      audiences,
  };
  ```

  `title` after `derive_post_naming` remains `Option<PostTitle>` because
  `UpdatePostInput` and `RenderedPostContent` require owned typed values.

- [x] **Step 4: Migrate storage call sites and rerun targeted tests**

  Update all `PostCreation` and `PostUpdate` struct literals in `storage/src` to
  pass typed title values or `None`. For title literals, use concrete parsed
  fixtures such as `parse_post_title("Custom Title")` and `.as_ref()` if needed.

  Run both:
  - `devtool run -- cargo nextest run -p storage creation_with_key`
  - `devtool run -- cargo nextest run -p storage seed_post_builder_setters_apply`

  Expected: PASS.

- [x] **Step 5: Commit Task 2**

  Run: `devtool run -- cargo xtask check`

  Stage `storage/src/post_service.rs`, `storage/src/test_support.rs`, and any
  formatter changes. Commit:

  devtool run -- git commit -m "types(post): type storage post service inputs"

### Task 3: Type web render view seams

**Files:**

- Modify: `web/src/posts/render.rs:101-150,157-205,346-607`
- Modify: `web/src/posts/component.rs` `PostView` construction sites
- Test: `web/src/posts/render.rs` in-file tests

**Interfaces:**

- Consumes: existing `RenderedPost.title: Option<PostTitle>` and
  `RenderedPost.summary: Option<PostSummary>`.
- Produces: `pub(crate) struct PostView<'a>` with
  `pub title: Option<&'a PostTitle>`, `pub summary: Option<&'a PostSummary>`,
  and all non-title/summary fields unchanged.

- [x] **Step 1: Update render tests to construct typed titles and summaries**

  In `web/src/posts/render.rs` tests, replace `Some("T")` title fixtures with a
  local typed value:

  ```rust
  let title = common::test_support::parse_post_title("T");
  let summary = common::test_support::parse_post_summary("Summary");
  let view = PostView {
      title: Some(&title),
      summary: Some(&summary),
      // keep username, banner, rendered_html, time, permalink, tags, and tag_ctx as in the existing fixture
  };
  ```

  Tests with no title/summary continue to use `None`.

- [x] **Step 2: Run web render tests and verify expected compile failures**

  Run: `devtool run -- cargo nextest run -p web posts::render`

  Expected: FAIL to compile until `PostView` and construction sites use typed
  references.

- [x] **Step 3: Implement typed `PostView` fields and migrate constructors**

  Change `PostView.title` and `PostView.summary` to typed references. Update
  `permalink_article`, `render_posts`, and `web/src/posts/component.rs` to use
  `.as_ref()` instead of `.as_deref()`. Leave markup expressions unchanged
  except where the compiler requires `as_ref()`/`Deref` coercion; `PostTitle`
  and `PostSummary` implement `Display`/`Deref<str>` through ADR-0063.

- [x] **Step 4: Run web render tests and verify unchanged rendering**

  Run: `devtool run -- cargo nextest run -p web posts::render`

  Expected: PASS.

- [x] **Step 5: Commit Task 3**

  Run: `devtool run -- cargo xtask check`

  Stage `web/src/posts/render.rs`, `web/src/posts/component.rs`, and any
  formatter changes. Commit:

  devtool run -- git commit -m "types(post): type web post view text fields"

### Task 4: Type AtomPub mapping, handler, and ETag helper seams

**Files:**

- Modify: `server/src/atompub/mapping.rs:551-650` and affected tests
- Modify:
  `server/src/atompub/posts.rs:73-93, collection_post/member_put PostCreation/PostUpdate calls, 515-604`
- Test: `server/src/atompub/mapping.rs` in-file tests
- Test: `server/src/atompub/posts.rs` in-file tests

**Interfaces:**

- Consumes: Task 2 typed `PostCreation` / `PostUpdate` title fields.
- Produces: `MakePost` with typed `post_id: PostId`, `title: Option<PostTitle>`,
  `slug: Slug`, `body: PostBody`, `summary: Option<PostSummary>`,
  `tags: Vec<(Tag, TagLabel)>`, and unchanged `format` / `published_at`.
- Produces:
  `fn mk_tag(post_id: PostId, tag_id: TagId, slug: Tag, display: TagLabel) -> PostTag`.
- Produces: `EtagContent<'a>` with typed `title: Option<&'a PostTitle>`,
  `body: &'a PostBody`, `summary: Option<&'a PostSummary>`,
  `tags: Vec<&'a TagLabel>`, and unchanged `format` / `draft`.

- [x] **Step 1: Update AtomPub tests and builders to typed fixtures**

  In `server/src/atompub/mapping.rs`, update `MakePost` and `make_post` callers
  to parse literals before constructing the builder:

  ```rust
  MakePost {
      post_id: PostId::from(1),
      title: Some(parse_post_title("Title")),
      slug: parse_slug("my-post"),
      body: parse_post_body("Body"),
      format: PostFormat::Markdown,
      published_at: None,
      summary: Some(parse_post_summary("Summary")),
      tags: vec![("rust".parse::<Tag>().unwrap(), "Rust".parse::<TagLabel>().unwrap())],
  }
  ```

  In `server/src/atompub/posts.rs`, update `mk_tag` callers to pass concrete
  typed fixtures such as `PostId::from(1)`, `TagId::from(1)`, parsed `Tag`, and
  parsed `TagLabel`.

- [x] **Step 2: Run AtomPub targeted tests and verify expected compile
      failures**

  Run both:
  - `devtool run -- cargo nextest run -p server atompub::mapping`
  - `devtool run -- cargo nextest run -p server etag_tests`

  Expected: FAIL to compile until builder/helper signatures and handler call
  sites are migrated.

- [x] **Step 3: Implement typed AtomPub helpers and handler forwarding**

  Change `MakePost` and `mk_tag` signatures to the typed interfaces above.
  Update `collection_post` and `member_put` to pass typed titles from
  `PostFields` into `PostCreation` / `PostUpdate` without converting through
  `as_deref()`. Borrow with `.as_ref()` or move/clone consistently with Task 2's
  chosen service input shape.

  Change `EtagContent` fields to typed references and construct it with:

  ```rust
  title: post.title.as_ref(),
  body: &post.body,
  summary: post.summary.as_ref(),
  tags: post.tags.iter().map(|t| &t.tag_display).collect(),
  ```

  Do not change which fields contribute to the ETag.

- [x] **Step 4: Run AtomPub targeted tests and verify unchanged semantics**

  Run both:
  - `devtool run -- cargo nextest run -p server atompub::mapping`
  - `devtool run -- cargo nextest run -p server etag_tests`

  Expected: PASS. Existing ETag tests must still show
  identity/timestamp/rendered_html changes are ignored and
  title/body/summary/format/tag/draft changes alter the ETag.

- [x] **Step 5: Commit Task 4**

  Run: `devtool run -- cargo xtask check`

  Stage `server/src/atompub/mapping.rs`, `server/src/atompub/posts.rs`, and any
  formatter changes. Commit:

  devtool run -- git commit -m "types(post): type atompub post helper seams"

### Task 5: Type JSON Feed helpers and run the final branch gate

**Files:**

- Modify: `common/src/feed/json.rs:69-84` and direct test callers
- Possibly modify: any remaining direct call site found by compiler after Tasks
  1-4
- Test: `common/src/feed/json.rs` in-file tests
- Plan/spec tracking:
  `docs/superpowers/plans/2026-08-20-issue-694-post-title-body-summary.md`

**Interfaces:**

- Consumes: existing `FeedItem` with typed `title: Option<PostTitle>`,
  `summary: Option<PostSummary>`, `tags: Vec<TagLabel>`, and unchanged
  id/permalink/content/time fields.
- Produces: `fn item(title: Option<PostTitle>, tags: Vec<&str>) -> FeedItem` and
  `fn item_with_summary(title: Option<PostTitle>, tags: Vec<&str>, summary: Option<PostSummary>) -> FeedItem`.

- [x] **Step 1: Update JSON Feed helper tests to typed title/summary inputs**

  Change helper signatures so title/summary are not `Option<&str>`. Update
  callers to use concrete parsed fixtures such as
  `parse_post_title("JSON Title")` and `parse_post_summary("JSON Summary")` at
  the literal boundary. Keep `tags: Vec<&str>` unless the implementation is
  already editing a same-signature tag hazard in this helper; #694 only requires
  title/summary for `common/src/feed/json.rs`.

- [x] **Step 2: Run JSON Feed tests and verify the helper migration**

  Run: `devtool run -- cargo nextest run -p common feed::json`

  Expected: PASS after helper migration. JSON output assertions remain
  unchanged.

- [x] **Step 3: Search for remaining issue-listed primitive seams**

  Use code search or compiler fallout, not a broad rewrite. Required no-match
  checks:
  - `PostCreation` / `PostUpdate` title fields are no longer `Option<&str>`.
  - `PostView.title` / `PostView.summary` are no longer `Option<&str>`.
  - `EtagContent` no longer has title/body/summary `&str` fields.
  - `MakePost`, `SeedPost`, `creation_with_key`, and `mk_tag` no longer expose
    the primitive parameters named in #694.

  Use `grep`/`read` tools for the source checks; do not use shell `grep`.

- [x] **Step 4: Run the final local gate**

  Run: `devtool run -- cargo xtask check`

  Expected: PASS. If the gate formats files, stage those exact files and rerun
  the same gate before committing.

- [x] **Step 5: Commit Task 5 and the completed plan checkbox**

  Tick this task checkbox before staging. Stage `common/src/feed/json.rs`,
  `docs/superpowers/specs/2026-08-20-issue-694-post-title-body-summary.md`,
  `docs/superpowers/plans/2026-08-20-issue-694-post-title-body-summary.md`, and
  any final mechanically formatted files. Commit:

  devtool run -- git commit -m "types(post): finish typed post helper seams"
