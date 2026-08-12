# ADR-0124: rstest_reuse templates resolve cross-module by bare name

- Status: accepted
- Date: 2026-08-11

## Context

The dual-backend test matrix (`#[apply(backends)]`, `#[apply(backends_matrix)]`)
is defined once in `storage::test_support` and applied across test crates. How
`rstest_reuse` resolves a `#[template]` across module and crate boundaries is
not documented upstream and was established by a spike; the findings lived only
in test-file comments, restated in several places.

## Decision

Rely on these verified properties, and keep the matrix templates in
`storage::test_support`:

- A `#[template]` expands to a name-mangled `macro_rules!` that a plain
  `use storage::test_support::backends;` brings into scope. `#[apply(backends)]`
  then resolves it by bare name — no `#[apply(path::to::template)]` and no
  `pub use` re-export.
- `use rstest_reuse::*` alone is not enough at a crate root: the expansion names
  the `rstest_reuse` crate path, so a bare `use rstest_reuse;` must also be
  present.
- The backend axis of `backends_matrix` is `#[values]`-based, because a
  `#[case]`-based axis cannot coexist with a test's own named `#[case]` rows; it
  composes as rows × backends. Attribute order: `#[apply(backends_matrix)]`,
  then the `#[case::name(..)]` rows, then `#[tokio::test]` (#127).

> **Annotation (2026-08-12).** The second bullet's extra requirement does not
> hold in this tree. No bare `use rstest_reuse;` exists anywhere in `server/` or
> `storage/` — every site uses `use rstest_reuse::*;` (or
> `use rstest_reuse::template;`) alone, and `server/tests/main.rs` imports the
> crate not at all. The rule appears to have been hoisted from a stale test-file
> comment. The rest of this ADR — bare-name resolution and the `#[values]`-based
> backend axis — is unaffected. Current inventory:
> [ARCHITECTURE.md](../ARCHITECTURE.md).

## Consequences

- Test files reference this draft instead of re-deriving the resolution rules in
  comments.
- If rstest_reuse changes its expansion (name mangling, crate-path reference),
  the failures will be import-resolution errors at the `use` sites named here.
