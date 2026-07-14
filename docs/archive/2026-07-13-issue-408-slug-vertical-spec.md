# Spec — Issue #408: `Slug` vertical (StrNewtype + thread `Slug` everywhere)

- Issue: [#408](https://github.com/jaunder-org/jaunder/issues/408) — part of the
  #404 umbrella, milestone "Domain-value type safety (newtypes)".
- Blocked by: #403 (StrNewtype/IdNewtype derives — **CLOSED**) and #414
  (client-side validation pattern, ADR-0065 — **CLOSED**). Both resolved;
  unblocked.
- Governing ADRs: **ADR-0063** (newtype convention: generated trailer, boundary
  rule §4), **ADR-0065** (typed `#[server]` wire args + client-side
  pre-validation), and **ADR-0023** (the read-only `j:slug` atompub extension).
- Direct precedent: the just-merged **#407** `Username` vertical (PR #427) —
  this spec copies its shape, adapted to `Slug`'s already-more-advanced state.

## Goal

Complete the end-to-end `Slug` vertical: adopt `#[derive(StrNewtype)]` on
`Slug`, and replace every remaining bare `String`/`&str` that carries a
**resolved slug value** with `Slug` (or `&Slug`, or `&str`-via-`Deref` where a
read-only borrow is idiomatic), storage-outward, per ADR-0063 §4 (parse at the
outermost boundary, hold the newtype inward). The governing acceptance test
(user-stated, and reviewed at ship): **at the end, no slug value anywhere is a
`String`/`&str`** — the only surviving raw strings are the enumerated
slug-_generation_ candidates that funnel through the `Slug::from_str` chokepoint
(§I). No wire/behavior change.

## Current state (from the site survey)

Unlike #407, the `Slug` vertical is **already half-done**: `Slug` is already
threaded through `storage` (`PostRecord.slug`, `CreatePostInput.slug`,
`get_post_by_permalink`, the draft/permalink helpers) and the `server`-internal
call paths. The `macros` crate is already a `common` dependency (added for
#407), so no new dependency wiring. The remaining surface is:

- **`common/src/slug.rs`** — still hand-writes the whole trailer + serde bridge;
  must adopt the derive.
- **`web`** — 6 DTO/view `slug: String` fields, the `get_post` `#[server]` slug
  arg, the `create_post`/`update_post` `slug_override: Option<String>` args, and
  the raw `slug_override` `<input>` on the create/edit forms + the `ui.rs`
  composer.
- **`server`** — one atompub emit site (`mapping.rs:164`) still calls the
  soon-deleted inherent `.as_str()`.
- **`storage`** — the `slug_override: Option<&str>` params on the two
  post-creation metadata structs, and the `.as_str()` sweep sites (SQL binds +
  test asserts).

### What `#[derive(StrNewtype)]` generates (so we know what to delete)

`Display`, `AsRef<str>`, `Borrow<str>`, `Deref<Target = str>`, `TryFrom<String>`
(via `FromStr`), `From<Self> for String`, `PartialEq<str>`, `PartialEq<&str>`,
and direct `Serialize`/`Deserialize` impls (serialize borrows; deserialize
routes through `FromStr`). It does **not** generate `FromStr`, the std
`#[derive]`s, or any inherent `as_str()`.

### The #414 reference pattern (already shipped)

`web/src/forms.rs` provides the shared chokepoint:
`field_error<T>(&str) -> Option<String>` (both-target, host-tested), `Field<T>`
(parent-owned live value + validity, signal-only), and
`#[component] ValidatedInput<T>`. `pages/auth.rs` (register/login) and
`pages/password_reset.rs` are the exemplars: `Field::<T>::new()`,
`<ValidatedInput<T> name=… />`, submit gated `disable-until-valid`, typed wire
arg. **Gap:** these primitives model **required** fields only — `Field::new()`
seeds its error from `field_error::<T>("")`, so an empty field is deliberately
invalid. `slug_override` is **optional** (empty ⇒ auto-generate ⇒ valid), the
first such field; see Decision 3.

## Approved decisions (design interview)

1. **`get_post`'s slug arg becomes `Slug`; the projector permalink handler is
   unchanged (already `Slug`-typed via inner parse) but gains a justification
   comment.** `get_post(…, slug: Slug)` types the wire arg and drops the in-body
   `slug.parse::<Slug>()?` (mirrors #407's `get_post(username)` move). The slug
   is URL-derived, so — exactly as #407 did for `username` — the `PostPage`
   component parses the route segment into `Slug` client-side and skips the
   fetch when it fails to parse (no server round-trip for a malformed URL). The
   **public projector** permalink handler (`server/src/projector/mod.rs`
   `permalink`, `:136-142`) **already** extracts `Path<(String, …, String)>` and
   parses both `username` and `slug` inside the handler (`:138`, `slug.parse()`
   → `Slug`) with shell-fallback — identical to how #407 treats `username`
   there. It stays `Path<String>` (**no functional change**): a typed extractor
   would reject a malformed public URL with an axum 400 _before_ the handler
   runs, defeating the shell-fallback that serves the SPA (a
   client-rendered 404) for bare permalink URLs. This is the deliberate
   **projector-vs-atompub boundary split** (#407: atompub handlers _are_
   `Path<Username>` — a 400-on-malformed API; the projector is not). Because the
   review bar is "no slug as `String`", **add an adjacent comment** at the
   `Path<(String, …, String)>` extractor documenting why the raw segment is kept
   (shell-fallback; typing it would 400 a public URL) so the retained `String`
   reads as intentional, not an unfinished site. Slug has **no** inbound atompub
   path segment (member URLs key on `(Username, post_id)`; `j:slug` is
   emit-only, ADR-0023), so the atompub `Path<Username>` pattern has no slug
   analogue. A malicious direct `get_post` call with an invalid slug hits the
   generic decode error (ADR-0065 defense-in-depth, acceptable — not a
   normal-user path).

2. **`slug_override` becomes `Option<Slug>` on the wire; validation moves to the
   boundary.** `create_post`/`update_post` take `slug_override: Option<Slug>`.
   The two storage metadata structs — `PostCreation.slug_override` and
   `PostUpdate.slug_override: Option<&'a str>` — become `Option<&'a Slug>`, and
   **both** override re-parses drop: `perform_post_creation`'s
   `raw.parse::<Slug>()?` (`post_service.rs:470-474`) **and**
   `perform_post_update`'s identical
   `slug_override.and_then(non_empty).…parse::<Slug>()` block
   (`post_service.rs:312-319`). A `Some` branch now holds an already-valid
   `Slug` (and `non_empty`, which takes `&str`, is no longer applied to the
   override). Consequence: the storage-layer test
   `test_perform_post_creation_invalid_slug_override` (feeding
   `Some("Invalid Slug!")`) becomes **unconstructible** — an invalid `&Slug`
   cannot exist — so that rejection is **relocated** to the boundary: the client
   `field_error` (§H) and the serde-bridge deserialize test. The dedup path is
   unchanged: a valid override is still fed through the collision-suffix
   generator (§I).

3. **Extend the #414 helper with optional-field support (within #408); amend
   ADR-0065.** Add an optional mode so a pristine empty field counts as valid: a
   `field_error` optional path (empty ⇒ `None`), a `Field` optional constructor
   (seeds `error = None`), and a `ValidatedInput` `optional` flag — unit-tested
   alongside the existing required tests. `slug_override` is the first optional
   domain-value field and the sole in-tree adopter this issue; the #409/#410
   verticals reuse it. Amend **ADR-0065** with a one-paragraph note on the
   optional variant (via the `jaunder-adr` flow — edit the existing 0065).

4. **`Slug` omits `Hash`.** Derive list stays
   `Clone, Debug, PartialEq, Eq, StrNewtype` — **no `Hash`** (issue-explicit;
   ADR-0063 §… documents "Slug omits `Hash`"). Verified: no
   `HashMap`/`HashSet`/`BTreeMap`/`BTreeSet` keys on `Slug` anywhere. (This is
   the one intentional deviation from the `Username` template, which carries
   `Hash`.)

5. **The slug-generation zone stays `String`, funneling through `from_str`.**
   `Slug::from_str` remains the single construction chokepoint (its doc comment:
   "the single chokepoint both slug generation and inbound URL resolution funnel
   through"). `slugify_title(title: &str) -> String` (input is a _title_, not a
   slug) and `candidate_slug(base: &Slug, attempt) -> String` (input typed
   `&Slug`; output an unvalidated collision-_candidate_) produce candidate
   strings that are validated at the call site (`…​.parse::<Slug>()?`). These
   two `-> String` returns are the **complete enumerated residual** (§I) — pre-
   chokepoint candidates, not stored/transported slug values — and are called
   out so the ship review confirms them by design rather than flagging them.

## Scope — sites to convert (grouped by layer)

### A. Adopt the derive (`common/src/slug.rs`)

- `#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]` (**no `Hash`**);
  `use macros::StrNewtype;`. Keep `FromStr` and `InvalidSlug` verbatim, and the
  free fns `slugify_title` / `base_is_alphanumeric` / `MAX_SLUG_CHARS`.
- Delete the hand-written `TryFrom<String>` (L62-68), `From<Slug> for String`
  (L70-74), inherent `as_str()` (L76-81), `Display` (L83-87), and the
  `#[derive(Serialize, Deserialize)]` + `#[serde(try_from, into)]` bridge
  (L23-24). Drop now-unused `serde`/`fmt` imports.
- Rewrite the in-file test `.as_str()` asserts (the inherent method is deleted)
  to `== "lit"` (`PartialEq<str>`) / `.as_ref()`. **No new trailer tests** — the
  generated surface is exhaustively tested in `macros/tests/str_newtype.rs`, and
  every generated impl is cyclomatic-complexity-1 → below the CRAP gate even
  uncovered (Risk 1). Keep the `FromStr`/`slugify_title`/serde-bridge tests
  (updating the idempotence asserts to the new surface).

### B. web — `#[server]` wire args (typed; drop the internal parse)

- `posts::get_post(…, slug: Slug)` — drop the `slug.parse::<Slug>()?`; pass
  `&slug` to `fetch_post_record` / `find_draft_by_permalink_for_user` (Decision
  1).
- **`PostPage` read path** (`pages/posts.rs`, ~L164-170): the `slug: String`
  pulled from `params.get("slug")` and passed into `get_post` becomes a
  client-side `params.get("slug")?.parse::<Slug>()` — skip the fetch
  (client 404) when it fails to parse, so the typed wire arg only ever receives
  a valid `Slug` from a legitimate client (Decision 1 / Risk 3). This is the
  read-path counterpart to the `slug_override` write-path forms in §D.
- `posts::create_post(…, slug_override: Option<Slug>)` and
  `posts::update_post(…, slug_override: Option<Slug>)` — pass
  `slug_override.as_ref()` (→ `Option<&Slug>`) into the storage metadata struct
  (Decision 2). Drop the `common::text::non_empty` juggling where the field now
  arrives pre-typed.

### C. web — DTO / view fields (→ `Slug`)

- `posts/listing.rs::TimelinePostSummary.slug`
- `posts/mod.rs::{CreatePostResult, UpdatePostResult, DraftSummary, PublishPostResult, PostResponse}.slug`
- Construction sites feeding them (`record.slug.to_string()` etc.) become a move
  or `.clone()` of the `Slug` now that the field is `Slug`.

### D. web — forms: client-side pre-validation (ADR-0065) on `slug_override`

- **`CreatePostPage`** and **`EditPostPage`** (`pages/posts.rs`) and the
  **`ui.rs` composer** — replace the raw `slug_override` `RwSignal<String>` +
  hand-rolled `<input>` with a `Field::<Slug>` (optional; `EditPostPage` seeds
  it from the fetched draft's `Slug` via the optional prefilled constructor) and
  `<ValidatedInput<Slug> name="slug_override" optional=true />`. Dispatch reads
  `field.parsed()` → `Option<Slug>` for the typed wire arg. Submit is **not**
  gated on this field (it is optional — empty is valid); an invalid non-empty
  entry shows the inline error and gates submit.
- Keep the existing e2e selector / `name="slug_override"` stable.

### E. storage — post-creation metadata (`post_service.rs`)

- Both metadata structs' `slug_override: Option<&'a str>` → `Option<&'a Slug>`
  (`PostCreation` and `PostUpdate`).
- `perform_post_creation` (:470-474) **and** `perform_post_update` (:312-319):
  drop the `…parse::<Slug>()` override re-parse; a `Some` branch clones the
  already-valid `&Slug` as the seed (Decision 2).
  `seed_post_input(slug: Slug, …)` is already typed — unchanged.
- **Seed-type unification (§I / Decision 5):** the `None` branch's
  `slugify_title(...) -> String` output must be parsed to a `Slug` (`.parse()?`,
  infallible-by-construction but kept honest) so both branches yield a `Slug`
  seed feeding `candidate_slug`'s now-`&Slug` parameter.
- Delete `test_perform_post_creation_invalid_slug_override` (unconstructible);
  ensure the invalid-override rejection is covered at the boundary — the client
  `field_error::<Slug>` test (§H) and a `serde_json::from_str::<Slug>` reject
  test (already in `slug.rs`).
- Update the `slug_override: Some("my-custom-slug")` valid-override test to pass
  a `&Slug` (`"my-custom-slug".parse().unwrap()`).

### F. server — atompub emit + projector comment

- `server/src/atompub/mapping.rs:164`
  `set_j_slug(&mut entry, post.slug.as_str())` → `post.slug.as_ref()` (or
  `&post.slug`). The title-fallback `post.slug.to_string()` (`:141`) uses
  generated `Display` — unchanged. The `j:slug` element stays emit-only (inbound
  `j:slug` ignored, ADR-0023) — no behavior change. Test fixtures building
  `PostRecord { slug: … }` from `&str` literals continue to work via the derived
  `TryFrom`/`FromStr` (`.parse()`).
- `server/src/projector/mod.rs:136` `permalink` — **no functional change**
  (already parses the slug segment to `Slug` at `:138` with shell-fallback);
  **add an adjacent justification comment** at the `Path<(String, …, String)>`
  extractor explaining the retained `String` (Decision 1): the raw segment is
  parsed inside the handler so a malformed public URL serves the SPA shell
  rather than a typed-extractor 400 — the projector-vs-atompub boundary split.

### G. host

- No slug **value** travels through `host`; only the boundary error surface
  (`host/src/error.rs` registers `common::slug::InvalidSlug` in the `check!`
  macro) — unchanged. No signature changes.

### H. forms helper — optional-field support (`web/src/forms.rs`)

- Add the optional path (Decision 3): empty ⇒ valid. Unit-test it (empty
  optional field is valid + submittable; non-empty invalid shows the newtype's
  own message; non-empty valid parses) alongside the existing required tests.
  `field_error::<Slug>` for a non-empty invalid slug already returns
  `InvalidSlug`'s `Display` — assert it (this is the relocated §E rejection).

### I. `.as_str()` / generation-zone sweep (compiler-forced)

Deleting the inherent `as_str()` makes every `.as_str()` on a `Slug` a compile
error — the sweep is mandatory, not optional:

- **SQL binds** (`storage/src/posts.rs:83, 981, 1869`) → `.as_ref()` / `&*slug`.
- **atompub** (`server/src/atompub/mapping.rs:164`) → `.as_ref()` (§F).
- **Test asserts** (`storage/src/post_service.rs` ~11 sites:
  `record.slug.as_str()` → `== "lit"` / `.as_ref()`; `common/src/slug.rs`
  in-file asserts → §A).
- **Generation zone** (Decision 5 / §E): `slugify_title` stays `-> String`;
  `candidate_slug` param typed `base: &Slug`, stays `-> String`; both funnel
  through `from_str` at the call site. These are the enumerated residual
  `String` slug-candidates — no other `-> String`/`&str` slug value remains.

## Acceptance

- `Slug` derives `StrNewtype`; no hand-written trailer or `#[serde]` bridge
  remains; `FromStr`/`slugify_title`/`InvalidSlug` unchanged; derive omits
  `Hash`.
- **No bare `String`/`&str` remains for a resolved slug value** across storage
  records/signatures, host, server, web DTOs, internal web, `#[server]` wire
  args, or component props (per the boundary policy). The only residual raw
  strings are (a) the two enumerated slug-generation candidates (§I / Decision
  5), each funneling through `from_str`, and (b) the projector permalink's
  inbound `Path<…, String>` slug segment (Decision 1 / §F) — raw HTTP-boundary
  input parsed to `Slug` in-handler with shell-fallback, carrying an **adjacent
  comment** justifying the retained `String`. Each residual is enumerated and
  commented so the ship review confirms it by design.
- `slug_override` is a typed `Option<Slug>` wire arg with
  `<ValidatedInput<Slug> optional=true>` client pre-validation (inline errors,
  no round-trip for the invalid-override case), matching #414/#407.
- The forms helper supports optional fields, unit-tested; ADR-0065 amended.
- No wire/behavior change: `Option<Slug>` serializes identically to
  `Option<String>`; the projector shell-fallback UX and atompub `j:slug` output
  are unchanged.
- `cargo xtask validate --no-e2e` clean; e2e green for the affected
  create/edit-post and permalink/atompub flows.

## Risks / notes

1. **Coverage of the generated trailer — a non-risk, kept as a note.** `Slug`'s
   generated impls attribute to `slug.rs`, but the trailer's behavior is tested
   in `macros/tests/str_newtype.rs`, and every generated impl is CC-1 → CRAP ≈ 2
   even fully uncovered (ADR-0050, T=30). No adopter-site trailer tests needed.
2. **Validation relocation (Decision 2).** Removing the storage re-parse moves
   the invalid-override rejection to the boundary. Confirm the client
   `field_error` + serde-bridge tests fully cover it before deleting the storage
   test, so no coverage is lost.
3. **Route-driven typed `get_post` arg.** `PostPage` moves parse-on-entry from
   the server body to a client parse of the route slug; confirm the
   malformed-route UX (skip fetch / client 404) matches today's. Internal web
   behavior, not a wire change. The projector shell-fallback is the backstop for
   bare-URL hits.
4. **leptosfmt on generic tags (#420).** `<ValidatedInput<Slug> …>` invocations
   get mangled by leptosfmt; follow the existing `auth.rs`/`password_reset.rs`
   workaround.
5. **Optional-field semantics.** An empty optional field must be _valid and
   submittable_ yet still show a non-empty invalid entry's error — verify the
   `touched`/`disable-until-valid` interaction doesn't wrongly gate submit on an
   empty optional slug.

## Out of scope / follow-ups

- No separable concern warrants a spun-off issue: the vertical is one cohesive
  value-class change (ADR-0063 "each value class is its own reviewable change"),
  finished in one pass — including the full `.as_str()` sweep (§I). The
  optional-field forms-helper support (Decision 3) is done **within** this issue
  because #408 is the first adopter that needs it and it is small; the ADR-0065
  amendment is the only cross-cutting artifact.
- `tag_slug: &Tag` lookups and `TagSummary.slug` are the **`Tag`** newtype
  (`common::tag::Tag`), a different value class (#409-ish) — out of scope.
- Absorbs the `Slug` half of #14 (closed).
