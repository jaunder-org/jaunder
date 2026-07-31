# post→media reference table — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-07-30-issue-711-post-media-table.md` —
the "what/why". This plan is the "how" and does not restate it. Decisions are
cited as D1–D9, criteria as A1–A21. **ADR draft:**
`docs/adr/drafts/media-references-extracted-at-render.md` (numberless;
`cargo xtask adr promote` numbers it at ship). **Issue:**
[#711](https://github.com/jaunder-org/jaunder/issues/711)

**Goal:** Record which posts reference which media in a `post_media` table
written at render time, so the media-delete guard becomes an exact, untruncated,
atomic lookup.

**Architecture:** Extraction is a pure `&str → Vec<MediaRef>` walk over
ammonia's sanitized output, driven by a declarative `(element, attribute)`
table. `RenderOutput` binds HTML and references together with rendering as its
only constructor, so the pair cannot desynchronise; publication moves off the
update path first, so every remaining writer renders. The delete guard becomes
one conditional `DELETE … RETURNING`.

**Tech Stack:** Rust, sqlx 0.8 (SQLite + PostgreSQL), ammonia 4.1, html5ever
0.39, Leptos server fns, Playwright e2e.

## Global Constraints

- **Backend parity (ADR-0053):** every storage and `server` integration test is
  dual-backend —
  `#[apply(backends)] #[tokio::test] async fn …(#[case] backend: Backend)`. A
  bare `#[tokio::test]` that should be dual-backend fails the
  `test-backend-pattern` guard.
- **ADR-0019:** no `#[cfg(test)]` tests inside `storage/src/{sqlite,postgres}/*`
  dialect files. Dialect behaviour is tested through the generic store.
- **Reuse the existing fixtures.** `storage/src/test_support.rs` is
  builder-shaped — `SeedUser` (`:696`), `seed_users::<N>` (`:772`), `SeedPost`
  (`:796`), `SeedRawPost` (`:901`, with `.build() -> CreatePostInput` at
  `:1003`, `.create()`, `.seed()`), `seed_posts` (`:645`), and
  `TestBase::execute` / `scalar_i64` (`:90`, `:109`) for raw SQL. New helpers
  extend those builders; do not add a parallel set of flat functions.
- **Per-commit gate:** run `devtool run -- cargo xtask check` before every
  commit; the pre-commit hook runs it too and **fails on reformat rather than
  folding it in**, so re-stage if it reformats (`jaunder-commit`).
- **No `Co-Authored-By` trailer** on any commit.
- **Migrations are per-dialect** — `sqlite/` uses `INTEGER`, `postgres/` uses
  `BIGINT` and must declare `DEFERRABLE INITIALLY IMMEDIATE` (D7). `git add`
  them as soon as they exist: Nix flakes ignore untracked files, so an unstaged
  migration is invisible to the gate.
- **New dep pins to a version already in `Cargo.lock`** (`html5ever` `0.39`) so
  the Nix vendor is reused rather than rebuilt.
- **Worktree:**
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-711-post-media-table`. Run
  everything from there.

## Scope

**In:** the `post_media` table and its migrations; `MediaRef` + URL parser; the
HTML extractor and its coupling test; `RenderOutput`; `publish_post`;
`try_delete_media`; the force-delete affordance; rewiring `web::media::delete`;
dual-backend and e2e coverage.

**Out:** widening the sanitiser for `<video>`/`<audio>` (Task 1 files it);
host-aware absolute-URL matching (Task 1 files it); orphan reclamation; any
backfill (D9 — no production users).

## Tasks

| #   | Task                                                   | Criteria established              |
| --- | ------------------------------------------------------ | --------------------------------- |
| 1   | File the two follow-up issues                          | —                                 |
| 2   | `MediaRef` + URL parser in `common::media`             | A1, A5, A6, A7                    |
| 3   | HTML extractor + coupling test in `common::render`     | A4, A8, A9, A9b, A9c              |
| 4   | `publish_post` — take publication off the update path  | A15b (fields), A15c               |
| 5   | `RenderOutput` + thread through the post inputs        | A10                               |
| 6   | `post_media` migrations + write path + shared fixtures | A2, A3, A11–A14, A15, A15b (rows) |
| 7   | `list_posts_referencing_media` read side               | A16, A17                          |
| 8   | `try_delete_media` atomic guard + force affordance     | A17b–A17e                         |
| 9   | e2e for the refuse/force flow                          | A18, A19, A21                     |
| 10  | Backup golden list                                     | A20                               |

**Ordering note — why Task 4 precedes Task 5.** Task 5 replaces
`CreatePostInput`/`UpdatePostInput`'s `rendered_html` field with
`rendered: RenderOutput`. The one production site that cannot supply a
`RenderOutput` is `web/src/posts/api.rs:511-521` (`publish`, which passes
`existing.rendered_html`) — the very site D1 exists to remove. So publication
must move off the update path _first_, or Task 5 leaves a tree that does not
compile.

**Key risks:**

- **Task 8 / A17e** is the one genuine unknown:
  `storage/src/sqlite/sessions.rs:19` records `SQLITE_BUSY` from `RETURNING`
  with a correlated subquery. Our shape differs (subquery in `WHERE`, returning
  only `media`'s own column) but is unverified. The task carries the concurrency
  exercise _and_ the fallback.
- **Task 5** is a wide compiler-guided churn across ~20 enumerated sites.
  Mechanical, but do not leave a partial sweep.
- **Task 8** carries real production UI work: there is currently **no
  force-delete control**, though `web/src/media/component.rs:264` already tells
  users to use one.
- **Task 3's** coupling test must be _demonstrated to bite_ (A9), not merely
  observed to pass.

---

### Task 1: File the two follow-up issues

Separable concerns surfaced during the design interview. Filed first so they can
be picked up concurrently rather than blocked behind this cycle.

**Files:** none — tracker only.

**Interfaces:**

- Consumes: nothing.
- Produces: two issue numbers, cited in the spec's "Follow-up issues" section.

- [x] **Step 1: File the sanitiser-widening issue** —
      [#743](https://github.com/jaunder-org/jaunder/issues/743)

Use `jaunder-issues`. Title:
`media: allow <video>/<audio> embeds in post bodies`. Type Feature, label `web`.
The body must carry the two facts the spec records, so whoever takes it does not
rediscover them: `ammonia-4.1.4/src/lib.rs:2531` treats `src` as a URL attribute
on _any_ element and `video[poster]` explicitly, so scheme filtering is free;
`srcset` is **not** in that list, so permitting it would admit an unfiltered URL
attribute _and_ does not fit the extractor's one-attribute-one-URL table. Note
that #711's coupling test forces every newly permitted pair to be classified
before it can land, and that `<source>` (format alternatives) and `<track>`
(WebVTT captions) belong with it.

- [x] **Step 2: File the host-aware-matching issue** —
      [#744](https://github.com/jaunder-org/jaunder/issues/744)

Title: `media: match absolute media URLs against the configured site host`. Type
Feature, label `web`. Body: #711 matches on path alone and ignores
scheme/host/port to keep `render` pure and config-free (spec D4); this issue
revisits that, and must address that `rendered_html` is stored, so a hostname
change would invalidate previously extracted references.

- [x] **Step 3: Record the numbers in the spec**

Replace the two "Follow-up issues" entries' prose with `#<N> — <title>` links.

- [x] **Step 4: Commit** — `8870292a`

```bash
git add docs/superpowers/specs/2026-07-30-issue-711-post-media-table.md
git commit -m "docs(spec): link #711's follow-up issues"
```

---

### Task 2: `MediaRef` and the URL parser

The inverse of `media_url`, beside it, so the two definitions of the layout
cannot drift (D6). Pure string work, ungated — no HTML, no `sanitize`.

**Files:**

- Modify: `common/src/media.rs` (beside `media_path` `:629` / `media_url`
  `:647`)

**Interfaces:**

- Consumes: `MediaSource`, `ContentHash`, `Filename`, `ProfferedFilename`,
  `media_url`, `media_path` — all already in `common::media`. Test helpers
  `parse_filename` and `parse_content_hash` live in `common/src/test_support.rs`
  (`:295`, `:283`); the shared `CANONICAL` digest constant is in
  `common/src/media.rs`'s test module (`:1224`).
- Produces:

```rust
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaRef {
    pub source: MediaSource,
    pub sha256: ContentHash,
    pub filename: Filename,
}

/// Parses a media URL's path into the triple it names, or `None` if the path is not one
/// of jaunder's two media layouts. Scheme, host, port, query and fragment are ignored
/// (spec D4).
#[must_use]
pub fn parse_media_url(url: &str) -> Option<MediaRef>;
```

- [x] **Step 1: Write the failing tests**

Add to `common/src/media.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn parse_media_url_round_trips_every_source_and_encoded_names() {
    // A1: the parser is the exact inverse of the formatter.
    for source in [MediaSource::Upload, MediaSource::Cached] {
        for raw in ["photo.jpg", "my photo.jpg", "ünïcode nàme.png", "100%.jpg"] {
            let filename: Filename = raw
                .parse::<ProfferedFilename>()
                .expect("a legal filename")
                .into();
            let hash = parse_content_hash(CANONICAL);
            let url = media_url(&source, &hash, &filename);
            assert_eq!(
                parse_media_url(&url),
                Some(MediaRef {
                    source,
                    sha256: hash.clone(),
                    filename: filename.clone()
                }),
                "round trip failed for {source:?} / {raw}"
            );
        }
    }
}

#[test]
fn parse_media_url_accepts_the_atompub_member_layout_as_upload() {
    // A3 (parser half): the member URL carries no source; it pins Upload.
    let url = format!("/atompub/alice/media/{CANONICAL}/photo.jpg");
    assert_eq!(
        parse_media_url(&url),
        Some(MediaRef {
            source: MediaSource::Upload,
            sha256: parse_content_hash(CANONICAL),
            filename: parse_filename("photo.jpg"),
        })
    );
}

#[test]
fn parse_media_url_canonicalises_a_raw_filename_to_the_stored_spelling() {
    // A2 (parser half): raw and encoded spellings converge (spec D5).
    let raw = format!("/media/upload/e3/b0/{CANONICAL}/my photo.jpg");
    let encoded = format!("/media/upload/e3/b0/{CANONICAL}/my%20photo.jpg");
    assert_eq!(parse_media_url(&raw), parse_media_url(&encoded));
    assert_eq!(
        parse_media_url(&raw).expect("raw spelling parses").filename.as_ref(),
        "my%20photo.jpg"
    );
}

#[test]
fn parse_media_url_rejects_a_prefix_that_does_not_match_the_hash() {
    // A5: server/src/media.rs:236 404s these, so they name nothing.
    assert_eq!(parse_media_url(&format!("/media/upload/ff/b0/{CANONICAL}/photo.jpg")), None);
    assert_eq!(parse_media_url(&format!("/media/upload/e3/ff/{CANONICAL}/photo.jpg")), None);
}

#[test]
fn parse_media_url_strips_query_and_fragment() {
    // A6.
    let base = format!("/media/upload/e3/b0/{CANONICAL}/photo.jpg");
    let expected = parse_media_url(&base);
    assert!(expected.is_some());
    assert_eq!(parse_media_url(&format!("{base}?v=2")), expected);
    assert_eq!(parse_media_url(&format!("{base}#frag")), expected);
    assert_eq!(parse_media_url(&format!("{base}?v=2#frag")), expected);
}

#[test]
fn parse_media_url_is_host_blind() {
    // A7: deliberate — see spec D4. Pinned so nobody "fixes" it silently.
    let path = format!("/media/upload/e3/b0/{CANONICAL}/photo.jpg");
    let expected = parse_media_url(&path);
    assert!(expected.is_some());
    assert_eq!(parse_media_url(&format!("https://elsewhere.example{path}")), expected);
    assert_eq!(parse_media_url(&format!("http://localhost:3000{path}")), expected);
}

#[test]
fn parse_media_url_rejects_non_media_paths() {
    assert_eq!(parse_media_url("/posts/hello"), None);
    assert_eq!(parse_media_url("/media/upload/e3/b0/short/photo.jpg"), None);
    assert_eq!(
        parse_media_url(&format!("/media/bogus-source/e3/b0/{CANONICAL}/photo.jpg")),
        None,
        "an unknown source token is not a media URL"
    );
    assert_eq!(parse_media_url(""), None);
    assert_eq!(parse_media_url("/media/upload/e3/b0"), None);
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p common parse_media_url` Expected: FAIL
— `parse_media_url` / `MediaRef` not defined.

- [x] **Step 3: Implement against the tests**

Write `MediaRef` and `parse_media_url` to the signatures above. Every branch is
pinned: both layouts, prefix mismatch, query/fragment, host-blindness,
short/non-hex hash, unknown source, and the raw→canonical filename convergence.
Two constraints the tests cannot state:

- The filename segment goes through **percent-decode then `ProfferedFilename`**
  — the same door `server/src/media.rs:418` uses — never a bespoke transform
  (D5). `ProfferedFilename` is `FromStr` (`common/src/media.rs:384`), so
  `.parse()`, not `From`.
- Derive the layout by _parsing_ what `media_path` composes; do not re-spell it
  as a literal, per `media_path`'s "single definition" doc comment.

- [x] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p common parse_media_url` Expected: PASS

- [x] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/media.rs
git commit -m "feat(media): parse a media URL back into the triple it names (#711)"
```

---

### Task 3: The HTML extractor and its coupling test

**Files:**

- Modify: `common/src/render.rs` (beside `SANITIZER` `:119` and `render` `:341`)
- Modify: `common/Cargo.toml`, `Cargo.toml` (workspace dep)

**Interfaces:**

- Consumes: `common::media::{MediaRef, parse_media_url}` (Task 2).
- Produces:

```rust
/// The (element, attribute) pairs whose values name media. Adding an element to
/// `SANITIZER` means adding its URL-bearing attributes here — the walk knows no tag
/// names of its own, so extending to `<video>`/`<audio>` is a data edit.
pub(crate) const MEDIA_URL_ATTRS: &[(&str, &str)] = &[("a", "href"), ("img", "src")];

/// Permitted pairs deliberately *not* treated as media references. Present so
/// `sanitizer_surface_is_fully_classified` can tell "considered and excluded" from
/// "nobody looked".
pub(crate) const KNOWN_INERT_ATTRS: &[(&str, &str)] = &[/* … */];

/// Extracts the media a sanitized HTML fragment references, deduplicated and sorted.
#[cfg(feature = "sanitize")]
#[must_use]
pub fn extract_media_refs(html: &str) -> Vec<MediaRef>;

/// Table-driven core of `extract_media_refs`; separate so a test can drive it with a
/// synthetic pair table and prove no tag name is baked in (A9b).
#[cfg(feature = "sanitize")]
fn extract_media_refs_with(html: &str, pairs: &[(&str, &str)]) -> Vec<MediaRef>;
```

- [x] **Step 1: Add the dependency**

`Cargo.toml` workspace deps: `html5ever = "0.39"` (the version already in
`Cargo.lock` via ammonia, so the Nix vendor is reused). `common/Cargo.toml`:
`html5ever = { workspace = true, optional = true }`, and extend
`sanitize = ["dep:ammonia", "dep:html5ever"]`.

- [x] **Step 2: Write the failing tests**

Add to `common/src/render.rs`'s `#[cfg(test)] mod tests`. `CANONICAL` is defined
in `media.rs`'s test module and is **not** in scope here — add a local
`const CANONICAL: &str = "…";` (same digest) or import it, whichever the crate's
test modules already do for shared constants.

````rust
fn media_url_for(name: &str) -> String {
    format!("/media/upload/e3/b0/{CANONICAL}/{name}")
}

#[test]
fn extract_finds_a_markdown_image() {
    // Rendered via the real renderer, so this pins end-to-end behaviour rather than a
    // hand-written fragment.
    let body: PostBody = format!("![alt]({})", media_url_for("photo.jpg")).into();
    let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].filename.as_ref(), "photo.jpg");
}

#[test]
fn extract_finds_a_raw_img_embedded_in_a_markdown_body() {
    // A4: the rendered-HTML choice — raw HTML passes through the Markdown parser.
    let body: PostBody = format!("<img src=\"{}\">", media_url_for("photo.jpg")).into();
    let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].filename.as_ref(), "photo.jpg");
}

#[test]
fn extract_finds_a_raw_filename_spelling() {
    // The #675 regression, at the extractor level.
    let body: PostBody = format!("<img src=\"{}\">", media_url_for("my photo.jpg")).into();
    let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].filename.as_ref(), "my%20photo.jpg");
}

#[test]
fn extract_finds_an_atompub_member_url_in_a_link() {
    let body: PostBody =
        format!("<a href=\"/atompub/alice/media/{CANONICAL}/photo.jpg\">doc</a>").into();
    let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].source, MediaSource::Upload);
}

#[test]
fn extract_ignores_media_in_stripped_elements_and_code_blocks() {
    // A8: sanitisation removes <video>, so it can never load and is not a reference.
    let video: PostBody = format!("<video src=\"{}\"></video>", media_url_for("clip.mp4")).into();
    assert!(extract_media_refs(render(&video, &PostFormat::Markdown).as_ref()).is_empty());

    let fenced: PostBody = format!("```\n{}\n```", media_url_for("photo.jpg")).into();
    assert!(extract_media_refs(render(&fenced, &PostFormat::Markdown).as_ref()).is_empty());
}

#[test]
fn extract_deduplicates_and_sorts() {
    let one = media_url_for("a.jpg");
    let two = media_url_for("b.jpg");
    let body: PostBody =
        format!("<img src=\"{two}\"><img src=\"{one}\"><img src=\"{one}\">").into();
    let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
    assert_eq!(refs.len(), 2, "duplicate references collapse to one row");
    assert!(refs[0] < refs[1], "output is sorted for deterministic writes");
}

#[test]
fn extract_ignores_non_media_links() {
    let body: PostBody = "<a href=\"https://example.com/page\">x</a>".to_owned().into();
    assert!(extract_media_refs(render(&body, &PostFormat::Markdown).as_ref()).is_empty());
}

#[test]
fn extract_walk_is_table_driven_not_tag_hardcoded() {
    // A9b: drive the walk with a pair absent from MEDIA_URL_ATTRS. This fails if any tag
    // name is baked into the walk, which is what makes "adding <video> is a data edit" a
    // checked claim rather than a hope.
    let html = format!("<span data-src=\"{}\"></span>", media_url_for("photo.jpg"));
    let refs = extract_media_refs_with(&html, &[("span", "data-src")]);
    assert_eq!(refs.len(), 1);
    assert!(extract_media_refs(&html).is_empty(), "the real table does not pick it up");
}

/// Permitted `(element, attribute)` pairs appearing in neither classification table.
/// The enumeration is `tags × generic_attributes ∪ tag_attributes` — `generic_attributes`
/// applies to every tag, so omitting that product would leave a hole.
fn unclassified_sanitizer_pairs(builder: &ammonia::Builder<'_>) -> Vec<(String, String)> {
    // clone_tags / clone_generic_attributes / clone_tag_attributes are public.
    todo!("implemented in Step 4")
}

#[test]
fn sanitizer_surface_is_fully_classified() {
    // A9.
    let unclassified = unclassified_sanitizer_pairs(&SANITIZER);
    assert!(
        unclassified.is_empty(),
        "SANITIZER permits {unclassified:?}, which appear in neither MEDIA_URL_ATTRS nor \
         KNOWN_INERT_ATTRS. Classify each: add it to MEDIA_URL_ATTRS if its value names \
         media, otherwise to KNOWN_INERT_ATTRS with a reason."
    );
}

#[test]
fn sanitizer_coupling_test_bites_when_the_allowlist_widens() {
    // A9: prove the guard can fail. A widened builder with an unclassified URL-bearing
    // attribute must be reported — otherwise the check above is decorative.
    let mut widened = ammonia::Builder::default();
    widened.add_tags(["video"]);
    widened.add_tag_attributes("video", ["src"]);
    let unclassified = unclassified_sanitizer_pairs(&widened);
    assert!(
        unclassified.contains(&("video".to_owned(), "src".to_owned())),
        "the coupling check must flag a newly permitted, unclassified pair"
    );
}
````

- [x] **Step 3: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p common --features sanitize extract`
Expected: FAIL — `extract_media_refs` not defined.

- [x] **Step 4: Implement against the tests**

`extract_media_refs_with` tokenises with `html5ever`'s **tokenizer** (not the
tree builder — only start tags and their attributes are needed), and for each
start tag whose `(element, attribute)` matches a pair in the table, hands the
value to `parse_media_url`. Collect into a `BTreeSet<MediaRef>` and return its
`Vec`, which delivers dedup and sort in one move. `extract_media_refs` is
`extract_media_refs_with(html, MEDIA_URL_ATTRS)`.

`unclassified_sanitizer_pairs` reads `clone_tags`, `clone_generic_attributes`
and `clone_tag_attributes` off the builder, forms
`tags × generic ∪ tag_attributes`, and subtracts both tables.

Populate `KNOWN_INERT_ATTRS` with every currently permitted pair that isn't
`a[href]`/`img[src]` — including the `cite` attributes on
`blockquote`/`del`/`ins`/`q`, which D2 excludes as a deliberate scope call —
each with a short reason comment.

- [x] **Step 5: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p common --features sanitize` Expected:
PASS

- [x] **Step 6: Write the D3 documentation (A9c)**

Three obligations, all load-bearing:

1. Both tables carry the doc comments given in **Interfaces** above.
2. **`SANITIZER`'s comment (`common/src/render.rs:108-117`) gains the extractor
   obligation.** It already says "Widening this list is a security decision;
   `sanitize_*` tests pin both halves." Append that widening also obliges
   classifying each new attribute into `MEDIA_URL_ATTRS` or `KNOWN_INERT_ATTRS`,
   that `sanitizer_surface_is_fully_classified` enforces it, and that a
   multi-URL attribute such as `srcset` does not fit the table's shape and needs
   it widened first. This is the comment a person widening the allowlist is
   actually reading — the obligation belongs here, not only at the tables.
3. `sanitizer_surface_is_fully_classified`'s doc comment states what a failure
   means and how to resolve it, consistent with the assert message.

- [x] **Step 7: Verify wasm still builds**

`common` compiles for wasm without `sanitize`; the new dep must not leak. Run:
`devtool run -- cargo clippy -p common --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS

- [x] **Step 8: Commit**

```bash
devtool run -- cargo xtask check
git add Cargo.toml Cargo.lock common/Cargo.toml common/src/render.rs
git commit -m "feat(render): extract media references from sanitized HTML (#711)"
```

---

### Task 4: `publish_post` — take publication off the update path

**Must precede Task 5** (see the ordering note above): Task 5 removes the
`rendered_html` field that `publish` currently supplies, and `publish` has no
`RenderOutput` to give. Independently, this is a real bug fix — routing
publication through the full update path is what would let publication clobber a
post's media rows.

**Files:**

- Modify: `storage/src/posts.rs` (`PostStorage` trait, `:586` region; generic
  impl)
- Modify: `storage/src/sqlite/posts.rs`, `storage/src/postgres/posts.rs`
- Modify: `web/src/posts/api.rs:489-537` (`publish`)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces:

```rust
/// Publishes a draft: sets `published_at` to now if it is NULL, leaving an already-
/// published post's timestamp untouched. Changes nothing else — not the body, rendered
/// HTML, format, slug, summary, audiences, or media rows (spec D1).
///
/// # Errors
/// `UpdatePostError::NotFound` if the post does not exist or is soft-deleted;
/// `UpdatePostError::Unauthorized` if `user_id` does not own it.
async fn publish_post(
    &self,
    post_id: PostId,
    user_id: UserId,
) -> Result<PostRecord, UpdatePostError>;
```

- [x] **Step 1: Write the failing tests**

In `storage/src/posts.rs`'s test module, using the existing seeding builders:

```rust
#[apply(backends)]
#[tokio::test]
async fn publish_post_changes_only_the_publication_timestamp(#[case] backend: Backend) {
    // A15b (field half). The media-row half lands in Task 6, once post_media exists.
    let TestEnv { state, .. } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let post = SeedPost { published: false, ..SeedPost::for_user(user) }.seed(&state).await;
    let audiences_before = state.posts.get_post_audiences(post.post_id).await.unwrap();

    let after = state.posts.publish_post(post.post_id, user).await.expect("publish succeeds");

    assert!(after.published_at.is_some(), "the draft is now published");
    assert_eq!(after.body, post.body);
    assert_eq!(after.rendered_html, post.rendered_html);
    assert_eq!(after.format, post.format);
    assert_eq!(after.slug, post.slug);
    assert_eq!(after.title, post.title);
    assert_eq!(after.summary, post.summary);
    assert_eq!(
        state.posts.get_post_audiences(post.post_id).await.unwrap(),
        audiences_before
    );
}

#[apply(backends)]
#[tokio::test]
async fn publish_post_keeps_an_already_published_timestamp(#[case] backend: Backend) {
    // A15b, second half — COALESCE, not overwrite.
    let TestEnv { state, .. } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let post = SeedPost { published: false, ..SeedPost::for_user(user) }.seed(&state).await;
    let first = state.posts.publish_post(post.post_id, user).await.unwrap().published_at;
    let second = state.posts.publish_post(post.post_id, user).await.unwrap().published_at;
    assert_eq!(first, second, "republishing must not restamp");
}

#[apply(backends)]
#[tokio::test]
async fn publish_post_rejects_a_foreign_or_deleted_post(#[case] backend: Backend) {
    // A15c — the same errors update_post returns today.
    let TestEnv { state, .. } = backend.setup().await;
    let [owner, stranger] = seed_users::<2>(&state).await;
    let post = SeedPost { published: false, ..SeedPost::for_user(owner) }.seed(&state).await;

    assert!(matches!(
        state.posts.publish_post(post.post_id, stranger).await,
        Err(UpdatePostError::Unauthorized)
    ));

    state.posts.soft_delete_post(post.post_id).await.unwrap();
    assert!(matches!(
        state.posts.publish_post(post.post_id, owner).await,
        Err(UpdatePostError::NotFound)
    ));
}
```

Match `SeedPost`'s real field names and constructor
(`storage/src/test_support.rs:796-847`) rather than the illustrative
`SeedPost::for_user` above.

- [x] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p storage publish_post` Expected: FAIL —
`publish_post` not defined.

- [x] **Step 3: Implement**

Per dialect, inside a transaction: the ownership/not-deleted check `update_post`
already performs (`sqlite/posts.rs:43-51`; `postgres/posts.rs:41-52`, which uses
`FOR UPDATE`), then

```sql
UPDATE posts SET published_at = COALESCE(published_at, $1), updated_at = $1
 WHERE post_id = $2
```

then re-read the record with the existing record query so tags and
`author_username` are populated. Write no child rows of any kind.

- [x] **Step 4: Rewire `web::posts::publish`**

Replace the `get_post_by_id` + `get_post_audiences` + `update_post` sequence
(`web/src/posts/api.rs:494-523`) with a single `publish_post` call. The
ownership and soft-delete checks at `:499-501` move into storage, so drop them
here. Keep the `published_at`-missing guard (`:525-527`), the feed-event enqueue
(`:530-533`), and the `host::metrics::post` call (`:535`) — all read from the
returned record.

- [x] **Step 5: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p storage -p web` Expected: PASS

- [x] **Step 6: Commit**

```bash
devtool run -- cargo xtask check
git add storage/src/ web/src/posts/api.rs
git commit -m "fix(posts): publish without rewriting the post (#711)"
```

---

### Task 5: `RenderOutput` and threading it through the post inputs

**Files:**

- Modify: `common/src/render.rs`
- Modify: `storage/src/posts.rs` (`CreatePostInput` `:207`, `UpdatePostInput`
  `:229`, insert SQL `:1918-1932`)
- Modify: `storage/src/post_service.rs` (`:79`, `:169`, `:331`)
- Modify: `storage/src/sqlite/posts.rs`, `storage/src/postgres/posts.rs` (update
  SQL binds)
- Modify (test/fixture construction sites — **the complete list**, since
  "whatever the compiler surfaces" is exactly how a partial sweep happens):
  - `storage/src/test_support.rs:1003-1018` (`SeedRawPost::build`)
  - `storage/src/posts.rs:2390, 2432, 2489, 2738, 2937, 3126`
  - `server/tests/storage/mod.rs:2271, 2298, 2335, 2540, 2630, 4244, 4598, 4728`

  **Correction, recorded during execution.** This list was built by grepping
  `rendered_html` rather than by the input types, so it over-counted: of the six
  `storage/src/posts.rs` entries only **two** are input constructions — the
  other four are `PostRecord` literals, which keep their own `rendered_html`
  field and are untouched. `storage/src/test_support.rs` needed two edits, not
  one (`into_input`, plus the `SeededPost` read-back in `create`). The
  `server/tests/storage/mod.rs` entries were all real, and nearby struct-update
  sites (`..update_private.clone()`) ride along because `RenderOutput` is
  `Clone`. Nothing was missed: the field no longer exists on either struct, so a
  missed site is a compile error, and `cargo check -p jaunder --all-targets`
  passes.

**Interfaces:**

- Consumes: `extract_media_refs` (Task 3); `publish_post` (Task 4) — without it,
  `web/src/posts/api.rs:511` cannot be made to compile.
- Produces:

```rust
/// A rendered post body and the media it references — derived together, never
/// separately. The reference set is private and rendering is the only constructor, so a
/// value whose set disagrees with its HTML is unrepresentable (spec D1).
#[cfg(feature = "sanitize")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderOutput {
    pub html: RenderedHtml,
    media: Vec<MediaRef>,
}

impl RenderOutput {
    #[must_use]
    pub fn render(body: &PostBody, format: &PostFormat) -> Self;
    #[must_use]
    pub fn media(&self) -> &[MediaRef];
}
```

`CreatePostInput` / `UpdatePostInput`: field `rendered_html: RenderedHtml`
becomes `rendered: RenderOutput`.

- [x] **Step 1: Write the failing tests**

In `common/src/render.rs` tests:

```rust
#[test]
fn render_output_derives_its_media_from_its_html() {
    let body: PostBody = format!("<img src=\"{}\">", media_url_for("photo.jpg")).into();
    let out = RenderOutput::render(&body, &PostFormat::Markdown);
    assert_eq!(out.media(), extract_media_refs(out.html.as_ref()).as_slice());
    assert_eq!(out.media().len(), 1);
}

#[test]
fn render_output_media_is_empty_for_a_body_referencing_nothing() {
    let out = RenderOutput::render(&"plain text".to_owned().into(), &PostFormat::Markdown);
    assert!(out.media().is_empty());
}
```

And A10 as a doctest **pair** on `RenderOutput` — the `compile_fail` alone is
vacuous if the item doesn't exist (it is `#[cfg(feature = "sanitize")]`, so a
feature-less doctest build would fail for the wrong reason and pass green). The
passing twin proves the type is present and constructible:

````rust
/// The reference set is derived, never supplied:
/// ```
/// # use common::render::RenderOutput;
/// # use common::post_format::PostFormat;
/// let out = RenderOutput::render(&"hello".to_owned().into(), &PostFormat::Markdown);
/// assert!(out.media().is_empty());
/// ```
///
/// ```compile_fail
/// # use common::render::{RenderOutput, RenderedHtml};
/// let _ = RenderOutput { html: RenderedHtml::from_trusted("<p>x</p>"), media: vec![] };
/// ```
````

- [x] **Step 2: Run the tests, verify they fail**

Run:
`devtool run -- cargo nextest run -p common --features sanitize render_output`
Expected: FAIL — `RenderOutput` not defined.

- [x] **Step 3: Implement `RenderOutput`**

`RenderOutput::render` calls the existing free `render` for the HTML, then
`extract_media_refs` on the result. Keep the free `pub fn render` — Task 3's
tests and the `sanitize_*` tests use it, and this is a thin composition over it.

- [x] **Step 4: Thread it through, completing the sweep**

Change both input structs' field, then work the enumerated list in **Files**
above. The three `post_service` sites become
`RenderOutput::render(&body, &format)`. Fixtures passing
`RenderedHtml::from_trusted(…)` become `RenderOutput::render(&body, &format)` —
`test_support.rs:1375` already documents "default rendered_html equals
render(body)", so that is what they meant. Update every SQL bind of
`rendered_html` to `input.rendered.html`.

- [x] **Step 5: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p common -p storage -p web -p jaunder`
(**all four** — `-p common -p storage` alone never compiles `web`, which is
where the publish site lives.) Expected: PASS

Run the doctests, which `cargo doc` does **not** do:
`devtool run -- cargo test -p common --doc --features sanitize` Expected: PASS

- [x] **Step 6: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/render.rs storage/src/ server/tests/
git commit -m "refactor(posts): bind rendered HTML to its media references (#711)"
```

---

### Task 6: `post_media` migrations, the write path, and the shared fixtures

**Files:**

- Create: `storage/migrations/sqlite/0025_create_post_media.sql`
- Create: `storage/migrations/postgres/0025_create_post_media.sql`
- Modify: `storage/src/posts.rs` (beside `replace_post_audiences` `:1971`;
  dialect consts `sqlite/posts.rs:19`, `postgres/posts.rs:20`)
- Modify: `storage/src/sqlite/posts.rs` (`:112` region),
  `storage/src/postgres/posts.rs`
- Modify: `storage/src/test_support.rs` (the shared fixtures below)
- Create/modify: an AtomPub integration test under `server/tests/atompub/`

**Interfaces:**

- Consumes: `RenderOutput::media()` (Task 5), `MediaRef` (Task 2).
- Produces:

```rust
/// Replaces a post's `post_media` rows to exactly match `media`. Deletes every existing
/// row for `post_id`, then inserts one per reference. Runs on the caller's executor so it
/// shares the create/update transaction — mirrors `replace_post_audiences`.
pub(crate) async fn replace_post_media<DB>(
    conn: &mut DB::Connection,
    post_id: PostId,
    media: &[MediaRef],
) -> sqlx::Result<()>
where
    DB: PostDialect,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>;
```

Plus `PostDialect::DELETE_POST_MEDIA` / `INSERT_POST_MEDIA` consts, and these
shared fixtures in `storage/src/test_support.rs`, used by Tasks 6–8:

```rust
/// The canonical serve URL for `name` under the shared test digest — the single place a
/// test spells a media URL.
pub fn media_url_for(name: &str) -> String;
/// The `MediaRef` that `media_url_for(name)` names.
pub fn media_ref_for(name: &str) -> MediaRef;
/// Seeds a `media` row owned by `user`; returns the `MediaRef` naming it.
pub async fn seed_media(state: &Arc<AppState>, user: UserId, name: &str) -> MediaRef;
/// Whether a `media` row exists for `user` and `media`.
pub async fn media_row_exists(state: &Arc<AppState>, user: UserId, media: &MediaRef) -> bool;
/// A post's `post_media` rows as `(source, sha256, filename)`, ascending.
pub async fn fetch_post_media(base: &TestBase, post_id: PostId) -> Vec<(String, String, String)>;
/// Creates a post through `post_service::perform_post_creation` — the same entry point
/// `web::posts::create` uses — so tests exercise the product's path, not a synthetic
/// input. `create_draft_via_service` is the unpublished twin.
pub async fn create_post_via_service(state: &Arc<AppState>, user: UserId, body: &str) -> PostId;
pub async fn create_draft_via_service(state: &Arc<AppState>, user: UserId, body: &str) -> PostId;
/// Edits a post's body through `post_service::perform_post_update`.
pub async fn update_post_body_via_service(
    state: &Arc<AppState>,
    post_id: PostId,
    user: UserId,
    body: &str,
);
```

Build these on the existing builders and `TestBase::execute` / `scalar_i64`
(`:90`, `:109`) rather than hand-rolling connections.

- [x] **Step 1: Write the migrations**

Exactly as spec D7 — SQLite `INTEGER`; Postgres `BIGINT` **and**
`DEFERRABLE INITIALLY IMMEDIATE`; both with the
`(post_id, source, sha256, filename)` primary key and the
`(sha256, filename, source)` lookup index. `git add` both files immediately.

- [x] **Step 2: Add the shared fixtures**

The full set above. One definition, reused by Tasks 6–8.

- [x] **Step 3: Write the failing tests**

In `storage/src/posts.rs`'s test module:

```rust
#[apply(backends)]
#[tokio::test]
async fn create_post_writes_its_media_rows(#[case] backend: Backend) {
    // A11, and the web half of A14 — create_post_via_service is the product's own path.
    let TestEnv { state, base } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let body = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
    let post_id = create_post_via_service(&state, user, &body).await;

    let rows = fetch_post_media(&base, post_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2, "photo.jpg");
}

#[apply(backends)]
#[tokio::test]
async fn create_post_records_a_raw_filename_and_a_member_url(#[case] backend: Backend) {
    // A2, A3 at the persistence level — the issue's two headline spellings become rows.
    let TestEnv { state, base } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let raw = media_url_for("my photo.jpg").replace("%20", " ");
    let member = format!("/atompub/alice/media/{CANONICAL}/photo.jpg");
    let body = format!("<img src=\"{raw}\"><a href=\"{member}\">doc</a>");
    let post_id = create_post_via_service(&state, user, &body).await;

    let names: Vec<String> = fetch_post_media(&base, post_id).await.into_iter().map(|r| r.2).collect();
    assert!(names.contains(&"my%20photo.jpg".to_owned()), "raw spelling canonicalised");
    assert!(names.contains(&"photo.jpg".to_owned()), "member URL recorded");
}

#[apply(backends)]
#[tokio::test]
async fn a_post_referencing_nothing_writes_no_media_rows(#[case] backend: Backend) {
    // A13 — no false positives.
    let TestEnv { state, base } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let post_id = create_post_via_service(&state, user, "just some prose").await;
    assert!(fetch_post_media(&base, post_id).await.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn updating_a_post_replaces_its_media_rows(#[case] backend: Backend) {
    // A12 — both directions.
    let TestEnv { state, base } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let a = media_url_for("a.jpg");
    let b = media_url_for("b.jpg");
    let post_id = create_post_via_service(&state, user, &format!("<img src=\"{a}\">")).await;

    update_post_body_via_service(&state, post_id, user, &format!("<img src=\"{b}\">")).await;
    let rows = fetch_post_media(&base, post_id).await;
    assert_eq!(rows.len(), 1, "the removed reference is gone");
    assert_eq!(rows[0].2, "b.jpg", "the added reference is present");

    update_post_body_via_service(&state, post_id, user, "no media at all").await;
    assert!(fetch_post_media(&base, post_id).await.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn publishing_a_draft_preserves_its_media_rows(#[case] backend: Backend) {
    // A15 and the row half of A15b — the D1 regression. Fails against any design that
    // routes publication through update_post. Lives here, not in Task 4, because it
    // needs post_media to exist.
    let TestEnv { state, base } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let body = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
    let post_id = create_draft_via_service(&state, user, &body).await;
    let before = fetch_post_media(&base, post_id).await;
    assert_eq!(before.len(), 1, "precondition: the draft records its reference");

    state.posts.publish_post(post_id, user).await.expect("publish succeeds");

    assert_eq!(fetch_post_media(&base, post_id).await, before, "rows survive publication");
}
```

- [x] **Step 4: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p storage media_rows` Expected: FAIL —
no `post_media` table.

- [x] **Step 5: Implement**

`replace_post_media` mirrors `replace_post_audiences` exactly
(delete-all-then-insert on the caller's connection; real bounds copied from
`storage/src/posts.rs:1976-1982`). Call it in `create_post` (the `:1888` region)
and in both dialects' `update_post`, **immediately beside** the existing
`replace_post_audiences` call (`sqlite/posts.rs:112`), inside the same
`BEGIN IMMEDIATE` transaction — they are the same concern (a post's child rows)
and should not be separated.

- [x] **Step 6: Cover the AtomPub creation path (A14, second half)**

Step 3 covers the web path. A14 asks for _both_, and AtomPub reaches storage
through `server/src/atompub/posts.rs:382` — a path no `storage` test touches.
Add a dual-backend test alongside the existing ones in
`server/tests/atompub/atompub_posts.rs`, using that crate's real harness
(`server/tests/helpers/mod.rs`): `setup_with_base_url(backend)` (`:356`),
`make_app(&state, &tmp)` (`:332`),
`create_user_and_session(&state) -> SeededSession` (`:184`), and
`atompub_post_xml(session, suffix, xml) -> Request<Body>` (`:299`), driven
through the router. The collection endpoint is `/atompub/<user>/posts` — see
`atompub_posts.rs:334-344` for the established shape of such a test, and follow
it rather than inventing helpers.

The test posts an Atom entry whose content embeds `media_url_for("photo.jpg")`,
asserts the 201, then reads the new post's `post_media` rows and asserts the
single expected row.

Run: `devtool run -- cargo nextest run -p jaunder atompub` Expected: PASS

- [x] **Step 7: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p storage -p jaunder` Expected: PASS

- [x] **Step 8: Commit**

```bash
devtool run -- cargo xtask check
git add storage/migrations/ storage/src/ server/tests/
git commit -m "feat(storage): record post→media references at write time (#711)"
```

---

### Task 7: The read side — `list_posts_referencing_media`

**Correction, recorded during execution.** The truncation test written below is
defective: it seeds 1200 posts with _no_ media plus one needle, but the new
query filters on the media triple **before** any limit, so the result set is one
row and a `LIMIT 1000` truncates nothing. It would pass green against the very
regression it is named for — pinning the _old_ code's shape rather than the new
query's. As implemented, every one of the 1201 posts embeds the same media, so a
cap has something to bite on; a `LIMIT 1000` mutation then fails with
`got 1000 of 1201`.

**Files:**

- Modify: `storage/src/posts.rs` (`PostStorage` trait + generic impl)

**Interfaces:**

- Consumes: `MediaRef` (Task 2), `post_media` and the shared fixtures (Task 6).
- Produces:

```rust
/// The given user's non-deleted posts that reference `media`, ascending by id.
/// No limit — the truncation half of #711.
async fn list_posts_referencing_media(
    &self,
    user_id: UserId,
    media: &MediaRef,
) -> sqlx::Result<Vec<PostId>>;
```

- [x] **Step 1: Write the failing tests**

```rust
#[apply(backends)]
#[tokio::test]
async fn list_posts_referencing_media_scopes_and_orders(#[case] backend: Backend) {
    // A16.
    let TestEnv { state, .. } = backend.setup().await;
    let [owner, stranger] = seed_users::<2>(&state).await;
    let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));

    let first = create_post_via_service(&state, owner, &embed).await;
    let second = create_post_via_service(&state, owner, &embed).await;
    let deleted = create_post_via_service(&state, owner, &embed).await;
    let foreign = create_post_via_service(&state, stranger, &embed).await;
    let unrelated = create_post_via_service(&state, owner, "no media").await;
    state.posts.soft_delete_post(deleted).await.unwrap();

    let found = state
        .posts
        .list_posts_referencing_media(owner, &media_ref_for("photo.jpg"))
        .await
        .unwrap();

    assert_eq!(found, vec![first, second], "own, non-deleted, ascending");
    assert!(!found.contains(&deleted), "soft-deleted posts do not block a delete");
    assert!(!found.contains(&foreign), "another user's post is not reported (spec D9)");
    assert!(!found.contains(&unrelated));
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_referencing_media_finds_a_post_beyond_the_old_scan_window(
    #[case] backend: Backend,
) {
    // A17 — the truncation half. The old code capped at RowLimit::at_most(1000).
    let TestEnv { state, .. } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;

    // One batched transaction via create_posts, not 1200 round trips. Build the inputs
    // with SeedRawPost::build() (test_support.rs:1003); the last one embeds the needle.
    let mut inputs: Vec<CreatePostInput> = (0..1200)
        .map(|i| SeedRawPost::for_user(user).slug(format!("filler-{i}")).body("filler").build())
        .collect();
    inputs.push(
        SeedRawPost::for_user(user)
            .slug("needle")
            .body(format!("<img src=\"{}\">", media_url_for("needle.jpg")))
            .build(),
    );
    state.posts.create_posts(&inputs).await.unwrap();

    let found = state
        .posts
        .list_posts_referencing_media(user, &media_ref_for("needle.jpg"))
        .await
        .unwrap();
    assert_eq!(found.len(), 1, "a reference past the old 1000-row window is found");
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_referencing_media_returns_empty_for_unreferenced_media(
    #[case] backend: Backend,
) {
    let TestEnv { state, .. } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    create_post_via_service(&state, user, "no media").await;
    let found = state
        .posts
        .list_posts_referencing_media(user, &media_ref_for("absent.jpg"))
        .await
        .unwrap();
    assert!(found.is_empty());
}
```

Match `SeedRawPost`'s real builder methods (`test_support.rs:901-1003`) rather
than the illustrative `for_user`/`slug`/`body` chain above.

- [x] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p storage list_posts_referencing_media`
Expected: FAIL — method not defined.

- [x] **Step 3: Implement**

One shared query in the generic impl (the SQL is identical across backends, so
no dialect method):

```sql
SELECT pm.post_id FROM post_media pm
  JOIN posts p ON p.post_id = pm.post_id
 WHERE p.user_id = $1 AND p.deleted_at IS NULL
   AND pm.source = $2 AND pm.sha256 = $3 AND pm.filename = $4
 ORDER BY pm.post_id
```

No `LIMIT` — its absence is the point.

- [x] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p storage list_posts_referencing_media`
Expected: PASS

- [x] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add storage/src/posts.rs
git commit -m "feat(storage): look up the posts referencing a media item (#711)"
```

---

### Task 8: The atomic delete guard and the force affordance

**Files:**

- Modify: `storage/src/media.rs` (`MediaStorage` `:105`, `MediaStore` impl
  `:325`, `MediaDialect` `:138` and its doc comment `:131-136`)
- Modify: `storage/src/sqlite/media.rs:24`, `storage/src/postgres/media.rs:27`
- Modify: `web/src/media/api.rs:133-191`
- Modify: `web/src/media/component.rs:250-320` (the force affordance)
- Modify: `server/src/atompub/media.rs:187`

**Interfaces:**

- Consumes: `MediaRef` (Task 2), `list_posts_referencing_media` (Task 7).
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryDeleteOutcome { Deleted, RefusedReferenced }

/// Deletes a media record, refusing when one of the owner's live posts references it
/// unless `force`. The check and the delete are one statement, so no post can start
/// referencing the media between them (spec D8).
///
/// # Errors
/// `DeleteMediaError::NotFound` if no such record exists — distinguished from a refusal
/// by a follow-up existence check on the cold path.
async fn try_delete_media(
    &self,
    user_id: UserId,
    media: &MediaRef,
    force: bool,
) -> Result<TryDeleteOutcome, DeleteMediaError>;
```

The old four-argument `delete_media` is removed. `web::media::delete` and
AtomPub's `member_delete` move to `try_delete_media`; AtomPub passes
`force = true`, since it has no confirmation UI and its current behaviour is an
unconditional delete that must not change.

> **Note for the conformance reviewer:** this signature deliberately diverges
> from spec D8's literal `-> sqlx::Result<bool>`. D8's own "not-found must stay
> distinguishable" bullet contradicts that signature; this is the resolution,
> not drift.

- [ ] **Step 1: Write the failing tests**

```rust
#[apply(backends)]
#[tokio::test]
async fn try_delete_media_refuses_a_referenced_item_unless_forced(#[case] backend: Backend) {
    // A17b.
    let TestEnv { state, .. } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let media = seed_media(&state, user, "photo.jpg").await;
    let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
    create_post_via_service(&state, user, &embed).await;

    assert_eq!(
        state.media.try_delete_media(user, &media, false).await.unwrap(),
        TryDeleteOutcome::RefusedReferenced
    );
    assert!(media_row_exists(&state, user, &media).await, "refusal leaves the row");

    assert_eq!(
        state.media.try_delete_media(user, &media, true).await.unwrap(),
        TryDeleteOutcome::Deleted
    );
    assert!(!media_row_exists(&state, user, &media).await);
}

#[apply(backends)]
#[tokio::test]
async fn try_delete_media_deletes_an_unreferenced_item(#[case] backend: Backend) {
    let TestEnv { state, .. } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let media = seed_media(&state, user, "photo.jpg").await;
    assert_eq!(
        state.media.try_delete_media(user, &media, false).await.unwrap(),
        TryDeleteOutcome::Deleted
    );
}

#[apply(backends)]
#[tokio::test]
async fn try_delete_media_reports_not_found_distinctly_from_refusal(#[case] backend: Backend) {
    // A17c — preserves today's DeleteMediaError::NotFound.
    let TestEnv { state, .. } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    assert!(matches!(
        state.media.try_delete_media(user, &media_ref_for("never-uploaded.jpg"), false).await,
        Err(DeleteMediaError::NotFound)
    ));
}

#[apply(backends)]
#[tokio::test]
async fn try_delete_media_holds_under_concurrent_reference_writes(#[case] backend: Backend) {
    // A17d/A17e. Be honest about what this establishes: a stress test cannot *prove*
    // atomicity — that would need controlled interleaving inside the statement, which
    // SQL gives no hook for. Atomicity here is structural: it is one statement. What
    // this does establish is (a) the statement survives sustained concurrency without
    // SQLITE_BUSY (A17e), and (b) the guard does not ignore references under load.
    //
    // Written monotone — the writer only ever ADDS references — so it cannot false-fail
    // the way an add/remove churn would, where a reference legitimately appearing between
    // the delete and a separate verification read looks identical to a violation.
    let TestEnv { state, .. } = backend.setup().await;
    let [user] = seed_users::<1>(&state).await;
    let media = seed_media(&state, user, "photo.jpg").await;
    let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));

    // One reference exists before any delete is attempted, and none is ever removed, so
    // every unforced delete from here on must refuse.
    create_post_via_service(&state, user, &embed).await;

    let writer = tokio::spawn({
        let state = Arc::clone(&state);
        let embed = embed.clone();
        async move {
            for _ in 0..100 {
                create_post_via_service(&state, user, &embed).await;
            }
        }
    });

    for _ in 0..100 {
        let outcome = state
            .media
            .try_delete_media(user, &media, false)
            .await
            .expect("no SQLITE_BUSY under concurrency");
        assert_eq!(
            outcome,
            TryDeleteOutcome::RefusedReferenced,
            "a live reference exists throughout, so no unforced delete may succeed"
        );
    }
    writer.await.unwrap();
    assert!(media_row_exists(&state, user, &media).await);
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p storage try_delete_media` Expected:
FAIL — `try_delete_media` not defined.

- [ ] **Step 3: Implement**

The conditional statement from spec D8, run with `fetch_optional`. Implement it
in the **generic** `MediaStore` impl, not the dialect:
`MediaDialect::delete_media_row` exists only because `.rows_affected()` needs
monomorphising (`storage/src/media.rs:131-136`), and `RETURNING` +
`fetch_optional` is generic. So delete `MediaDialect::delete_media_row` and its
two impls, and update that doc comment, which currently explains why
`delete_media` lives there.

On no row returned, classify with one existence check on `media`: present →
`RefusedReferenced`; absent → `Err(NotFound)`.

- [ ] **Step 4: Run the tests; if `SQLITE_BUSY` appears, take the fallback**

Run: `devtool run -- cargo nextest run -p storage try_delete_media` Expected:
PASS

If `try_delete_media_holds_under_concurrent_reference_writes` fails on SQLite
with `SQLITE_BUSY`, the shape _does_ reproduce the `sessions.rs:19` hazard.
Then: reintroduce a `MediaDialect` method, keep the single statement for
Postgres, and give SQLite the two-statement form inside `BEGIN IMMEDIATE` (which
serialises correctly there). Observable behaviour is identical, so the tests are
unchanged — only the implementation diverges, which ADR-0053 permits. **Record
which branch was taken in the commit message**, and if it was the fallback, add
a comment at the SQLite impl citing `sessions.rs:19` so the next reader knows it
was measured, not assumed.

- [ ] **Step 5: Add the force-delete affordance**

**This is production UI work, and A18 cannot be met without it.**
`web/src/media/component.rs:264` already tells the user "Use force delete to
remove anyway", but the only delete control is a single `ActionForm`
(`:309-320`) with hidden `sha256`/`filename`/`source` and **no `force` field** —
the UI instructs the user to do something it does not offer. The server side is
already able: `web/src/media/api.rs:137` takes `force: Option<bool>`.

Add a force submission carrying
`<input type="hidden" name="force" value="true">`, rendered in the refusal
branch at `:250-276` where the referencing post IDs are shown, so it appears
only once a delete has actually been refused. Give it an accessible name
containing "Force delete" — Task 9's e2e selects on it.

- [ ] **Step 6: Rewire the handler**

`web/src/media/api.rs::delete` becomes: build the `MediaRef`, call
`list_posts_referencing_media` for the message, call `try_delete_media` for the
decision, map the outcome onto the unchanged `DeleteResult`. Remove the
`RowLimit` import (`:26`), the scan (`:144-172`) and the now-unused
`viewer_identity` call. `server/src/atompub/media.rs:187` moves to
`try_delete_media(…, force = true)`.

- [ ] **Step 7: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p storage -p web -p jaunder` Expected:
PASS

- [ ] **Step 8: Commit**

```bash
devtool run -- cargo xtask check
git add storage/src/ web/src/media/ server/src/atompub/media.rs
git commit -m "fix(media): decide and delete in one statement (#711)"
```

---

### Task 9: End-to-end cover for the refuse/force flow

The causal chain — render writes the rows, the guard reads them — only exists
end to end; no Rust test spans it.

**Files:**

- Modify: `end2end/tests/media.spec.ts`
- Modify: `docs/coverage/server-fns.json` (regenerated)

**Interfaces:**

- Consumes: everything above, including Task 8 Step 5's force control.
- Produces: e2e test names referenced by the coverage snapshot.

- [ ] **Step 1: Write the failing e2e**

Add to `media.spec.ts`, whose existing imports already cover `BASE_URL`,
`click`, `waitForSelector`, `register`, and
`slowBrowserFirstNavigationTimeoutMs`. Additionally import `createPostViaApi`
from `./posts` — signature
`(page, {body, tags?, publish?, slug?}) -> {post_id, permalink}`
(`end2end/tests/posts.ts:21-48`); it hardcodes `format: "markdown"`, which is
fine since raw HTML passes through the Markdown renderer. `register` returns the
username (`helpers.ts:191-224`).

```ts
/** Uploads `name`; returns its canonical URL and encoded filename key. */
async function uploadMedia(page, name: string) {
  const response = await page.request.post(BASE_URL + "/api/media/upload", {
    multipart: {
      file: {
        name,
        mimeType: "image/jpeg",
        buffer: Buffer.from("guard test content"),
      },
    },
  });
  expect(response.status()).toBe(200);
  return await response.json();
}

/** Opens the media library and clicks Delete, accepting the confirm dialog. */
async function attemptDelete(page) {
  await click(page, "a[href='/media']");
  await waitForSelector(page, "button:has-text('Attach media')");
  page.on("dialog", (dialog) => dialog.accept());
  await click(page, 'button:has-text("Delete")');
}

test("deleting media referenced by a post is refused, then forced", async ({
  page,
}, testInfo) => {
  // A18. The whole causal chain: rendering wrote post_media rows, the guard reads them.
  await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));
  const { url } = await uploadMedia(page, "referenced.jpg");
  await createPostViaApi(page, { body: `![pic](${url})` });

  await attemptDelete(page);
  await expect(
    page.getByText(/Cannot delete: referenced in post/),
  ).toBeVisible();
  // The library link text is the DECODED name (component.rs:289).
  await expect(
    page.getByRole("link", { name: "referenced.jpg" }),
  ).toBeVisible();

  await click(page, 'button:has-text("Force delete")');
  await expect(page.getByRole("link", { name: "referenced.jpg" })).toHaveCount(
    0,
  );
});

test("a post embedding the raw filename spelling blocks deletion", async ({
  page,
}, testInfo) => {
  // A19, first half — the #675 symptom, proved through the guard rather than the parser.
  // The upload returns the canonical encoded URL; the post embeds the RAW spelling, which
  // the old substring match could not see.
  await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 30000));
  const { url } = await uploadMedia(page, "my holiday photo.jpg");
  const rawUrl = url.replace(/%20/g, " ");
  expect(rawUrl).not.toBe(url);
  await createPostViaApi(page, { body: `<img src="${rawUrl}">` });

  await attemptDelete(page);
  await expect(
    page.getByText(/Cannot delete: referenced in post/),
  ).toBeVisible();
});

test("a post embedding the AtomPub member URL blocks deletion", async ({
  page,
}, testInfo) => {
  // A19, second half. The member URL shares no prefix with the serve URL, so the old
  // exact-URL match could never have matched it.
  const username = await register(
    page,
    slowBrowserFirstNavigationTimeoutMs(testInfo, 30000),
  );
  const { url } = await uploadMedia(page, "linked.jpg");
  // /media/upload/<p1>/<p2>/<sha>/<name> → /atompub/<user>/media/<sha>/<name>
  const [, , , , sha, name] = url.split("/");
  await createPostViaApi(page, {
    body: `<a href="/atompub/${username}/media/${sha}/${name}">doc</a>`,
  });

  await attemptDelete(page);
  await expect(
    page.getByText(/Cannot delete: referenced in post/),
  ).toBeVisible();
});
```

The refusal copy is verbatim from `web/src/media/component.rs:264`: "Cannot
delete: referenced in post(s) {ids}. Use force delete to remove anyway."

- [ ] **Step 2: Run them, verify they fail**

Run: `devtool run -- cargo xtask e2e-local media.spec.ts` Expected: FAIL —
before Tasks 2–8 are wired through, deletion succeeds and the refusal text never
appears.

- [ ] **Step 3: Make them pass**

The behaviour comes from Tasks 2–8, including Task 8 Step 5's force control. If
a test fails for a UI reason (e.g. the editor escaping the raw URL), fix the
test, not the guard.

- [ ] **Step 4: Update the coverage snapshot (A21)**

Run: `devtool run -- cargo xtask server-fn-coverage regenerate`
(`CONTRIBUTING.md:492`)

`media::delete` is already covered in `docs/coverage/server-fns.json:369` with
no allowlist entry (retired in #720), so **no allowlist change is expected** —
if the gate demands one, stop and work out why rather than adding it. Verify
against **sqlite/chromium**, the only combo that fails on an omitted coverage
update.

Run: `devtool run -- cargo xtask e2e sqlite chromium` Expected: PASS

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add end2end/tests/media.spec.ts docs/coverage/server-fns.json
git commit -m "test(e2e): cover the media delete guard end to end (#711)"
```

---

### Task 10: Backup golden list — DONE as part of Task 6

**Planning error, recorded.** This could never have been a separate commit. The
moment `0025_create_post_media.sql` lands,
`backup_covers_every_table_or_deliberately_excludes_it` fails on all three of
its assertions, so a Task 6 commit that omitted this would have failed its own
gate. The two must land together, and did: the golden list, the schema comment
(21 → 22) and `live_count` (23 → 24) were all updated in the Task 6 commit. The
steps below are kept for the record; all three parts are verified applied.

**Files:**

- Modify: `storage/src/backup.rs` — golden list `:699-724`, schema comment
  `:730`, `live_count` assertion `:747`

- [x] **Step 1: Run the drift test, verify it fails**

The relevant test is `backup_covers_every_table_or_deliberately_excludes_it`
(`:681`), **not** `backup_table_set_drops_internal_and_denylisted_and_sorts`
(`:653`) — that one drives `backup_table_set` over a hardcoded list a migration
cannot move, so it would pass and prove nothing. The drift test is
`#[apply(backends)]`, so it needs the PostgreSQL-capable runner.

Run: `devtool run -- cargo nextest run -p storage backup_covers_every_table`
Expected: FAIL — "backup set drifted — add the new table to the golden list or
to `TABLES_EXCLUDED_FROM_BACKUP`" (`:727`).

That failure is the proof `post_media` is auto-discovered and travels with
backup; the list is a tripwire, not a registration.

- [x] **Step 2: Update all three places**

1. Add `post_media` to the golden list (`:699-724`) in sorted position — the
   list is asserted sorted, and `"post_media" < "posts"`.
2. The comment at `:730` says "the whole schema is 21 backed-up tables" →
   **22**.
3. The `assert_eq!(live_count, 23, …)` at `:747` → **24**.

Steps 2 and 3 are easy to miss: the test fails at the golden-list assertion
first, so fixing only that leaves a second failure behind it.

- [x] **Step 3: Run it, verify it passes**

Run: `devtool run -- cargo nextest run -p storage backup` Expected: PASS

- [x] **Step 4: Commit**

```bash
devtool run -- cargo xtask check
git add storage/src/backup.rs
git commit -m "test(backup): admit post_media to the backup set (#711)"
```

---

## Final verification

- [ ] Run the full local gate: `devtool run -- cargo xtask validate`
      (foreground, `timeout: 600000` — background runs get killed). Expected:
      PASS.
- [ ] Confirm `git status --porcelain` is clean — `cargo xtask check`
      auto-formats and does not commit for you.
- [ ] Hand off to `jaunder-ship`, which runs the final review, promotes the ADR
      draft (`cargo xtask adr promote`), archives this plan, opens the PR, and
      releases #711 to Done.
