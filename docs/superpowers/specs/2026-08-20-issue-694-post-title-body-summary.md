# #694 — typed Post title/body/summary seams

Issue: [#694](https://github.com/jaunder-org/jaunder/issues/694). Milestone:
Domain-value type safety (newtypes).

## Summary

Jaunder already has domain newtypes for a Post's authored content: `PostTitle`,
`PostBody`, and `PostSummary`. `PostBody` and `PostSummary` are threaded through
most high-level paths, but several input, view, and test helper seams still
accept `&str` for values whose type exists. This cycle removes those primitive
seams without changing Post behavior or serialized representations.

The target state is simple: once a title/body/summary value has crossed a
boundary, interior structs and helpers carry the domain type. String parsing
remains at real boundaries and in explicit test-literal helpers.

## Decisions

### D1 — Service inputs carry typed title/body/summary values

`storage::PostCreation` and `storage::PostUpdate` keep `body: PostBody` and
`summary: Option<PostSummary>`, and change their explicit title field from
`Option<&str>` to `Option<&PostTitle>`.

`common::render::derive_post_naming` should take `Option<&PostTitle>` for an
explicit title. Boundary code must preserve the existing behavior that blank
submitted titles mean absence (`None`), then pass only a successfully parsed
`PostTitle` into storage. Derived titles from Markdown/Org remain parsed inside
`derive_post_naming`, because those strings are extracted from the body rather
than supplied through the explicit-title boundary.

Delete the `PostUpdate` / `PostCreation` doc comment that says naming the struct
is the mitigation for `title`/`slug_override` transposition. After this change
the type, not the comment, prevents mixing a title and slug.

### D2 — Render and ETag views preserve content types

`web::posts::render::PostView` should expose `title: Option<&PostTitle>` and
`summary: Option<&PostSummary>`. Existing markup output must remain
byte-for-byte equivalent for title and summary text: the ADR-0063 string-newtype
trailer makes the values render and coerce as strings without unwrapping.

`server::atompub::posts::etag_for` should build its `EtagContent` with
`Option<&PostTitle>`, `&PostBody`, and `Option<&PostSummary>` instead of
borrowed `str`s. The ETag payload must remain the same JSON shape, because the
derived serde bridge serializes these newtypes as plain strings.

### D3 — Test builders and fixture helpers should be typed at the seam

The issue-listed helper shapes are test support, but they still train call sites
into adjacent primitive parameters. Update them to carry the existing domain
types:

- `storage::test_support::SeedPost` stores `Option<PostTitle>` and its setter
  accepts a `PostTitle`.
- `storage::post_service`'s `creation_with_key` test helper and the adjacent
  service test helpers in `storage::test_support` accept `PostBody` where they
  currently accept a body `&str`.
- `server::atompub::mapping::MakePost` uses `PostId`, `Option<PostTitle>`,
  `Slug`, `PostBody`, `Option<PostSummary>`, and typed tag slug/display values
  for the matching fields.
- `common::feed::json` test helpers use `Option<PostTitle>` and
  `Option<PostSummary>` for item title and summary.
- `server::atompub::posts` test-only tag helpers use `PostId`, `TagId`, `Tag`,
  and `TagLabel` where those newtypes already exist.

String literals in tests should be parsed at the call site with the existing
helpers (`parse_post_title`, `parse_post_body`, `parse_post_summary`,
`parse_slug`) or local equivalents. Do not add lossy `impl From<&str>`
convenience paths to the domain types.

### D4 — No product behavior or wire-shape change

This issue is a type-threading change only. It must not change:

- title presence semantics: omitted/blank explicit title still behaves as
  absent;
- body canonicalization or title extraction rules from ADR-0105;
- Post summary validation/truncation rules from ADR-0101;
- rendered HTML, AtomPub XML, JSON Feed output, ETag JSON shape, or storage
  rows.

Any new parse failure introduced by moving a boundary inward is a bug unless the
same invalid input already failed at that boundary before this cycle.

## Acceptance criteria

- **AC1 — service structs are typed.** `PostCreation.title` and
  `PostUpdate.title` are `Option<&PostTitle>` (or an equivalent borrowed typed
  shape), and no mitigation comment remains claiming a `&str`/`Slug`
  transposition is handled by naming alone.
- **AC2 — naming derivation accepts typed explicit titles.**
  `derive_post_naming` takes a typed explicit title; web and AtomPub boundaries
  still convert blank submitted titles to absence before storage.
- **AC3 — view/ETag seams are typed.** `PostView` uses `Option<&PostTitle>` /
  `Option<&PostSummary>`, including `web::posts::component` construction sites,
  and `etag_for`'s serialization helper uses `PostTitle`, `PostBody`, and
  `PostSummary` references without changing the serialized content fields.
- **AC4 — listed fixture helpers are typed.** The issue-listed `MakePost`,
  `SeedPost`, `creation_with_key`, JSON Feed item helper, and AtomPub post/tag
  test helpers no longer accept adjacent bare `&str`/`i64` values where
  `PostTitle`, `PostBody`, `PostSummary`, `PostId`, `TagId`, `Slug`, `Tag`, or
  `TagLabel` already exist.
- **AC5 — call sites migrate cleanly.** All affected callers construct or borrow
  the domain values at their real boundary; no deprecated aliases, primitive
  shims, or blanket `From<&str>` shortcuts are added.
- **AC6 — behavior is unchanged.** Existing service-layer, AtomPub, JSON Feed,
  ETag, and web-render tests that cover the touched paths pass. Any added test
  must lock a type-threading invariant or unchanged output shape, not duplicate
  constructor tests already owned by the newtypes.

## Out of scope

- Introducing new domain types.
- Changing `PostTitle`, `PostBody`, `PostSummary`, `Slug`, `TagLabel`, `PostId`,
  or `TagId` invariants or generated trailers.
- Adding a new xtask adoption gate; that belongs to #697.
- Typing unrelated same-primitive hazards not listed in #694.
- Changing storage schemas, migrations, API payload shapes, rendered markup, or
  ETag semantics.

## Risks

- **Boundary drift.** Moving `PostTitle` inward can accidentally reject blank
  explicit titles instead of treating them as absence. Keep that policy at the
  web/AtomPub boundary and cover it with existing naming tests.
- **Fixture churn.** Test helpers become slightly noisier because literals must
  parse before use. Prefer existing parse helpers over broad convenience
  conversions so tests still name the boundary.
- **Serde confidence.** ETag content can use typed references only if the output
  JSON remains identical. Preserve or add a focused assertion if existing ETag
  tests do not pin this.
