# ADR-0068: A domain value with a canonical identity and a preserved label is two newtypes, not one

- Status: proposed
- Date: 2026-07-14
- Issue: [#409](https://github.com/jaunder-org/jaunder/issues/409)

## Context

ADR-0063 makes each domain value a validated newtype parsed at the outermost
boundary. Threading `Tag` end-to-end (#409) surfaced a value that resists the
one-type-per-value shape: a **tag** has two facets that live at **different
cardinalities**, in different tables.

- **Canonical slug** — one shared, interned row in `tags(tag_id, tag_slug)`. It
  is the unique key, the browse key (`/tags/rust`), the dedup key, and the
  feed-surface key. There is exactly **one** `rust` system-wide.
- **Display label** — one row per _tagging_ in
  `post_tags(post_id, tag_id, tag_display)`. Post A tags `Rust`, post B tags
  `rust`; both reference the same `tags.rust` row but each preserves its own
  casing. Label is **1-per-association (N)**; slug is **1-per-identity**.

The label is a real, surfaced value (a post's own tag list renders the author's
casing), and preserving it is required — `Tag::from_str` lowercases, so a label
cannot be stored _as_ a `Tag` without destroying the casing.

Two shapes were considered and rejected:

- **A single two-member `Tag { canonical, display }`.** It cannot use the
  `StrNewtype` derive (ADR-0062/0063), and it forces a `display` into every
  canonical-only context — browse, catalog (`TagRecord`), dedup `HashSet`,
  `FeedSurface::SiteTag` — where no single label exists (whose casing?). That
  conflates _a tag_ (identity) with _a tagging_ (its application to one post).
- **Leaving the label a bare `String`.** It leaves an unvalidated value on the
  boundary (its slug is _not_ guaranteed to parse), re-creating exactly the
  thin-type hazard the milestone exists to remove, and it is not "a tag value is
  never a bare string."

## Decision

**Model a canonical-identity-plus-preserved-label value as two composable
newtypes, paired only where both travel.** For tags:

- **`Tag`** — the canonical slug (`#[derive(StrNewtype)]`, hand-written
  lowercasing/validating `FromStr`). The identity: browse, catalog, dedup, and
  SQL keys use `Tag` alone.
- **`TagLabel`** — the case-preserving label (`#[derive(StrNewtype)]`,
  hand-written `FromStr` that trims, rejects empty, and validates that the
  trimmed input has a valid canonical form; the original casing is stored). It
  exposes `slug(&self) -> Tag` (infallible by the construction invariant). Every
  place a label travels — `PostTag.tag_display`, `tag_post`, `post_tag_diff`'s
  desired set, atompub category ingest, `TagSummary.display`, the create/update
  wire arg, and `parse_and_validate_tags`'s output — is a `TagLabel`, never a
  bare `String`.

The two are paired explicitly where the N-side needs both:
`PostTag { tag_slug: Tag, tag_display: TagLabel }`,
`TagSummary { slug: Tag, display: TagLabel }`. Equality/dedup on labels is **by
slug** (`TagLabel::slug`), never by raw casing — collapsing `Rust`/`rust` is a
canonical-slug operation on `Tag`, which keeps `Hash`/`Ord`.

**One validity source.** Both `TagLabel::from_str` (client pre-validation, wire
`Deserialize`) and the derived slug funnel through `Tag`'s rule — no
re-implemented validator anywhere (retires the #416 `is_valid_tag_slug` drift).
The web `TagInput` validates the **raw (trimmed) input** — it no longer
lowercases before constructing the label, so web-authored labels now preserve
the author's casing (the casing distinction `TagLabel` exists to carry),
consistent with atompub ingest.

## Consequences

- `Tag` and `TagLabel` are both ADR-0063 newtypes; no tag value is ever a bare
  `String`/`&str` across `common`/`storage`/`host`/`server`/`web`.
- No wire/schema change: `TagLabel` serializes as its label string (the current
  wire), and the `tags`/`post_tags` columns are untouched. One deliberate
  behavior change: the web `TagInput` now preserves author casing in
  `tag_display` (it previously lowercased), so casing distinctness originates
  from **both** web and atompub rather than atompub alone. A no-op for existing
  rows.
- This **amends ADR-0063**: a domain value is not always exactly one newtype —
  when it carries a canonical identity _and_ a preserved presentation variant at
  different cardinalities, it is two, composed at the pairing site.
- Rules out: a two-member conflated `Tag`; a bare-`String` label; and deduping
  labels by raw casing (identity is always the slug).
- The pattern generalizes (identity + preserved-label) but is applied only to
  tags here; future adopters cite this ADR rather than re-deciding.
