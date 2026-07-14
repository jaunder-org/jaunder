# `Tag` vertical (#409) Implementation Plan

> **For agentic workers:** Execute task-by-task with **jaunder-iterate**
> (delegating an individual task to a subagent via **jaunder-dispatch** when
> useful). Steps use checkbox (`- [ ]`) syntax; tick them in real time. The
> isolated worktree already exists (`jaunder-start`).

**Spec:**
[`2026-07-14-issue-409-tag-vertical.md`](../specs/2026-07-14-issue-409-tag-vertical.md)
— this plan is "how"; the spec is "what/why". Decision record:
`docs/adr/drafts/tag-identity-label-split.md`.

---

## Review header (approve this layer)

**Goal:** Thread the two tag newtypes — `Tag` (canonical slug) and the new
`TagLabel` (validated, case-preserving label) — through `common`/`storage`/
`server`/`web` so no tag value is a bare `String`/`&str` except the two
justified boundary sites (spec Decision 7). Absorbs #416 and adds one deliberate
behavior change: the web composer now preserves author casing.

**Scope — in:** `common` (`Tag` StrNewtype adoption, new `TagLabel`,
`parse_and_validate_tags` retype, `FeedItem.tags`, `CollectionDecl.categories`);
`storage` tag structs/traits/impls (`PostTag`, `post_tag_diff`, `tag_post`,
`PostTagJson`); `server` atompub category ingest/emit + service doc; `web`
`TagSummary`/DTOs, create/update wire, `TagInput` (#416 + casing), browse args +
`PageSeed` routing + projector parse; the `.as_str()`→`.as_ref()` sweep. **Scope
— out:** the `list_tags` search prefix and the projector `Path<String>`
extractor (Decision 7 — justified `String`); `list_tags` autocomplete per-post
casing (the separate "M5" catalog item); any wire/schema/migration change.

**Tasks:**

1. `common`: `Tag` adopts `StrNewtype`; add `TagLabel`; sweep every
   compiler-forced `.as_str()` on a `Tag` (incl.
   `feed_events::tag_slugs → BTreeSet<Tag>`). Green, no threading yet.
2. `storage` + `server` + `common` DTOs: thread `Tag`/`TagLabel` through the tag
   storage surface + atompub + `parse_and_validate_tags`; the web write body
   keeps a temporary wire-boundary parse (removed in T3). Green.
3. `web` write DTO + wire + `TagInput`:
   `TagSummary { slug: Tag, display: TagLabel }`, typed `Vec<TagLabel>` wire
   arg, `TagInput` #416 + casing preservation, render sweeps.
4. `web` browse/routing: `list_*_by_tag(tag: Tag)`,
   `PageSeed::{SiteTag,UserTag} { tag: Tag }`, projector `String→Tag` parse,
   `SiteTagPage`/`UserTagPage` client route-parse.
5. Final sweep + full `validate`; record ship-time actions (ADR promote, #416
   close).

**Key risks/decisions:** the write path is a tightly-coupled type graph (spec
"Requirements") — T2 keeps two documented web adapters that T3 removes, so each
commit is green; the atompub invalid-category **skip** must relocate to
`entry_to_post_fields` (spec R5) or a bad `<category>` fails whole-entry ingest;
`PostTagJson` read-path re-validation (spec R2); `TagValidationError::Invalid`
removal only if unreachable (spec R4); leptosfmt generic-tag mangling is N/A
here (`TagInput` uses no `<ValidatedInput<T>>`, spec R3). No new separable
concerns surfaced (#416 absorbed; "M5" catalog casing pre-exists and stays out).

---

## Global Constraints

_(verbatim intent from the spec's "Global constraints"; every task includes
these)_

- **No wire/schema change; one deliberate behavior change** — the web `TagInput`
  preserves author casing in `tag_display` (spec Decision 4). `Tag`/`TagLabel`
  serialize as plain strings (identical wire). Canonical-slug behavior
  unchanged.
- **`Tag` derive list:**
  `Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, StrNewtype` (keeps
  `Hash`/`Ord`). **`TagLabel` derive list:**
  `Clone, Debug, PartialEq, Eq, StrNewtype` (no `Hash`/`Ord`).
- **One validity source** — `Tag::from_str` (directly or via
  `TagLabel::from_str`); no re-implemented validator survives.
- **Gate** — the pre-commit hook runs `cargo xtask check`; run
  `devtool run -- cargo xtask check` clean first (**jaunder-commit**). Storage
  tests follow the dual-backend template (`CONTRIBUTING.md`). **No
  `Co-Authored-By` trailer.**
- **Governing:** the ADR draft, ADR-0063 §4, ADR-0065, ADR-0019, ADR-0023.

---

## Task 1: `common` — `Tag` adopts `StrNewtype`, add `TagLabel`, sweep `.as_str()`

Spec §A1, §A2, §B5, §F3. Adopting the derive deletes `Tag`'s inherent
`as_str()`, so every `.as_str()` on a `Tag` is a compile error — the sweep is
compiler-forced and lands in this commit. `TagLabel` is added here (pure new
code + unit tests) but not yet threaded; it is `pub`, so no dead-code lint. New
type → TDD; the `Tag` derive swap is a compiler-forced refactor (no new `Tag`
tests — the trailer is tested in `macros/tests/str_newtype.rs`, spec R1).

**Files:**

- Modify: `common/src/tag.rs` (derive swap + delete trailer; add `TagLabel`;
  sweep in-file test `.as_str()` and the `Invalid` assert per R4)
- Modify: `storage/src/posts.rs` (`.bind(tag_slug.as_str())` → `.as_ref()`, all
  tag binds), any other `Tag::as_str` in storage
- Modify: `web/src/feed_events.rs:15-17` (`tag_slugs(...) -> BTreeSet<Tag>`)
- Modify: any remaining `Tag::as_str` flagged by the build across `common`/
  `storage`/`server`/`web` (+ integration tests the gate compiles)

**Interfaces produced:**

- `Tag` with the full generated trailer (`Display`, `AsRef<str>`, `Borrow<str>`,
  `Deref<Target=str>`, `TryFrom<String>`, `From<Tag> for String`,
  `PartialEq<str>`/`<&str>`, validating serde) — **no** inherent `as_str`.
  `FromStr`, `InvalidTag`, `parse_and_validate_tags` (unchanged in T1),
  `TagValidationError`, `MAX_TAGS_PER_POST` unchanged.
- `TagLabel`:

  ```rust
  /// A validated, case-preserving tag label: the author's original spelling of a
  /// tag whose canonical slug ([`Tag`]) is guaranteed valid. Constructed via
  /// [`FromStr`] (trims, rejects empty, validates the slug), stored with casing.
  #[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
  pub struct TagLabel(String);

  #[derive(Debug, Error)]
  #[error("tag must be non-empty and match [a-z0-9][a-z0-9-]* (case preserved)")]
  pub struct InvalidTagLabel;

  impl FromStr for TagLabel {
      type Err = InvalidTagLabel;
      fn from_str(s: &str) -> Result<Self, Self::Err> {
          let trimmed = s.trim();
          // Validity is Tag's rule (the single source); casing is preserved.
          Tag::from_str(trimmed).map_err(|_| InvalidTagLabel)?;
          Ok(TagLabel(trimmed.to_owned()))
      }
  }

  impl TagLabel {
      /// The canonical slug. Infallible: `TagLabel`'s inner string already parses
      /// as a `Tag` (the `from_str` invariant), and both types live in this
      /// module, so the canonical `Tag` is built directly from its lowercase.
      #[must_use]
      pub fn slug(&self) -> Tag {
          Tag(self.0.to_lowercase())
      }
  }

  /// Every canonical slug is a valid label (used by catalog rows with no author
  /// casing, e.g. `list_tags`).
  impl From<Tag> for TagLabel {
      fn from(t: Tag) -> Self {
          TagLabel(t.into())
      }
  }
  ```

- [x] **Step 1: Write `TagLabel` unit tests** (`common/src/tag.rs` `mod tests`):

  ```rust
  #[test]
  fn tag_label_preserves_casing_and_derives_slug() {
      let l: TagLabel = "Rust".parse().unwrap();
      assert_eq!(l.as_ref(), "Rust");         // casing kept
      assert_eq!(l.slug(), "rust");           // canonical slug (PartialEq<&str>)
      assert_eq!(l.to_string(), "Rust");      // Display = label
  }
  #[test]
  fn tag_label_trims_and_rejects_invalid() {
      assert_eq!(" Rust ".parse::<TagLabel>().unwrap().as_ref(), "Rust");
      assert!("".parse::<TagLabel>().is_err());
      assert!("bad tag".parse::<TagLabel>().is_err());   // space
      assert!("-x".parse::<TagLabel>().is_err());        // leading hyphen
  }
  #[test]
  fn tag_label_serde_roundtrips_as_label_string() {
      let l: TagLabel = "Rust".parse().unwrap();
      assert_eq!(serde_json::to_string(&l).unwrap(), "\"Rust\"");
      assert_eq!(serde_json::from_str::<TagLabel>("\"Rust\"").unwrap(), l);
      assert!(serde_json::from_str::<TagLabel>("\"-bad\"").is_err());
  }
  #[test]
  fn tag_from_produces_label() {
      let t: Tag = "rust".parse().unwrap();
      assert_eq!(TagLabel::from(t).as_ref(), "rust");
  }
  ```

  Run: `devtool run -- cargo nextest run -p common tag::` — Expected: FAIL
  (`TagLabel` undefined).

- [x] **Step 2: Adopt the `Tag` derive + add `TagLabel`** in
      `common/src/tag.rs`.
  - `use macros::StrNewtype;`. Replace
    `#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)] #[serde(try_from="String", into="String")]`
    with
    `#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, StrNewtype)]`.
  - Delete `impl TryFrom<String> for Tag`, `impl From<Tag> for String`, the
    inherent `impl Tag { as_str }`, and `impl fmt::Display for Tag`. Drop unused
    `use serde::…` / `use std::fmt` (keep `str::FromStr`). Keep `FromStr`,
    `InvalidTag`, `parse_and_validate_tags`, `TagValidationError`,
    `MAX_TAGS_PER_POST`.
  - Add `TagLabel`, `InvalidTagLabel`, its `FromStr`/`slug`/`From<Tag>` per the
    interface above.

- [x] **Step 3: Sweep the in-file `Tag` tests.**
      `x.parse::<Tag>().unwrap()     .as_str()` → compare via `PartialEq<&str>`
      (`assert_eq!(…, "rust")`); the `tag_normalizes_*`/`as_str` asserts fold
      into `PartialEq`/`to_string` (Display). Leave `parse_and_validate_tags`
      tests (unchanged in T1).

- [x] **Step 4: Sweep production `Tag::as_str`.** `storage/src/posts.rs` tag
      binds `.bind(x.as_str())` → `.bind(x.as_ref())`;
      `web/src/feed_events.rs:15-17` →
      `fn tag_slugs(tags: &[PostTag]) ->     BTreeSet<Tag> { tags.iter().map(|t| t.tag_slug.clone()).collect() }`
      (verify `enqueue_feed_events` already takes `&BTreeSet<Tag>`).
      `rg 'as_str\(\)'` over `storage`/`web`/`server`/`common` and fix each
      `Tag` site the build flags.

- [x] **Step 5: Gate.** `devtool run -- cargo xtask check` — Expected: PASS.

- [x] **Step 6: Commit.**
      `git add common/src/tag.rs storage/src/posts.rs web/src/feed_events.rs` (+
      any swept file) —
      `refactor(common): Tag adopts StrNewtype; add TagLabel; sweep .as_str()`

---

## Task 2: `storage` + `server` + `common` DTOs — thread `Tag`/`TagLabel` through the storage tag surface

Spec §A3, §A4, §A5, §B1–B5, §D1–D2, R2, R4, R5. Types the write/read tag surface
end to end. `parse_and_validate_tags` is retyped here; the web `#[server]`
bodies keep a **temporary wire-boundary parse** (`Vec<String>` →
`Vec<TagLabel>`) so they stay green **and preserve the current invalid-tag →
`Validation` behavior** — T3 removes it when the wire arg itself becomes typed
(the ADR-0065 decode shift). Storage tests are dual-backend.

**Files:**

- `common/src/tag.rs` (`parse_and_validate_tags` → `Vec<TagLabel>`; tests),
  `common/src/feed/metadata.rs:23` (`FeedItem.tags: Vec<TagLabel>` +
  json/rss/feed test fixtures), `common/src/atompub/service.rs:25`
  (`CollectionDecl.categories: Vec<Tag>`)
- `storage/src/posts.rs` (`PostTag.tag_display`, `PostTagDiff.to_add`,
  `post_tag_diff`, `apply_post_tag_diff`, `tag_post` trait, tests `post_tag`
  helper/`post_tag_diff` test), `storage/src/sqlite/posts.rs` +
  `postgres/posts.rs` (`tag_post`), `storage/src/helpers.rs:136-158`
  (`PostTagJson` + collapse)
- `server/src/atompub/mapping.rs` (`PostFields.categories:24`,
  `entry_to_post_fields:85` filter_map, `PostTag` construction `:503`, category
  tests `:390,405`), `server/src/atompub/posts.rs` (`apply_categories:282`, ETag
  read `:89`, `mk_tag:554`), `server/src/atompub/service.rs:32-45` (categories
  from `list_tags` → `Vec<Tag>`)
- `web/src/posts/mod.rs` (create_post `:241,291`, update_post `:404,435`
  **temporary body adapter**), `web/src/posts/server.rs:40-47`
  (`post_tags_to_summaries` **temporary `.to_string()` adapter**)

**Interfaces produced:**

- `parse_and_validate_tags(Vec<TagLabel>) -> Result<Vec<TagLabel>, TagValidationError>`
  — dedup by `.slug()` (keep first label) + `MAX_TAGS_PER_POST` cap; `Invalid`
  variant removed iff unreachable (R4).
- `PostTag { post_id, tag_id, tag_slug: Tag, tag_display: TagLabel }`;
  `PostTagDiff { to_add: Vec<&TagLabel>, to_remove: Vec<&Tag> }`.
- `post_tag_diff(existing: &[PostTag], desired: &[TagLabel]) -> PostTagDiff`;
  `apply_post_tag_diff(.., desired: &[TagLabel])`;
  `tag_post(.., tag: &TagLabel)`;
  `PostTagJson { tag_id: i64, tag_slug: Tag, tag_display: TagLabel }`.
- `PostFields.categories: Vec<TagLabel>`;
  `apply_categories(.., desired: &[TagLabel])`;
  `CollectionDecl.categories: Vec<Tag>`; `FeedItem.tags: Vec<TagLabel>`.

- [x] **Step 1: Retype `parse_and_validate_tags`** (`common/src/tag.rs`, spec
      Decision 6) to
      `Vec<TagLabel> -> Result<Vec<TagLabel>, TagValidationError>`: dedup on
      `TagLabel::slug` via `HashSet<Tag>` (keep the first occurrence's label),
      enforce the cap. Update its tests: dedup `["Rust","rust"]` → one
      `TagLabel("Rust")`; `MAX+1` distinct → `TooMany`. Remove the invalid-token
      tests (input is pre-validated) and — if
      `rg     'TagValidationError::Invalid'` shows no other constructor — the
      `Invalid` variant (R4).

- [x] **Step 2: `common` DTOs.** `FeedItem.tags: Vec<TagLabel>` (+ json/rss/
      metadata test fixtures build labels; feed JSON output byte-identical);
      `CollectionDecl.categories: Vec<Tag>`.

- [x] **Step 3: `storage` structs + read path.**
      `PostTag.tag_display: TagLabel`;
      `PostTagJson { tag_slug: Tag, tag_display: TagLabel }` and collapse
      `parse_post_tags_json` to a field-move map (delete the manual `.parse()` +
      ad-hoc `Decode` map, `helpers.rs:147-150`). Fix the `post_tag` test helper
      (`posts.rs:2228`) to build a `TagLabel`. **Dual-backend read test (B4):**
      a stored tagging reads back with slug + label intact on sqlite **and**
      postgres.

- [x] **Step 4: `storage` write signatures.**
      `PostTagDiff.to_add: Vec<&TagLabel>`;
      `post_tag_diff(desired: &[TagLabel])` with internal `HashSet<Tag>` (drop
      the `.to_string()` round-trips and the `filter_map(Tag::from_str)` skip —
      a `&TagLabel` is valid); rename the diff test to `…_adds_removes_keeps`
      (the skips-invalid subcase is now unconstructible → relocates to Step 6,
      R5). `apply_post_tag_diff(desired: &[TagLabel])`.
      `tag_post(tag: &TagLabel)` in the trait + `sqlite`/`postgres` impls:
      derive the slug via `tag.slug()`, bind the label via `tag.as_ref()` — no
      internal `parse::<Tag>()`.

- [x] **Step 5: `server` atompub emit/service.**
      `apply_categories(desired:     &[TagLabel])` (calls unchanged —
      `&fields.categories` is now `&Vec<TagLabel>`); ETag content read
      `posts.rs:89` `t.tag_display.as_ref()`; the **emit** `atom:category`
      builder `mapping.rs:148-155`
      `Category { term:     t.tag_display.to_string(), .. }` (the
      `Category.term` field is `String`); `mk_tag` builds a `TagLabel`;
      `mapping.rs:503` `PostTag { tag_display:     <TagLabel>, .. }`;
      `service.rs:32-45` push the `Tag` (drop `.to_string()`) into `Vec<Tag>`
      (`media` stays empty). **Verify:** service-doc + entry `atom:category`
      output byte-identical.

- [x] **Step 6: `server` atompub ingest + skip test (R5).**
      `PostFields.categories:     Vec<TagLabel>`; `entry_to_post_fields`
      (`mapping.rs:85`) `filter_map(|c|     c.term()…parse::<TagLabel>().ok())`
      — **this is where the invalid-`<category>` silent-skip now lives** (Step 4
      removed `post_tag_diff`'s). Update category tests `:390,405` to
      `Vec<TagLabel>`. **New test:** an entry with one valid + one invalid
      category yields exactly one `TagLabel` (not an ingest failure).

- [x] **Step 7: web temporary adapters (removed in T3).**
  - `create_post`/`update_post` (`mod.rs:233,385`): keep the wire arg
    `Option<Vec<String>>`; at the top of the body **filter out empty/whitespace
    tokens** (preserving today's empty-drop, see the empty-token note below)
    then parse each to `TagLabel`, mapping a parse failure to
    `WebError::validation(...)` (`error.rs:43` — **not** the deleted
    `TagValidationError::Invalid`), yielding `Vec<TagLabel>` fed to
    `parse_and_validate_tags`. The `tag_post` loop (`:291`) passes `&label`;
    `apply_post_tag_diff` (`:435`) passes `&new_tags` (`Vec<TagLabel>`). Mark
    both `// TEMP(T3):`.
    - **Empty-token note (second minor behavior change, non-user-visible).**
      Today `parse_and_validate_tags` _drops_ empty/whitespace tokens
      (`tag.rs:348` test). A `TagLabel` cannot be empty, so from T3's typed wire
      (`Vec<TagLabel>`) onward an empty token is **rejected at arg-decode**, not
      dropped (ADR-0065 — only reachable by a non-browser client; the SPA
      `TagInput` never emits empty chips). To keep T2 and T3 behavior identical,
      the T2 adapter **filters empties before parsing** — matching T3, where the
      browser never sends them and a malformed client hits the decode path. The
      `tag.rs:348` empty-drop test is removed with the other raw-token tests
      (its input type no longer exists); TagLabel's own empty-rejection is
      covered in T1.
  - `post_tags_to_summaries` (`server.rs:40-47`):
    `display: t.tag_display .to_string()` (`TagSummary` still `String`).
    `// TEMP(T3):`.

- [x] **Step 8: Gate.** `devtool run -- cargo xtask check` — Expected: PASS
      (dual-backend storage tests green; wire unchanged).

- [x] **Step 9: Commit.**
      `refactor(storage,server): thread Tag/TagLabel through the tag storage surface + atompub`

---

## Task 3: `web` — `TagSummary`/wire typed, `TagInput` #416 + casing preservation

Spec §E1–E3, §F2 (render_tag_list), §G1, Decision 4/5. Types the web write DTO
and wire, removes T2's adapters, and reworks `TagInput` (delete the
re-implemented validator; preserve author casing). `TagInput` behavior change →
TDD for the new casing + agreement tests; the DTO retype is compiler-forced.

**Files:**

- `web/src/tags/mod.rs` (`TagSummary` `:26`; `list_tags` builder `:44`;
  **delete** `is_valid_tag_slug` `:64` + `normalize_tag_token` `:75` + their
  tests)
- `web/src/posts/mod.rs` (create/update wire arg `:233,385` →
  `Option<Vec<TagLabel>>`; drop T2 body adapter), `web/src/posts/server.rs`
  (`post_tags_to_summaries`; drop T2 `.to_string()`)
- `web/src/render/mod.rs:532-542` (`render_tag_list` `.as_ref()`/`.to_string()`
  sweep), `web/src/pages/ui.rs` (`TagInput` `:1150-1360`; `TagList` chip render)
- Tests: `web/src/tags/mod.rs` / `pages/ui.rs` (#416 agreement + casing);
  `server/tests/web/web_tags.rs` display assertions if affected

**Interfaces produced:**

- `TagSummary { slug: Tag, display: TagLabel }` (still
  `Serialize/Deserialize/ PartialEq/Eq`); `create_post`/`update_post`
  `tags: Option<Vec<TagLabel>>`.

- [x] **Step 1: `TagInput` tests (TDD)** — in `web/src/tags` (host-tested pure
      helper) or `pages/ui.rs`:
  - **Agreement (#416):** a token divergent under the old split validates
    identically client/server now — e.g. `"Rust".parse::<TagLabel>().is_ok()`
    and `" ab ".parse::<TagLabel>().unwrap().as_ref() == "ab"`.
  - **Casing:** committing "Rust" yields
    `TagSummary { slug: "rust", display: "Rust" }` (assert `slug`/`display`).
    Run: `devtool run -- cargo nextest run -p web tags::` — Expected: FAIL.

- [x] **Step 2: Type `TagSummary` + `list_tags` builder + drop T2 adapters.**
      `TagSummary { slug: Tag, display: TagLabel }`. `list_tags`
      (`tags/mod.rs:44`):
      `slug: rec.tag_slug.clone(), display: TagLabel::from(rec.tag_slug)`
      (catalog has no casing → label mirrors the slug, unchanged behavior).
      `post_tags_to_summaries`: move the typed fields (drop `.to_string()`).
      create/update: wire arg `Option<Vec<TagLabel>>`; delete the T2 body parse;
      the `tag_post` loop/`apply_post_tag_diff` now consume `Vec<TagLabel>`
      directly. (Per ADR-0065 the invalid-arg path is now arg-decode; the cap
      still surfaces as `Validation` via `parse_and_validate_tags`.)

- [x] **Step 3: Render sweeps.** `render_tag_list` (`render/mod.rs:538,542`):
      `escape_html(tag.slug.as_ref())`, `escape_html(tag.display.as_ref())`;
      `/tags/{slug}` hrefs use `tag.slug.as_ref()`/`Display`. `TagList` chips
      (`ui.rs:59,1284-1351`) render `{tag.display.to_string()}` (a newtype is
      not `IntoRender`); hidden-input `value=tag.display.to_string()`.

- [x] **Step 4: `TagInput` — delete the re-impl; preserve casing (#416 +
      Decision 4).** In the commit path (`ui.rs:1224-1246`): drop
      `normalize_tag_token`/`is_valid_tag_slug`; match
      `input_text.get()     .parse::<TagLabel>()`:

  ```rust
  match input_text.get().parse::<TagLabel>() {   // trims + validates, KEEPS case
      Ok(label) => {
          let slug = label.slug();
          tags.update(|t| {
              if !t.iter().any(|x| x.slug == slug) {       // dedup by canonical slug
                  t.push(TagSummary { slug, display: label });
              }
          });
          input_text.set(String::new()); error.set(None); /* …close suggest… */
      }
      Err(e) => error.set(Some(e.to_string())),  // InvalidTagLabel's Display
  }
  ```

  Keep the suggestion-prefix lowercase (`on_input` `val.trim().to_lowercase()`,
  `:1173` — case-insensitive `LIKE`). Delete `is_valid_tag_slug`/
  `normalize_tag_token` + their `tags/mod.rs` tests. Ensure `InvalidTagLabel`'s
  `Display` (Task 1) keeps the e2e `.j-tag-error` assertion satisfiable — align
  the e2e string to it in Task 5 if the wording differs.

- [x] **Step 5: Gate.** `devtool run -- cargo xtask check` — Expected: PASS. Fix
      any `server/tests/web/web_tags.rs` display assertion the retype flags
      (slugs unaffected; `display` now `TagLabel` — `list_tags` display still
      mirrors slug, so those asserts hold).

- [x] **Step 6: Commit.**
      `feat(web): typed TagSummary/wire (Tag+TagLabel); TagInput preserves casing (#416)`

---

## Task 4: `web` — browse args + `PageSeed` routing + projector parse

Spec §F1, §F2, Decision 5, Decision 7 (projector `Path<String>`). Types the tag
**identity** on the browse/read path; the raw HTTP extractor stays `String` and
is parsed inside (projector precedent). Behavior-preserving except a malformed
tag in the SPA route/projector now skips the fetch / serves the shell as it
already does.

**Files:**

- `web/src/posts/listing.rs` (`fetch_posts_by_tag:236`,
  `fetch_user_posts_by_tag :269`, `list_posts_by_tag:301`,
  `list_user_posts_by_tag:324`)
- `web/src/render/mod.rs:58-72` (`PageSeed::{SiteTag,UserTag}.tag: Tag`),
  `:201,209` (drop the now-redundant `tag.parse::<Tag>()`), and the in-file test
  fixtures `:1001,1024,1037,1109`
  (`PageSeed::SiteTag/UserTag { tag: "rust".into(), .. }` →
  `"rust".parse::<Tag>().unwrap()` — `Tag` has no `From<&str>`)
- `web/src/pages/posts.rs` (`SiteTagPage:1087`, `UserTagPage:1249` — read typed
  seed; CSR route-param parse `params.get("tag").parse::<Tag>()`, invalid → skip
  fetch)
- `server/src/projector/mod.rs` (`site_tag:239`, `user_tag:264` — parse
  `Path<String>` → `Tag`, invalid → shell)

**Interfaces produced:**

- `list_posts_by_tag(tag: Tag)`, `list_user_posts_by_tag(.. tag: Tag)`;
  `fetch_posts_by_tag(.. tag: &Tag)`, `fetch_user_posts_by_tag(.. tag: &Tag)`;
  `PageSeed::SiteTag { tag: Tag, page }`,
  `PageSeed::UserTag { username, tag: Tag, page }`.

- [x] **Step 1: `listing.rs`.** `fetch_*_by_tag(tag: &Tag)` — delete the
      internal `tag.trim().parse::<Tag>()` (`:244,278`).
      `#[server] list_*_by_tag(tag: Tag)` — pass `&tag`.

- [x] **Step 2: `PageSeed` + `render`.** `PageSeed::SiteTag { tag: Tag, page }`,
      `PageSeed::UserTag { username, tag: Tag, page }`. In `render_discovery`
      (`:201,209`) drop `tag.parse::<Tag>()` → use the typed `tag` (clone) for
      `FeedSurface::{SiteTag,UserTag}`. `render_head`/`render_body` matches use
      `tag` via `Display` (unchanged). Confirm the `PageSeed` serde round-trip
      test (`:1267`) still passes (`Tag` serializes as the same string).

- [x] **Step 3: Projector (Decision 7).** `site_tag`/`user_tag`
      (`projector/mod.rs:239,264`): keep `Path<String>`; parse
      `tag.parse::<Tag>()`, on `Err` return `shell_response` (the existing
      "unparseable tag is never public content" branch). `Tag::from_str`
      lowercases → drop the explicit `to_lowercase()`. Build
      `PageSeed::SiteTag { tag, page }` with the parsed `Tag`. Add the
      Decision-7 justification comment on the extractor.

- [x] **Step 4: SPA pages.** `SiteTagPage`/`UserTagPage` (`posts.rs:1087,1249`):
      read the typed `PageSeed` `tag`; on the CSR route-param path parse
      `params.get("tag").and_then(|s| s.parse::<Tag>().ok())` and skip the fetch
      (client 404) when it fails — mirror the #408 `PostPage` route-parse.

- [x] **Step 5: Gate + affected e2e.** `devtool run -- cargo xtask check` —
      Expected: PASS. Then `devtool run -- cargo xtask validate` — e2e green for
      tag-chip navigation + `/tags/:slug` listing (watch the known
      `posts.spec.ts` csr flake; re-run once).

- [x] **Step 6: Commit.**
      `refactor(web): thread Tag through browse args + PageSeed routing + projector parse`

---

## Task 5: Final sweep + ship-time notes

Spec §H1. Confirms the north star and records what `jaunder-ship` must do.

- [x] **Step 1: North-star sweep.**
      `rg -n 'tag.*: ?String|tags: ?Vec<String>|     tag.*&str|HashSet<String>|BTreeSet<String>'`
      across `common storage server     web host` (all five crates — §C1
      confirms `host` has no tag-value sites) for tag values. The **only**
      permitted hits: `list_tags` `prefix` (`web/src/tags/mod.rs`) and the
      projector `Path<String>` (`server/src/     projector/mod.rs`) — each
      carrying its Decision-7 justification comment. Anything else → thread it.
- [x] **Step 2: Full gate.** `devtool run -- cargo xtask validate` — Expected:
      PASS (static + coverage + e2e).
- [x] **Step 3: Ship notes (for `jaunder-ship`, not committed here).**
      `cargo     xtask adr promote` numbers
      `docs/adr/drafts/tag-identity-label-split.md`; close **#416** as absorbed;
      the PR references #409.

---

## Self-review

- **Spec coverage:** §A1/§A2/§B5/§F3(tag_slugs) → T1; §A3/§A4/§A5 → T2; §B1–B4 →
  T2; §C (host, no change) → noted (T5 sweep confirms); §D1/§D2 → T2; §E1–E3 →
  T3; §F1 → T4, §F2(render_tag_list) → T3 + §F2(PageSeed/projector/pages) → T4;
  §G1 → T3; §H1 → T5; §I1 → T5 ship note. Decisions 1–7 all land (D4 casing →
  T3; D5 typed wire → T3 create/update + T4 browse; D6 → T2; D7 → T2 PostTagJson
  / T4 projector). Risks: R1(T1), R2(T2 S3), R3(N/A), R4(T2 S1), R5(T2 S6).
- **Per-commit green:** T1 compiler-forced sweep in one commit; T2
  self-contained with two `// TEMP(T3)` web adapters (both removed in T3 S2);
  T3/T4 each leave the tree green; T5 verify-only.
- **No placeholders:** each step names exact files/symbols/lines and full Rust
  for `TagLabel`, the retyped `parse_and_validate_tags` (by contract), the
  `TagInput` commit path, and the projector parse. Line numbers are hints — the
  implementer greps the symbol to confirm before editing.
- **Type consistency:** `TagLabel::slug()->Tag` (T1) feeds dedup (T2/T3);
  `parse_and_validate_tags: Vec<TagLabel>` (T2) fed by the T2 body adapter then
  the T3 typed wire arg; `tag_post(&TagLabel)`/`post_tag_diff(&[TagLabel])` (T2)
  consumed by T3's `Vec<TagLabel>` write path; `TagSummary{Tag,TagLabel}` (T3)
  read by `render_tag_list`/`TagInput`; `PageSeed{tag:Tag}` (T4) fed by the
  projector parse and read by the SPA pages.
