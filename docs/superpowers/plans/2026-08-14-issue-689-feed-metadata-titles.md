# Typed Feed Metadata Titles Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks via `jaunder-dispatch` when useful). Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace primitive Syndication Feed and AtomPub title/description
fields with five validating domain values while preserving every existing wire
value.

**Architecture:** Define the values and their composition rules in `common`,
then propagate them through the Syndication Feed and AtomPub serializers
independently. Keep typed values intact until the external serialization APIs
require owned or borrowed text. No storage, endpoint, or protocol-shape changes.

**Tech Stack:** Rust 2024, `macros::StrNewtype`, `thiserror`, `rss`,
`atom_syndication`, `serde_json`, `cargo nextest`, `cargo xtask`.

**Spec:**
[`docs/superpowers/specs/2026-08-14-issue-689-feed-metadata-titles.md`](../specs/2026-08-14-issue-689-feed-metadata-titles.md)

## Review

**Scope in:** Five newtypes and constructors; all four affected metadata
structs; RSS/Atom/JSON Feed, AtomPub Service Document, and AtomPub Collection
feed serialization; exact title and description-presence regression assertions.

**Scope out:** New title wording or localization, DisplayName lookup, a
configured Syndication Feed description source, AtomPub accept-media typing,
storage/schema, endpoint, or wire-shape changes.

**Tasks:**

1. Add and test the five `common` metadata value types and composition APIs.
2. Propagate `FeedTitle`/`FeedDescription` through Syndication Feed generation
   and all three serializers.
3. Propagate the three AtomPub title types through Service Document and
   Collection feed generation and serialization.

**Key decisions/risks:**

- Every type trims and rejects blank input; named constructors bypass reparsing
  only because their typed inputs prove nonblank output.
- Five distinct types deliberately prevent cross-surface substitution despite a
  shared string representation.
- Exact serialization assertions are required because compile success alone
  cannot detect changed title spelling or description presence.
- No ADR or glossary edit: this directly applies ADR-0063 and ADR-0101.

## Global Constraints

- Preserve exact current production title spellings and whitespace.
- `None` remains the only absent Syndication Feed description state.
- `None` serializes as RSS's required empty channel description and omits Atom
  `subtitle` and JSON Feed `description`; `Some` serializes unchanged.
- Keep composition rules in `common`; server callers pass `SiteTitle`,
  `FeedSurface`, or `Username`, never preformatted title strings.
- Follow `CONTRIBUTING.md`: no lint suppression without explicit approval; no
  `Co-Authored-By` trailers; tick the task checkbox before its commit gate.

## File Structure

- `common/src/feed/metadata.rs`: `FeedTitle`, `FeedDescription`, their errors,
  parsing/composition tests, and the existing feed metadata structs.
- `common/src/feed/mod.rs`: explicit re-exports for the two feed metadata
  values.
- `common/src/atompub/title.rs`: focused home for `WorkspaceTitle`,
  `CollectionTitle`, `CollectionFeedTitle`, their errors, constructors, and
  tests.
- `common/src/atompub/mod.rs`: declare `title` and explicitly re-export its
  three public values.
- `common/src/feed/{rss,atom,json}.rs`: typed fixtures, boundary conversion,
  exact title/description serialization assertions.
- `server/src/feed/regenerate.rs`: typed Syndication Feed composition and
  focused regeneration assertion.
- `common/src/atompub/service.rs`: typed Service Document fields and exact title
  assertions.
- `common/src/atompub/entry.rs`: typed Collection feed field and exact title
  assertion.
- `server/src/atompub/{service,posts}.rs`: construct AtomPub titles only through
  named typed constructors.

---

### Task 1: Add the metadata value types

**Files:**

- Modify: `common/src/feed/metadata.rs`
- Modify: `common/src/feed/mod.rs`
- Create: `common/src/atompub/title.rs`
- Modify: `common/src/atompub/mod.rs`
- Test: `common/src/feed/metadata.rs`
- Test: `common/src/atompub/title.rs`

**Interfaces:**

- Consumes: `SiteTitle`, `FeedSurface`, `Username`, `macros::StrNewtype`.
- Produces:

```rust
pub struct FeedTitle(String);
pub struct FeedDescription(String);

impl FeedTitle {
    #[must_use]
    pub fn for_surface(site_title: &SiteTitle, surface: &FeedSurface) -> Self;
}

pub struct WorkspaceTitle(String);
pub struct CollectionTitle(String);
pub struct CollectionFeedTitle(String);

impl WorkspaceTitle {
    #[must_use]
    pub fn for_user(username: &Username) -> Self;
}

impl CollectionTitle {
    #[must_use]
    pub fn posts() -> Self;

    #[must_use]
    pub fn media() -> Self;
}

impl CollectionFeedTitle {
    #[must_use]
    pub fn posts(username: &Username) -> Self;
}
```

Each type derives `Clone, Debug, PartialEq, Eq, StrNewtype`, has its own
`Invalid<Type>` error, and implements `FromStr` by outer-trimming, rejecting an
empty result, and owning the trimmed text. Export the five values, not their
construction details, through the existing module surfaces.

- [x] **Step 1: Write failing feed metadata value tests**

Add in-file tests whose assertions are the contract:

```rust
#[test]
fn feed_title_parses_trims_and_rejects_blank() {
    assert_eq!("  A Feed  ".parse::<FeedTitle>().unwrap(), "A Feed");
    assert!("".parse::<FeedTitle>().is_err());
    assert!("   ".parse::<FeedTitle>().is_err());
}

#[test]
fn feed_description_parses_trims_and_rejects_blank() {
    assert_eq!(
        "  Posts from Alice  ".parse::<FeedDescription>().unwrap(),
        "Posts from Alice"
    );
    assert!("".parse::<FeedDescription>().is_err());
    assert!("\t\n".parse::<FeedDescription>().is_err());
}

#[test]
fn feed_title_composes_every_surface() {
    let site = "Jaunder".parse::<SiteTitle>().unwrap();
    assert_eq!(FeedTitle::for_surface(&site, &FeedSurface::Site), "Jaunder");
    assert_eq!(
        FeedTitle::for_surface(
            &site,
            &FeedSurface::SiteTag { tag: "rust".parse().unwrap() }
        ),
        "Jaunder — #rust"
    );
    assert_eq!(
        FeedTitle::for_surface(
            &site,
            &FeedSurface::User { username: "alice".parse().unwrap() }
        ),
        "Jaunder — @alice"
    );
    assert_eq!(
        FeedTitle::for_surface(
            &site,
            &FeedSurface::UserTag {
                username: "alice".parse().unwrap(),
                tag: "rust".parse().unwrap(),
            }
        ),
        "Jaunder — @alice #rust"
    );
}
```

- [x] **Step 2: Write failing AtomPub title value tests**

Create `common/src/atompub/title.rs` tests:

```rust
#[test]
fn atompub_titles_parse_trim_and_reject_blank() {
    assert_eq!("  Workspace  ".parse::<WorkspaceTitle>().unwrap(), "Workspace");
    assert_eq!("  Posts  ".parse::<CollectionTitle>().unwrap(), "Posts");
    assert_eq!(
        "  Alice's posts  ".parse::<CollectionFeedTitle>().unwrap(),
        "Alice's posts"
    );
    assert!("".parse::<WorkspaceTitle>().is_err());
    assert!(" ".parse::<WorkspaceTitle>().is_err());
    assert!("".parse::<CollectionTitle>().is_err());
    assert!("\n".parse::<CollectionTitle>().is_err());
    assert!("".parse::<CollectionFeedTitle>().is_err());
    assert!(" \t ".parse::<CollectionFeedTitle>().is_err());
}

#[test]
fn atompub_title_constructors_preserve_current_policy() {
    let username = "alice".parse::<Username>().unwrap();
    assert_eq!(WorkspaceTitle::for_user(&username), "alice");
    assert_eq!(CollectionTitle::posts(), "Posts");
    assert_eq!(CollectionTitle::media(), "Media");
    assert_eq!(CollectionFeedTitle::posts(&username), "alice's posts");
}
```

- [x] **Step 3: Run the focused tests and verify RED**

Run:

```bash
devtool run -- cargo nextest run -p common -E 'test(feed::metadata::tests) | test(atompub::title::tests)'
```

Expected: FAIL because the five types and constructors do not exist.

- [x] **Step 4: Implement the five values and module exports**

Use the exact interfaces above. Keep each `FromStr` implementation direct and
boring; do not add a shared parser abstraction for five one-branch validators.
Constructors may build the private tuple structs directly because all literal or
typed inputs are nonblank. Move the current four-arm `compute_title` body into
`FeedTitle::for_surface`; do not remove the server helper until Task 2 migrates
its caller.

- [x] **Step 5: Run the focused tests and verify GREEN**

Run:

```bash
devtool run -- cargo nextest run -p common -E 'test(feed::metadata::tests) | test(atompub::title::tests)'
```

Expected: PASS for all new parsing and composition cases.

- [x] **Step 6: Mark Task 1 complete**

Check every preceding Task 1 step, then check this completion checkpoint before
staging or running the commit gate.

- [x] **Step 7: Gate and commit**

Follow `jaunder-commit`: stage the Task 1 files plus the approved spec and this
plan, run `devtool run -- cargo xtask check` with the required long foreground
timeout, inspect/stage any formatter changes, and commit:

```text
feat(common): type feed metadata titles (#689)
```

No trailer.

---

### Task 2: Propagate typed Syndication Feed metadata

**Files:**

- Modify: `common/src/feed/metadata.rs`
- Modify: `common/src/feed/rss.rs`
- Modify: `common/src/feed/atom.rs`
- Modify: `common/src/feed/json.rs`
- Modify: `server/src/feed/regenerate.rs`
- Test: `common/src/feed/{rss,atom,json}.rs`
- Test: `server/src/feed/regenerate.rs`

**Interfaces:**

- Consumes: Task 1's `FeedTitle::for_surface` and `FeedDescription`.
- Produces:

```rust
pub struct FeedMetadata {
    pub title: FeedTitle,
    pub description: Option<FeedDescription>,
    pub canonical_url: CanonicalUrl,
    pub self_url: FeedUrl,
    pub hub_url: Option<HubUrl>,
    pub updated_at: DateTime<Utc>,
}
```

`regenerate_feed` constructs `FeedTitle` directly from `&identity.title` and
`&surface`; the old server-local `compute_title` function is deleted. Renderers
convert only where their external builder/value API requires `String`.

- [ ] **Step 1: Add exact failing Syndication serialization tests**

Change each renderer fixture to accept `description: Option<&str>` and parse
`FeedTitle`/`FeedDescription`. Add assertions:

```rust
// RSS
assert!(render_rss(&meta(None, None), &[]).contains("<title>Site</title>"));
assert!(render_rss(&meta(None, None), &[]).contains("<description></description>"));
assert!(render_rss(&meta(None, Some("A site")), &[]).contains("<description>A site</description>"));

// Atom
let none = render_atom(&meta(None, None), &[]);
assert!(none.contains("<title>Site</title>"));
assert!(!none.contains("<subtitle"));
assert!(render_atom(&meta(None, Some("A site")), &[]).contains("<subtitle>A site</subtitle>"));

// JSON Feed
let none: Value = serde_json::from_str(&render_json(&meta(None, None), &[])).unwrap();
assert_eq!(none["title"], "Site");
assert!(none.get("description").is_none());
let some: Value = serde_json::from_str(&render_json(&meta(None, Some("A site")), &[])).unwrap();
assert_eq!(some["description"], "A site");
```

Use the renderer's actual RSS empty-element spelling if `rss` serializes the
required empty description as a self-closing element; assert by parsing the
channel when a raw-string assertion would depend on serializer formatting. The
contract is an empty description value, not one XML spelling.

Convert the existing mock-store regeneration regression into the named test
`regenerate_user_tag_feed_emits_typed_composed_title_and_base_anchored_url`:
regenerate `"/~alice/tags/rust/feed.json"` rather than `"/feed.rss"`, parse
`row.body` as `serde_json::Value`, and assert
`body["title"] == "Jaunder — @alice #rust"`. Retain the existing absolute
canonical-URL assertion, updated for the user-tag canonical URL. This test must
call `regenerate_feed`; a constructor-only assertion does not satisfy the
contract.

- [ ] **Step 2: Run the focused renderer/server tests and verify RED**

Run:

```bash
devtool run -- cargo nextest run -p common -E 'test(feed::rss::tests) | test(feed::atom::tests) | test(feed::json::tests)'
devtool run -- cargo nextest run -p jaunder -E 'test(feed::regenerate::tests)'
```

Expected: FAIL because `FeedMetadata` still accepts primitive fields and the new
presence assertions are not all implemented.

- [ ] **Step 3: Migrate `FeedMetadata`, renderers, and regeneration**

Change the two field types, migrate all LSP-reported struct literals, and keep
external conversions at these boundaries:

- `rss::ChannelBuilder`: owned title/description strings;
- `atom_syndication::Text::plain`: owned title/subtitle strings;
- `serde_json::Value`: serialize/convert the title and optional description
  without flattening earlier.

Replace `compute_title(&identity.title, &surface)` with
`FeedTitle::for_surface(&identity.title, &surface)` and delete `compute_title`
plus its now-redundant server-local four-arm unit test. Preserve
`description: None` in production.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the same two commands from Step 2.

Expected: PASS with exact title text and both description states pinned.

- [ ] **Step 5: Mark Task 2 complete**

Check every preceding Task 2 step, then check this completion checkpoint before
staging or running the commit gate.

- [ ] **Step 6: Gate and commit**

Follow `jaunder-commit`, run the full per-commit
`devtool run -- cargo xtask check`, stage its mechanical changes, and commit:

```text
refactor(feed): carry typed document metadata (#689)
```

No trailer.

---

### Task 3: Propagate typed AtomPub titles

**Files:**

- Modify: `common/src/atompub/service.rs`
- Modify: `common/src/atompub/entry.rs`
- Modify: `server/src/atompub/service.rs`
- Modify: `server/src/atompub/posts.rs`
- Test: `common/src/atompub/service.rs`
- Test: `common/src/atompub/entry.rs`

**Interfaces:**

- Consumes: Task 1's `WorkspaceTitle::for_user`,
  `CollectionTitle::{posts, media}`, and `CollectionFeedTitle::posts`.
- Produces:

```rust
pub struct CollectionDecl {
    pub href: CollectionHrefUrl,
    pub title: CollectionTitle,
    pub accept: Vec<String>,
    pub categories: Vec<Tag>,
}

pub struct ServiceDocument {
    pub workspace_title: WorkspaceTitle,
    pub posts_collection: CollectionDecl,
    pub media_collection: CollectionDecl,
}

pub struct FeedMeta {
    pub id: EntryIdUrl,
    pub title: CollectionFeedTitle,
    pub updated: UtcInstant,
    pub self_url: FeedUrl,
    pub first: Option<PaginationUrl>,
    pub next: Option<PaginationUrl>,
    pub previous: Option<PaginationUrl>,
}
```

- [ ] **Step 1: Add exact failing AtomPub title serialization tests**

Migrate fixtures to the named constructors, then pin the serializer output:

```rust
#[test]
fn service_document_serializes_exact_workspace_and_collection_titles() {
    let out = render_service_document(&sample_doc());
    assert!(out.contains("<atom:title>alice</atom:title>"), "out: {out}");
    assert!(out.contains("<atom:title>Posts</atom:title>"), "out: {out}");
    assert!(out.contains("<atom:title>Media</atom:title>"), "out: {out}");
    assert_eq!(out.matches("<atom:title>").count(), 3, "out: {out}");
}
```

In `entry.rs`, strengthen `render_feed_wraps_entries_with_paging` from a
fragment assertion to the exact Collection feed title assertion
`contains("<title>alice's posts</title>")`, constructing its title from a parsed
`alice` `Username`. Update the Bob fixture to a parsed `bob` username and assert
`<title>bob's posts</title>` so both typed construction and serialization remain
covered.

- [ ] **Step 2: Run focused AtomPub tests and verify RED**

Run:

```bash
devtool run -- cargo nextest run -p common -E 'test(atompub::service::tests) | test(atompub::entry::tests::render_feed)'
devtool run -- cargo nextest run -p jaunder -E 'test(atompub::atompub_service::service_document_returns_200_with_app_password) | test(atompub::atompub_posts::collection_lists_user_posts)'
```

Expected: FAIL until the structs and server construction sites accept the new
value types.

- [ ] **Step 3: Migrate Service Document title fields and callers**

Change `ServiceDocument.workspace_title` and `CollectionDecl.title`. In
`server/src/atompub/service.rs`, keep `AuthUser.username` typed instead of
flattening it. Replace only the three primitive title expressions:

```rust
workspace_title: WorkspaceTitle::for_user(&auth_user.username),
title: CollectionTitle::posts(),
title: CollectionTitle::media(),
```

The two `title` expressions belong in the existing Posts and Media
`CollectionDecl` literals respectively; their URL, accept, and category fields
remain byte-for-byte unchanged.

Borrow the typed strings in `write_text_element`; do not allocate a second
intermediate `String`.

- [ ] **Step 4: Migrate Collection feed title and caller**

Change `FeedMeta.title`; construct it in `server/src/atompub/posts.rs` with
`CollectionFeedTitle::posts(username)` where `username` remains `&Username`.
Convert to owned text only in `atom_syndication::Text::plain`.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run the same two commands from Step 2.

Expected: PASS with the exact three Service Document titles and exact Collection
feed title preserved.

- [ ] **Step 6: Mark Task 3 complete**

Check every preceding Task 3 step, then check this completion checkpoint before
staging or running the commit gate.

- [ ] **Step 7: Gate and commit**

Follow `jaunder-commit`, run `devtool run -- cargo xtask check`, stage any
formatter changes, and commit:

```text
refactor(atompub): carry typed document titles (#689)
```

No trailer.

---

## Final self-review before execution handoff

- Spec AC1-4 map to Tasks 1-3's interfaces and migrations.
- Spec AC5 maps to Tasks 2-3's exact title assertions.
- Spec AC6 maps to Task 2's `None`/`Some` presence matrix.
- Spec AC7 is recorded in the approved spec; the plan does not touch the removed
  accumulator or accept-media strings.
- Spec AC8 runs at every task commit; `jaunder-ship` later runs the full
  `cargo xtask validate` gate.
- All produced symbols consumed by later tasks are declared in Task 1.
- No separable concern surfaced; no issue-filing task is needed.
