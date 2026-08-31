# Centralize Tag Summary Conversion

## Outcome

Every owned `TagLabel` converted into a `TagSummary` uses one domain-owned
conversion while preserving canonical slug identity and case-preserving display
behavior.

## Load-bearing decisions

- Implement `From<TagLabel> for TagSummary` beside `TagSummary` in `common`.
- The conversion consumes the label, derives its canonical slug before moving
  the original label into `display`, and performs no cloning.
- Do not add `From<&TagLabel>`; no current caller has a borrowed conversion
  need.
- Migrate all five eligible current constructions: the tags list endpoint, its
  API conversion test, the committed-input parser, and the input-logic and
  input-state test helpers.
- Relocate the API conversion test beside `TagSummary` in `common`, where it
  directly owns the mixed-case display and canonical-slug conversion contract.
- Preserve the tags list endpoint's current display: catalog rows still expose
  their canonical lowercase slug as both identity and display because they carry
  no author-cased label.
- Update `TagSummary` documentation to describe both valid display sources:
  author-cased labels from tagging rows and canonical-slug fallback from catalog
  rows that carry no separate label.
- Preserve ADR-0068's distinction between canonical `Tag` identity and
  case-preserving `TagLabel` presentation.

## Acceptance

- All five eligible owned-label constructions are migrated or, for the API
  conversion test, relocated to use the standard conversion.
- A unit test beside `TagSummary` proves mixed-case labels retain their original
  display and derive lowercase slugs.
- The tags list endpoint returns the same slug and display values as before.
- `PostTag` conversions retaining authoritative slug/display pairs and literal
  test fixtures remain unchanged.
- Existing type and function signatures, serialized shapes, and test interfaces
  remain unchanged; the new `From<TagLabel>` implementation is the intended
  additive Rust interface.
- Focused common/web tag tests pass.
- `cargo xtask check` passes.

## Boundaries

- Do not change tag parsing, validation, trimming, equality, deduplication, wire
  format, persistence, endpoint sorting, or prefix behavior.
- Do not absorb #694 or #697.
- No domain glossary or ADR change is required; this conversion implements the
  existing tag identity/label model.
