# RenderedHtml Reconstruction Implementation Outline

> Execute with dev-cycle-iterate. This outline exists because the change moves a
> security invariant across a public crate boundary and removes its static gate.

## Scope

In:

- Move sanitizer ownership and the only public production constructor into
  `common::render` behind an optional host-only feature.
- Confine raw reconstruction to common-private SQLx/serde paths and test
  support.
- Migrate every caller, delete the superseded gate, and amend existing
  decisions.

Out:

- Sanitizer-policy, rendered-byte, media-extraction, storage-schema, and
  wire-format changes.
- General `SqlxBridge`, newtype derive, or typed SQLx bind redesign.

## Task outline

- [x] Task 1: Add common-owned sanitization and reconstruction seams
  - Contract: add optional `common` feature `sanitize`; expose
    `common::render::sanitize(&str) -> RenderedHtml` only with that feature;
    keep SQLx decode and seed deserialization common-private; expose
    `common::test_support::rendered_html(&str) -> RenderedHtml` only under
    `cfg(test)` or `test-support`. Existing callers remain until Task 2.
  - Verification: focused common tests preserve sanitizer behavior and seed DTO
    serialization/field-specific deserialization; storage post and revision
    decoding on SQLite and Postgres preserves rendered bytes without
    re-sanitization or new failure behavior.

- [x] Task 2: Cut every caller over and close the public raw door
  - Contract: `host` enables `common/sanitize`; rendering and other production
    callers invoke `common::render::sanitize` directly; exact fixtures use
    `common::test_support::rendered_html`; after every caller migrates, delete
    the host sanitizer alias and generic trusted constructor.
  - Verification: compile-fail contracts prove no raw external production
    construction, no blanket deserialization, and fixture-helper feature
    confinement; focused host render suites preserve sanitizer bytes, all three
    authoring formats, allowed code-block classes, and media-reference
    classification; the CSR/wasm dependency graph contains no sanitizer.

- [ ] Task 3: Remove aftermarket policy and record the new ownership
  - Contract: delete the `rendered-html-from-trusted` xtask step, registration,
    tests, and allowance markers; amend `common/src/render.rs`, ADR-0079,
    ADR-0123, and `docs/ARCHITECTURE.md` to describe sanitizer establishment,
    private trusted reconstruction, test-only fixtures, and the accepted SQLx
    column-typing review responsibility after gate removal.
  - Verification: repository search finds no gate symbol, marker, generic
    trusted constructor, or production raw fixture door; focused tests and
    `cargo xtask validate` pass.

## Risk checks

- `ammonia` and its parser dependencies remain absent from the CSR/wasm graph.
- Sanitizer allowlist and resulting bytes do not change during the ownership
  move.
- The host media-classification check still observes the sanitizer's complete
  permitted attribute surface.
- SQLx decoding continues to construct the private field directly and does not
  sanitize, allocate an extra copy, or become fallible.
- Seed DTO representation and field-specific deserialization remain unchanged.
- Feature unification does not expose raw fixture construction in production.
- Every former `from_trusted` caller is migrated before the constructor and gate
  are removed; no alias or compatibility path remains.
