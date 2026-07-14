# `Tag` vertical (#409) — thread `Tag` + `TagLabel` everywhere a tag travels

> Spec of record for issue
> [#409](https://github.com/jaunder-org/jaunder/issues/409) (Tag half of the
> #404 umbrella; absorbs bug
> [#416](https://github.com/jaunder-org/jaunder/issues/416)). Design settled in
> interview 2026-07-14. Decision record:
> `docs/adr/drafts/tag-identity-label-split.md`.

## North star

**No tag value anywhere in the source is represented by anything other than a
tag newtype** — no bare `String`/`&str`/`Vec<String>`/`HashSet<String>` for a
tag value across `common`/`storage`/`host`/`server`/`web`. A tag has two facets,
so "tag newtype" means one of **two composable types** (Decision 1):

- **`Tag`** — the canonical, lowercased slug: the tag's _identity_ (browse key,
  catalog key, dedup key, SQL key). Adopts `#[derive(StrNewtype)]`.
- **`TagLabel`** — the author's case-preserving _label_ for one tagging;
  validated (its slug is guaranteed to parse) but not lowercased. Adopts
  `#[derive(StrNewtype)]` and exposes `slug(&self) -> Tag`.

Paired only where both travel:
`PostTag { tag_slug: Tag, tag_display: TagLabel }`,
`TagSummary { slug: Tag, display: TagLabel }`.

## Global constraints

_(every task implicitly includes these)_

- **No wire/schema change; one deliberate behavior change.** `Tag` and
  `TagLabel` each serialize as a plain string (the `StrNewtype` serde bridge),
  identical to the current `String` wire; no serialized shape, DB column, or
  migration changes. Canonical-slug behavior (browse, dedup, autocomplete,
  atompub `atom:category` slugging) is byte-for-byte unchanged. **The one
  intentional change** (Decision 4, newly in scope): the web `TagInput` now
  **preserves the author's casing** in `tag_display` instead of lowercasing it —
  so a web-composed "Rust" renders on the post as "Rust". This realizes the
  casing the `tag_display` column exists to hold, matching atompub ingest (which
  already preserves it), and is stated here so the ship conformance review reads
  it as intended, not a regression. It is a no-op for existing rows (only new
  web-created tags are affected) and for the all-lowercase e2e corpus. **A
  second, non-user-visible nuance** (Decision 6): an empty tag token, today
  silently dropped, is now rejected at the typed-wire arg-decode — reachable
  only by a non-browser client (the SPA never sends empty tokens), so not a
  user-facing change.
- **`Tag` derive list:**
  `Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, StrNewtype` — **keeps
  `Hash`/`Ord`** (unlike `Slug`): `Tag` is a `HashSet`/`BTreeSet`/`HashMap` key
  (dedup, feed enqueue). Hand-written `FromStr` unchanged. The unrelated
  `TagValidationError` helper is retyped, not deleted (Decision 6).
- **One validity source.** Every tag validity decision funnels through `Tag`'s
  rule — directly (`Tag::from_str`) or via `TagLabel::from_str` (which delegates
  to it). No re-implemented validator survives (retires `is_valid_tag_slug`,
  #416).
- **Gate:** the pre-commit hook runs `cargo xtask check`; run it clean first
  (**jaunder-commit**). Storage changes follow the dual-backend template
  (`CONTRIBUTING.md`; both sqlite + postgres impls change together). **No
  `Co-Authored-By` trailer.**
- **Governing:** `docs/adr/drafts/tag-identity-label-split.md` (this vertical's
  decision), ADR-0063 (§4 boundary rule), ADR-0065 (typed wire args + client
  pre-validation), ADR-0023 (atompub emit-only wire extensions), ADR-0019
  (dual-dialect storage).

## Decisions

1. **Two newtypes, not one.** `Tag` (identity) + `TagLabel` (label), per the ADR
   draft. Rejected: a two-member `Tag { canonical, display }` (breaks
   `StrNewtype`; forces a meaningless label into canonical-only contexts) and a
   bare-`String` label (unvalidated boundary value).

2. **`TagLabel::from_str`** trims, rejects empty, and validates that the trimmed
   input parses as a `Tag` (`Tag::from_str(trimmed).is_ok()`); it stores the
   trimmed input **with its original casing**. Its `FromStr::Err` is a dedicated
   `InvalidTagLabel` whose `Display` is the client-facing message.
   `slug(&self) -> Tag` is **infallible** — `TagLabel` lives in
   `common/src/tag.rs` beside `Tag`, so it constructs the canonical `Tag` from
   its own already-valid inner string directly (no re-parse, no `unwrap`).
   `impl From<Tag> for TagLabel` (every canonical slug is a valid label) serves
   catalog rows that have no author casing.

3. **Identity is always the slug.** Dedup and equality _as tags_ are by
   `Tag`/`TagLabel::slug`, retaining `Tag`'s `Hash`/`Ord`. `TagLabel`'s derived
   `PartialEq`/`Eq` compare the raw label string (so `TagSummary` equality in
   tests is exact); code that means "same tag" compares slugs explicitly, never
   raw labels.

4. **Web `TagInput` preserves author casing; validity is `TagLabel::from_str`
   (#416 + newly-in-scope casing change).** The commit path feeds the **raw
   (trimmed) input** to `TagLabel::from_str` — no lowercasing — so the chip and
   the stored `tag_display` keep the author's casing; dedup and the slug are by
   `label.slug()`. `is_valid_tag_slug` **and** the commit-path lowercasing
   (`normalize_tag_token`, now dead) are deleted; the autocomplete-prefix
   lowercase (the inline `val.trim().to_lowercase()` in `on_input`, for the
   case-insensitive `LIKE` query) stays. Client and server accept identical
   inputs — both route through `TagLabel::from_str` (a regression test pins a
   previously-divergent case, e.g. an uppercase or leading-space token). The
   user-facing invalid-tag error stays a controlled message; the e2e "invalid
   tag" assertion is aligned to `InvalidTagLabel`'s `Display`.

5. **Typed wire boundaries (ADR-0065).**
   - **Browse** `#[server] list_posts_by_tag` / `list_user_posts_by_tag` take
     `tag: Tag`. The SPA `/tags/:slug` (and `/~user/tags/:slug`) route string is
     parsed client-side into `Tag`; an unparseable tag skips the fetch (client
     404), mirroring the #408 `PostPage` slug route-parse. The tag arrives from
     a link built off an existing `Tag`, so the parse is defense-in-depth.
   - **Create/update** `tags` arg is `Option<Vec<TagLabel>>`. The `TagInput`
     pre-validates each label via `TagLabel::from_str` (Decision 4). Per
     ADR-0065, an invalid label now fails at arg _decode_ (generic transport
     error, only reachable by a non-browser client); the `MAX_TAGS_PER_POST` cap
     still surfaces as a controlled `Validation` error in the body (Decision 6).

6. **`parse_and_validate_tags` retyped, not deleted.** Signature becomes
   `parse_and_validate_tags(Vec<TagLabel>) -> Result<Vec<TagLabel>, TagValidationError>`:
   its per-token _parse/validate_ job is now done by `TagLabel`'s
   deserialize/construction, so it only **dedups by slug (keeping the first
   occurrence's label)** and enforces `MAX_TAGS_PER_POST`. `TagValidationError`
   keeps `TooMany`; its `Invalid` variant is removed iff no caller can still
   reach it (confirm in-task). `MAX_TAGS_PER_POST` unchanged. **Empty tokens:**
   today the helper silently _drops_ empty/whitespace tokens; a `TagLabel`
   cannot be empty, so an empty token is now **rejected** (at the typed-wire
   arg-decode) rather than dropped — a non-user-visible consequence of the typed
   wire (the SPA `TagInput` never emits empty chips; only a non-browser client
   could send one, the ADR-0065 decode path). See the behavior-change note in
   the constraints.

7. **Deliberately-untyped boundaries (justified in a comment at each site) —
   three.**
   - `list_tags(prefix: Option<String>)` — a _search fragment_ (a partial,
     matched with SQL `LIKE prefix%`; `None`/empty ⇒ all tags), not a complete
     tag value; typing it `Tag` would misrepresent a query input.
   - The `server` feed router / projector path `/tags/{tag}` (and the per-user
     form) — the raw HTTP `Path<String>` extractor stays `String`, parsed to
     `Tag` inside the handler so a malformed tag serves the SPA shell / a
     not-found feed rather than an axum 400 before the handler runs (the
     projector-vs-atompub boundary split, ADR-0063 §4; mirrors #408 Decision 1).
   - The atom `Entry`/`Category` model (`common/src/atompub/entry.rs`) — atom
     `<category term>` values are arbitrary RFC-4287 protocol strings (not
     constrained to our tag rule), so the wire model holds them as `String`;
     `entry_to_post_fields` is the boundary that parses each conforming term
     into a `TagLabel` (skipping the rest, R5). (Surfaced in review; unlike
     `PostTagJson` below this is _external_ input, so it stays `String`.)

   `PostTagJson` is **typed** (not kept as `String`): it is trusted internal
   data deserialized via serde, so deserializing straight into the newtypes _is_
   "parse at the boundary" (ADR-0063) — see B4.

## Requirements by surface (each an observable acceptance criterion)

### §A `common`

- **A1** `Tag` derives `StrNewtype`; the hand-written trailer (`TryFrom`,
  `From<Tag> for String`, inherent `as_str`, `Display`) and the
  `#[derive(Serialize, Deserialize)]` + `#[serde(try_from, into)]` bridge are
  gone; derive list keeps `Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord`;
  `FromStr` byte-identical. The in-file test module is swept in the same commit
  (compiler-forced): `.as_str()` asserts → `PartialEq<&str>`
  (`tag.rs:169,199,300,…`), and the `TagValidationError::Invalid` assertion
  (`tag.rs:385-395`) is reconciled with the Decision-6 retype (R4). **Verify:**
  `common` compiles (incl. its tests); `tag.rs` has no
  `impl TryFrom`/`impl From`/ `impl fmt::Display`/inherent `as_str` for `Tag`.
- **A2** `TagLabel` exists in `common/src/tag.rs` per Decision 2, deriving
  `Clone, Debug, PartialEq, Eq, StrNewtype` (no `Hash`/`Ord` — never a key),
  with `InvalidTagLabel`, hand `FromStr`, `slug() -> Tag`, `From<Tag>`.
  **Verify:** unit tests — `"Rust".parse::<TagLabel>()` preserves `"Rust"` and
  `.slug() == "rust"`; `" Rust ".parse()` trims to `"Rust"`; `"".parse()` and
  `"bad tag".parse()` error; serde round-trips as `"Rust"`; a `TagLabel`
  deserialized from `"-bad"` is rejected.
- **A3** `parse_and_validate_tags` retyped per Decision 6. **Verify:** dedups
  `["Rust","rust"]` to one `TagLabel("Rust")`; `MAX_TAGS_PER_POST + 1` distinct
  tags returns `TooMany`.
- **A4** `common::feed::metadata::FeedItem.tags: Vec<TagLabel>` (feed-output
  category terms). **Verify:** `common`/feed tests build items from labels; JSON
  feed output byte-identical.
- **A5** `common::atompub::service::CollectionDecl.categories: Vec<Tag>` (the
  advertised canonical catalog). This is **live**, not a placeholder: the writer
  `server/src/atompub/service.rs:32-37` populates `posts_collection.categories`
  from `posts.list_tags(None, 100).map(|t| t.tag_slug…)` — drop the
  `.to_string()` and push the `Tag`; `write_collection` emits each term via
  `.as_ref()`. `media_collection.categories` stays empty (`service.rs:51`). Test
  fixtures that build categories from `"rust".into()` become
  `"rust".parse::<Tag>().unwrap()` (`Tag` has no `From<&str>`). **Verify:**
  service-doc `atom:category` output byte-identical.

### §B `storage`

- **B1** `PostTag.tag_display: TagLabel`; `PostTagDiff.to_add: Vec<&TagLabel>`.
- **B2** `post_tag_diff(existing: &[PostTag], desired: &[TagLabel])` and
  `apply_post_tag_diff(..., desired: &[TagLabel])`; the internal slug sets are
  `HashSet<Tag>` (via `.slug()` / `tag_slug`), dropping every `.to_string()`
  round-trip and the `filter_map(Tag::from_str…)` skip (a `&TagLabel` is already
  valid). The invalid-token subcase of the existing
  `post_tag_diff_adds_removes_keeps_and_skips_invalid` test (`posts.rs:2238`)
  becomes **unconstructible** (you cannot build an invalid `TagLabel`), so it
  relocates upstream to the atompub ingest filter (§D1); rename the storage test
  to `…_adds_removes_keeps`. **Verify:** `post_tag_diff` unit test — re-applying
  an existing tag with different casing is a no-op; add/remove keyed on slug.
- **B3** Trait + both impls: `tag_post(post_id, tag: &TagLabel)`. The impl
  derives the slug via `tag.slug()` and stores `tag.as_ref()` (the label) — no
  internal `parse::<Tag>()`. `untag_post`/`list_*_by_tag` already take `&Tag`
  (unchanged).
- **B4** `PostTagJson { tag_id: i64, tag_slug: Tag, tag_display: TagLabel }`
  (`helpers.rs:136`): serde validates both fields at the DB-read boundary, so
  `parse_post_tags_json` collapses to a field-move map — the hand-written
  `tag_slug.parse()` + ad-hoc `sqlx::Error::Decode` map is deleted, and
  `tag_display` (today passed through **unvalidated**) is now validated.
  **Verify:** a stored tagging reads back with slug and label intact across
  **both** backends; a `Vec<PostTagJson>` with an invalid slug still surfaces a
  decode error.
- **B5** Every `.as_str()` on a `Tag` (SQL binds, summaries) becomes `.as_ref()`
  (or `PartialEq`/`Display` as fits); `.bind(...)` stays valid via
  `Deref<str>`/`AsRef<str>`. **Verify:** `rg 'tag_slug\.as_str\(\)'` and
  `\.tags?.*\.as_str\(\)` over `storage/` returns nothing.

### §C `host`

- **C1** No tag-value sites (only `InvalidTag`/`TagValidationError` type
  references in error mapping) — **no change**; noted so the reviewer confirms
  the crate was surveyed, not skipped.

### §D `server` (atompub)

- **D1** `PostFields.categories: Vec<TagLabel>` (atom `<category term>` ingest =
  author labels). `entry_to_post_fields` is **infallible**, so it must
  `filter_map(|term| term.parse::<TagLabel>().ok())` — this is where today's
  silent-skip of an invalid `<category>` term must now live, because B2 removes
  `post_tag_diff`'s own skip (R5). The `storage::PostTag` construction uses the
  typed pair. **Verify:** the category-extraction test asserts `Vec<TagLabel>`;
  **a new test pins the skip** — an entry with one valid + one invalid category
  yields exactly one `TagLabel` (not an ingest failure); atompub round-trip e2e
  green.
- **D2** `apply_categories(desired: &[TagLabel])`; the ETag content tag list is
  built from `PostTag.tag_display` via `.as_ref()` (label string). **Verify:**
  atompub ETag/content tests unchanged in output.
- **D3** `server` router `/tags/{tag}` feed extractors: `String` retained,
  parsed to `Tag` inside the handler with the Decision-7 justification comment.

### §E `web` DTOs + server fns

- **E1** `TagSummary { slug: Tag, display: TagLabel }`. `list_tags` builds it
  from a `TagRecord` (canonical only): `display` via
  `TagLabel::from(rec.tag_slug…)`, `slug` the `Tag` — preserving today's
  "display == slug" autocomplete behavior.
- **E2** `post_tags_to_summaries(Vec<PostTag>) -> Vec<TagSummary>` moves the
  typed fields directly (no `.to_string()`).
- **E3** `create_post` / `update_post` `#[server]` arg
  `tags: Option<Vec<TagLabel>>` (Decision 5); body calls the retyped
  `parse_and_validate_tags`. `PostResponse.tags` / `TimelinePostSummary.tags`
  stay `Vec<TagSummary>`. **Verify:** create→fetch a post with tag "Rust"
  returns `display: "Rust"`, `slug: "rust"`; wire JSON unchanged.

### §F `web` browse / routing / render

- **F1** `list_posts_by_tag(tag: Tag)`, `list_user_posts_by_tag(tag: Tag)`
  (`web/src/posts/listing.rs:301,324`); `fetch_posts_by_tag(tag: &Tag)`,
  `fetch_user_posts_by_tag(tag: &Tag)` (`listing.rs:236,269`) — the internal
  `tag.trim().parse::<Tag>()` (`:244,278`) is removed. The **projector**
  (`server/src/projector/mod.rs`) is the server producer: `site_tag` (`:239`) /
  `user_tag` (`:264`) keep the raw `Path<String>` extractor (Decision 7 analog
  to the #408 slug projector) but parse it to `Tag` in the handler body; on
  parse failure they serve the shell (the existing "unparseable tag is never
  public content — let the client route it" branch, `:259`). `Tag::from_str`
  lowercases, subsuming the explicit `tag.to_lowercase()` (`:247,271`) — the
  projected heading stays lowercase, unchanged.
- **F2** The tag carriers are `PageSeed::SiteTag { tag: Tag, page }` and
  `PageSeed::UserTag { username, tag: Tag, page }`
  (`web/src/render/mod.rs:64-72`) — **not** a `Route` enum (none exists).
  `PageSeed` is `Serialize`/`Deserialize` (the SSR hydration seed); `Tag`
  serializes as the same string, so the embedded seed JSON is **byte-identical**
  and the serde round-trip test (`render/mod.rs:1267`) still passes. Server
  construction is the projector (F1); it only ever builds the seed with an
  already-parsed `Tag`. The `render_head`/`render_body`/ `render_discovery`
  matches read `tag` via `Display` (`format!("#{tag}")`) — unchanged; the
  `tag.parse::<Tag>()` at `render/mod.rs:201,209` collapses to using the
  already-typed `tag` (clone). Client consumers `SiteTagPage` (`posts.rs:1087`)
  / `UserTagPage` (`posts.rs:1249`) read the typed seed and, on the CSR
  route-param path, parse `params.get("tag").parse::<Tag>()` (invalid → skip the
  fetch, client 404), mirroring the #408 `PostPage` route-parse.
  `render_tag_list` reads `TagSummary`'s typed fields, `.to_string()`-ing at
  `view!`/href sites (a newtype is not `IntoRender`); `/tags/{slug}` hrefs
  unchanged.
- **F3** `web::feed_events::tag_slugs(&[PostTag]) -> BTreeSet<Tag>` (was
  `BTreeSet<String>`), built from `tag_slug.clone()`. **Verify:** feed-enqueue
  behavior identical.

### §G `web` `TagInput` (#416)

- **G1** `is_valid_tag_slug` and `normalize_tag_token` deleted; the commit path
  validates the **raw trimmed** input via `TagLabel::from_str` and stores
  `TagSummary { slug: label.slug(), display: label }` — **casing preserved**
  (Decision 4). The hidden-input `value` is the `TagLabel` (`Display`).
  **Verify:** (a) a client/server-agreement test pins a previously-divergent
  case (uppercase / leading-space); (b) a test pins the new casing behavior —
  creating a post with "Rust" via the web path yields `display: "Rust"`,
  `slug: "rust"`, and the chip renders "#Rust"; (c) the e2e "invalid tag" error
  assertion matches `InvalidTagLabel`'s message.

### §H sweep

- **H1** After all of the above, `rg` finds **no** bare `String`/`&str` tag
  value across the five crates except the three justified sites in Decision 7
  (each carrying its comment). **Verify:** a documented `rg` sweep in the ship
  review; `cargo xtask validate --no-e2e` clean.

### §I ADR

- **I1** `docs/adr/drafts/tag-identity-label-split.md` is promoted at ship
  (`cargo xtask adr promote`), numbering it and syncing the README table.

## Risks

- **R1 — trailer relocation to `tag.rs` (CRAP).** Two `StrNewtype` expansions
  now attribute their generated surface to `tag.rs`; the macro trailer is tested
  in `macros/tests/str_newtype.rs` and is CRAP-safe (mirrors #408 Risk 1). No
  new trailer tests; `TagLabel`'s hand `FromStr`/`slug` are unit-tested (A2).
- **R2 — read-path re-validation of `tag_display`.** Typing
  `PostTagJson.tag_display` means serde now validates the stored label as a
  `TagLabel` on read (where today it is passed through unvalidated). All stored
  labels were written through `tag_post`'s `Tag::from_str` check (which rejects
  spaces/invalid chars), so each re-validates; a hypothetical malformed legacy
  row would now fail the read. Accepted (no writer bypasses the check); the
  dual-backend read test (B4) is the guard.
- **R3 — leptosfmt mangles generic component tags** (`#420`): `TagInput` uses no
  `<ValidatedInput<T>>`, so this vertical is largely clear, but any generic tag
  added must mirror the `auth.rs` workaround.
- **R4 — `TagValidationError::Invalid` removal.** Only safe if no path
  constructs it post-retype; if any test or caller still does, keep the variant.
  Confirm by `rg 'TagValidationError::Invalid'` before deleting.
- **R5 — atompub silent-skip relocation (no-behavior-change hazard).** Today an
  invalid `<category term>` is dropped downstream (`post_tag_diff`'s
  `filter_map`, `apply_categories`' "skipped" doc). B2 removes that skip, so it
  **must** move into `entry_to_post_fields`'s
  `filter_map(...parse::<TagLabel>().ok())` (§D1). If missed, a single malformed
  category term would fail ingest of the whole entry — a behavior change. Guard:
  the §D1 skip test.

## Out of scope

- Surfacing per-post author casing in the **`list_tags` autocomplete** (the "M5
  display-casing wiring" noted at `server/tests/web/web_tags.rs:124`):
  `list_tags` reads the canonical `tags` catalog, which has no author casing, so
  its `TagSummary.display` continues to mirror the slug. (Preserving casing in
  the `TagInput` — Decision 4 — is now **in** scope; surfacing catalog casing is
  the separate, still-out item.)
- Typing the `list_tags` search `prefix` and the feed router path segment
  (Decision 7 — the two justified `String` sites).
- Any wire-format, schema, or migration change; the `tags`/`post_tags` columns
  and all serialized shapes are untouched.
- The other #404 verticals (#407/#410) and `#417` request-aggregate DTOs.
