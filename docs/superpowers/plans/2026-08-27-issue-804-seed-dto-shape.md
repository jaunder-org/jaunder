# Seed DTO Shape Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for delegated tasks.
> This outline exists because the change cuts over shared Rust and serialized
> server-function/seed API shapes.

## Scope

In:

- Derive `RenderedPost` draft state from its publication time and remove the
  redundant serialized field everywhere.
- Replace both concrete pagination envelopes with direct uses of one generic
  page type.
- Prove the intended wire change and the required wire compatibility behavior.

Out:

- AtomPub lifecycle parsing.
- Row DTO consolidation, listing membership, ordering, and pagination semantics.
- Compatibility aliases or decoding shims for the removed shapes.

## Task outline

- [x] Task 1: Make draft state derived and preserve publication-state behavior
  - Contract: `RenderedPost::is_draft()` returns exactly
    `self.published_at.is_none()`; scheduled Posts remain non-drafts. The
    serialized `RenderedPost` shape no longer contains `is_draft`.
  - Verification: focused common/web/server tests prove draft, scheduled, and
    published classification; serialization omits the key; the existing draft
    lifecycle browser flow still proves banner and Publish/Unpublish
    transitions.
- [ ] Task 2: Cut all page envelopes over to generic `Page<Row>`
  - Contract: `common::seed::Page<Row>` is the canonical public type and owns
    `posts`, `next_cursor`, and `has_more` in their current declaration order.
    Every former concrete page signature, constructor, fixture, and export uses
    `Page<RenderedPost>` or `Page<UnpublishedPost>` directly; construction
    helpers remain specialized where row conversion or cursor semantics differ.
  - Verification: pre-change-compatible golden bytes cover both row
    instantiations; focused endpoint/projector tests prove decoding and
    pagination behavior; a search scoped to maintained Rust source and module
    documentation finds neither obsolete concrete page name.

## Risk checks

- Distinguish the intentionally removed `RenderedPost.is_draft` key from the
  unchanged page envelope bytes.
- Do not alter AtomPub's separate parsed draft input.
- Keep future publication times non-draft even where unpublished listings
  include them.
- Do not genericize row conversion or cursor derivation: rendered, draft, and
  scheduled listings have different domain ordering inputs.
- Preserve serde field order and inferred generic bounds on host and wasm
  targets.
- Update every Rust caller, re-export, fixture, integration assertion, and
  relevant module documentation in the clean cutover.
- Run the repository's focused lanes during iteration and `cargo xtask validate`
  before shipping.
