# Spec — issue #545: `PostSummary` + `Bio` newtypes (full vertical)

- Issue: [#545](https://github.com/jaunder-org/jaunder/issues/545)
- Milestone: Domain-value type safety (newtypes)
- Date: 2026-07-20
- ADRs: [0063](../../adr/0063-domain-value-newtype-convention.md) (convention),
  [0065](../../adr/0065-client-side-domain-validation.md) (client validation),
  [0071](../../adr/0071-sqlx-string-newtype-bridge.md) (sqlx bridge)

## Goal

Replace the last two bare-`String` domain values on the post/profile surfaces
with validated string newtypes and thread them through **the whole vertical** —
`common` → `storage` → `common::feed` → `web` — so the value is parsed once at
the outermost boundary and held typed everywhere inward, with **no
`parse().expect()` at any seam** (ADR-0063 §4).

- **`PostSummary`** — a post's summary/excerpt.
- **`Bio`** — a user's profile biography.

This is the same shape already shipped for `DisplayName` (profile), `PostTitle`/
`PostBody` (posts), and `FeedItem.title`/`content_html` (#470, feed) —
`summary`/`bio` are the last flat `String`s sitting next to those already-typed
siblings.

## Resolved decisions

| #   | Decision                                 | Choice                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| --- | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Threading depth                          | **Full vertical** (common + storage + feed + web). Web-only was rejected: a validating cap at an infallible seam would force a `parse().expect()`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| D2  | `PostSummary` invariant                  | Trim outer whitespace (preserve inner incl. newlines); **non-empty after trim**; **≤ 500 Unicode scalars**. Validating `FromStr`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| D3  | `Bio` invariant                          | Trim outer whitespace (preserve inner incl. newlines); **non-empty after trim**; **≤ 1000 Unicode scalars**. Validating `FromStr`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| D4  | `summary_label` (derived fallback label) | **Type it as `PostSummary`** (not a display `String`). Requires an infallible truncating door (D6).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| D5  | `update_profile` bio wire arg            | Change `bio: String` → `bio: Option<Bio>`, mirroring the adjacent `display_name: Option<DisplayName>`. `None` clears (via omission); a non-empty invalid value is rejected client-side. Removes the `common::text::non_empty` blank-clears shim.                                                                                                                                                                                                                                                                                                                                                                                                           |
| D6  | Derived-label construction door          | `PostSummary::truncated(&str) -> PostSummary` — trim + truncate to the 500-char cap at a char boundary. Infallible **length-validated trusted door** (guarantees `≤ 500`; analogous to `NumNewtype::clamped`, but not `const fn`). It does **not** itself enforce non-emptiness — that half of the invariant is enforced only by the public `FromStr`/serde door; `truncated` is the internal trusted door (the `RenderedHtml::from_trusted` model), fed only non-empty derived labels (body line / title / slug). A `debug_assert!` guards the non-empty precondition (loud in test/debug); public but its only callers are the two label producers (D4). |

**Empty vs. reject (issue's open question).** For **both** types the `FromStr`
rejects an empty/whitespace-only string. Presence is modeled by `Option<T>` at
the boundary, so "no summary"/"no bio" is `None`, never `Some(empty)`. Rejecting
empty in `FromStr` also means a literal empty string on the wire is
**rejected**, which forces clearing to go through **omission** (`None`),
matching the established `Option<Newtype>` clear pattern (the client's
`Field::optional()` maps empty input to `None`, never to a parse of `""`).

## Newtype definitions (`common`)

Two new modules, each following the `DisplayName` template
(`common/src/display_name.rs`):
`#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]` (no `Hash` — neither is a
map/set key), a hand-written validating `FromStr`, a `thiserror` error type, and
a public `MAX_*_CHARS` const. The `StrNewtype` derive supplies the full trailer
(`Display`, `AsRef`/`Borrow`/`Deref<str>`, owned-`String` conversions,
`PartialEq<str>`/`<&str>`, validating serde bridge, and the ADR-0071 sqlx
bridge).

- `common/src/post_summary.rs` — `pub struct PostSummary(String)`;
  `pub const MAX_POST_SUMMARY_CHARS: usize = 500`; validating `FromStr`; plus
  the `truncated` door (D6).
- `common/src/bio.rs` — `pub struct Bio(String)`;
  `pub const MAX_BIO_CHARS: usize = 1000`; validating `FromStr`.
- `common/src/lib.rs` — register `pub mod post_summary;` and `pub mod bio;`.
- `common/src/test_support.rs` — add `parse_post_summary(&str) -> PostSummary`
  and `parse_bio(&str) -> Bio` (the single test-construction door per the repo
  convention), importing the two types.

## Threading map — exact surfaces

### storage

- `PostRecord.summary` (`storage/src/posts.rs:71`) → `Option<PostSummary>`;
  likewise the sibling storage structs `CreatePostInput`/`UpdatePostInput` at
  `posts.rs:206,233`.
- `PostRecord::fallback_summary_label` (`storage/src/posts.rs:93`) → return
  `PostSummary` (built via `PostSummary::truncated`). This is the **production**
  summary- label producer (feeds `DraftSummary.summary_label`).
- The service-layer arg structs (`storage/src/post_service.rs:42,143,279,443`)
  summary fields → `Option<PostSummary>`.
- SQLite/Postgres binds (`storage/src/posts.rs:1902`, plus the `SELECT` column
  decodes) bind/decode `PostSummary` directly via the ADR-0071 bridge — no
  `.as_deref()` strip (the `sqlx-newtype-bind` gate enforces this).
- `User.bio` (`storage/src/users.rs:32`) → `Option<Bio>`; `ProfileUpdate.bio`
  (`storage/src/users.rs:118`) → `Option<&Bio>` (mirror
  `display_name: Option<&DisplayName>`); its bind/decode use the bridge.

### common::render

- `DerivedPostMetadata.summary_label` (`common/src/render.rs:140`) →
  `PostSummary` (built via `PostSummary::truncated` at each
  `derive_post_metadata` assembly branch). This field is **production-dead**
  (only `slug_seed`/`title` are consumed downstream; the field is read only in
  `render.rs` tests) — but it is a `summary_label` value, so D4 + ADR-0063 §5
  type it as `PostSummary` rather than leave it a bare `String` with a recorded
  carve-out. `slug_seed` stays `String` (it is a slug seed, not a summary). The
  internal `fallback_label` helper (`render.rs:188`) keeps returning
  `Option<String>` (an internal building block); the `PostSummary` is minted at
  the struct assembly. Consequence: `derive_post_metadata` becomes the second
  `PostSummary::truncated` caller.

### common::feed

- `FeedItem.summary` (`common/src/feed/metadata.rs:24`) → `Option<PostSummary>`.
- `server/src/feed/regenerate.rs:151` — `summary: p.summary.clone()` moves the
  typed value directly (no conversion), matching the `title`/`content_html`
  lines beside it.
- Feed renderers (`common/src/feed/atom.rs`, `rss.rs`, `json.rs`) read the value
  out via `Deref`/`Display`/`Serialize` at the external-crate (atom_syndication
  / rss / serde_json) boundary — the ADR-0063 §5 external-type carve-out,
  exactly as `title` already does.

### web

- DTO fields → typed:
  - `CreatePostResult.summary` (`web/src/posts/mod.rs:64`),
    `UpdatePostResult.summary` (`:75`), `PostResponse.summary` (`:214`) →
    `Option<PostSummary>`.
  - `DraftSummary.summary_label` (`web/src/posts/mod.rs:162`) → `PostSummary`.
  - `TimelinePostSummary.summary` (`web/src/posts/listing.rs:40`) →
    `Option<PostSummary>`.
  - `ProfileData.bio` (`web/src/profile/mod.rs:27`) → `Option<Bio>`.
- `#[server]` args → typed:
  - `create_post` summary (`web/src/posts/mod.rs:254`), `update_post` summary
    (`:409`) → `Option<PostSummary>` (both use `input = Json`; `None`
    omitted/`null` → clears). The two
    `summary.and_then(common::text::non_empty_owned)` normalization sites in
    `web/src/posts/mod.rs` become dead (`non_empty_owned` takes `String`, and
    `PostSummary`'s `FromStr` already trims + rejects empty) and are **removed**
    at those two call sites only.
  - `update_profile` (`web/src/profile/mod.rs:58`) → `bio: Option<Bio>` (D5);
    body passes `bio.as_ref()` into `ProfileUpdate`, dropping the
    `common::text::non_empty(&bio)` call **at this site**. (The
    `common::text::non_empty`/`non_empty_owned` helpers stay — they have
    unrelated callers in site_config/auth/backup/site; only the summary/bio call
    sites drop them.)
- Seam builders (`web/src/posts/server.rs` `post_response`,
  `timeline_post_summary`) move the typed `summary` directly (they already do;
  only the field types change).

### Client validation (ADR-0065)

Both target forms today use a raw `RwSignal<String>` + `<textarea>`; convert
each to a parent-owned `Field<T>` (the direct-bind variant, since both submit
programmatically / via a bespoke layout):

- **Profile bio** (`web/src/pages/profile.rs`) — `Field::<Bio>::optional()` /
  `optional_prefilled(existing_bio)`, bound to the `name="bio"` textarea,
  submitting `bio: field.parsed()` (`Option<Bio>`). Mirror the
  `Field::<DisplayName>::optional()` control already in this file
  (`profile.rs:18`).
- **Post summary** (`web/src/pages/posts.rs`) —
  `Field::<PostSummary>::optional()` / `optional_prefilled`, bound to the
  `#edit-summary` textarea, submitting `summary: field.parsed()`
  (`Option<PostSummary>`), replacing `common::text::non_empty_owned`.
- Submit stays enabled for an empty (valid) field and is disabled for a
  non-empty invalid one; the touched-gated inline error is the newtype
  `FromStr::Err` `Display`.
- `field_error::<PostSummary>` / `field_error::<Bio>` are host-compiled and
  coverage-measured (ADR-0065 coverage boundary).

## xtask / ADR

- **No new xtask gate.** `PostSummary`/`Bio` are not secrets →
  `proffered-secret` untouched; `server-fn-registrar` is type-agnostic;
  `sqlx-newtype-bind` already covers the storage binds (bridge default-on).
- **ADR-0063 addendum** documenting the string truncating validated door
  (`truncated`, D6) as the string analog of the numeric `clamped` affordance —
  authored as a numberless draft via `jaunder-adr`, promoted at ship. (Confirm
  during planning whether a one-paragraph addendum suffices vs. a standalone
  ADR.)

## Acceptance criteria (observable)

1. `PostSummary` / `Bio` exist in `common` with the full ADR-0063 string
   trailer; a value > cap or empty/whitespace-only **fails** `parse()` and fails
   serde deserialization; a value at the cap and a trimmed value **succeed**;
   `Display`/serde round-trip to the plain (trimmed) string. (unit tests,
   mirroring `display_name`.)
2. `PostSummary::truncated` for any non-empty input returns a value ≤ 500 chars,
   trims outer whitespace, and truncates over-length input at a char boundary
   (no panic on multi-byte inputs). Its documented non-empty precondition is
   pinned by a `debug_assert!` that fires in test/debug on empty/whitespace-only
   input. (unit tests.)
3. No production source contains `summary`, `summary_label`, or `bio` typed as
   `String`/`Option<String>` on any of the enumerated storage/render/feed/web
   surfaces; each is `PostSummary`/`Bio`. (grep for `summary:`,
   `summary_label:`, `bio:` + compile.)
4. No `.parse().expect()` / `.parse().unwrap()` on a summary/bio value appears
   at any web or feed seam. (grep + review.)
5. `update_profile(bio: None)` **clears** a previously-set bio; `create_post`/
   `update_post` with `summary: None` store no summary. Round-trips through both
   storage backends preserve a set value. (storage backend-parity tests + e2e.)
6. A stored `summary`/`bio` value exceeding the cap fails to decode with an
   internal error (fail-closed), mirroring
   `authenticate_with_overlong_display_name_in_db_returns_internal_error`
   (`storage/src/users.rs:637`). (storage tests, both backends.)
7. **e2e**: in the browser, (a) setting then **clearing** a bio persists the
   cleared state after reload; (b) setting then clearing a post summary; (c) an
   over-cap summary/bio entry disables the form's submit and shows the inline
   validation error.
8. Feeds (atom/rss/json) still emit the post summary unchanged for a set value
   and omit it when absent. (existing feed tests pass with typed field.)
9. `cargo xtask validate` is green (static + clippy + coverage + full e2e
   matrix).

## Testing requirements

- Unit tests for both newtypes mirror `display_name.rs`
  (parse/trim/reject-empty/ reject-over-cap/at-cap/serde), plus `truncated` for
  `PostSummary`.
- Storage tests use `parse_post_summary`/`parse_bio` (never inline
  `.parse().unwrap()`), are `#[case]`-parametrized over both backends (ADR-0053
  `TestEnv` homing), and add the overlong-in-DB fail-closed test for each value
  (raw-SQL seed of an over-cap row).
- Web host tests cover `field_error::<PostSummary>`/`::<Bio>` and the seam
  builders.
- e2e covers the clear paths and the disable-until-valid UX (criterion 7); the
  clear path must be exercised in the browser, not merely by switching an
  integration body to omit the field.
- Backend parity, coverage policy, and the verify ladder per `CONTRIBUTING.md`.

## Risks / consequences

- **Backward-compat (fail-closed reads).** Because the validating serde/sqlx
  `Decode` routes through `FromStr`, any **pre-existing** `summary` > 500 or
  `bio` > 1000 in a database would now fail to read (surfacing as an internal
  error), where it previously read fine as unbounded `String`. This is the same
  trade `DisplayName` already accepts repo-wide (criterion 6 / `users.rs:637`).
  Given the project is pre-release with no real-data migration in scope, we
  accept fail-closed rather than a lenient/truncating `Decode`. **If real data
  with long summaries/bios exists, a backfill is required and is out of scope
  for this issue** — flag at approval.
- **Scope is a full vertical, wider than the issue title's "web boundary."**
  This is deliberate (D1) and matches the DisplayName / #470 precedents; it is
  one coherent goal, not split.
- **`truncated` is a caller-trusted door.** It guarantees the length cap but not
  non-emptiness — that half of the invariant is enforced only by the public
  `FromStr`/serde door (the `RenderedHtml::from_trusted` model). Both its
  callers (`storage::PostRecord::fallback_summary_label` and
  `common::render::derive_post_metadata`) always feed a non-empty label (first
  non-empty body line → non-empty title → non-empty slug), so the produced value
  is non-empty in practice; the `debug_assert!` makes any future violation loud.
  It is the only construction door that does not itself reject empty.

## Out of scope

- Typing timestamps (#91) or any other boundary value.
- A data backfill/migration for over-cap legacy rows (see Risks).
- A boilerplate-reducing `Field`/`ValidatedInput` macro (ADR-0065 future work).
