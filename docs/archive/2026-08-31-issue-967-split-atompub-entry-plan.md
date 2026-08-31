# Split AtomPub Entry implementation outline

> Execute with `jaunder-iterate` and `jaunder-dispatch`. This outline exists
> because an atomic public module-facade cutover must preserve extension
> namespace, upstream serializer, private visibility, doctest, and cross-crate
> caller contracts across six Rust files.

## Scope

In:

- Atomically replace `host/src/atompub/entry.rs` with the approved wiring-only
  facade and five cohesive implementation leaves.
- Move every existing unit test beside the implementation or observable contract
  it proves.
- Preserve every existing nested/root re-export, upstream Atom behavior, marker
  contract, rendering contract, and typed error/URL boundary.

Out:

- The unknown-extension namespace normalization bug tracked by #813.
- AtomPub mapping, HTTP route, upstream dependency, protocol, or
  domain-vocabulary changes.
- Historical stale-path cleanup outside current code and authoritative
  architecture.

## Task outline

- [x] Task 1: Land the atomic AtomPub Entry module cutover and prove the
      unchanged marker/rendering contracts.
  - Contract: `host/src/atompub/entry/mod.rs` explicitly re-exports the
    unchanged facade from private `foreign_markers`, `entry_document`,
    `collection`, and `media_member` leaves; private `render` supplies shared
    upstream XML/error/link helpers without adding a public path;
    `host/src/atompub/mod.rs` root re-exports remain unchanged.
  - Ownership: one integration owner assembles `mod.rs`, removes the old source
    file, reconciles private visibility and shared test fixtures, and verifies
    the complete tree; delegated leaf extraction may proceed in parallel only
    under that facade contract.
  - Verification: run the focused host-native `host` AtomPub tests first, then
    the test-enabled repository gate `cargo xtask check`; unchanged server
    callers, nested doctests, and moved error/namespace tests are required
    evidence.

## Risk checks

- `host/src/atompub/mod.rs` retains `pub mod entry` and its existing explicit
  root re-export list unchanged.
- `entry/mod.rs` contains only module documentation, attributes, declarations,
  and explicit re-exports under ADR-0128.
- Re-export coverage is exhaustive, with no alias, compatibility shim, public
  leaf module, or new shared-helper path.
- `foreign_markers.rs` retains namespace resolution, prefix selection, pruning,
  and Jaunder-owned marker behavior without broadening normalization to #813.
- All Atom documents still serialize through upstream `atom_syndication`; no XML
  postprocessing, serializer replacement, or output-layout promise appears.
- `collection.rs` and `media_member.rs` retain typed link roles, timestamps,
  titles, content, paging, and relation semantics; `entry_document.rs` retains
  standalone Entry error mapping.
- Cross-concern tests are homed by the behavior they prove; existing names,
  assertions, fixtures, doctests, and typed writer-error coverage move rather
  than being deleted or weakened.
- Cross-leaf helpers use the narrowest visibility that compiles; no lint
  suppression is introduced without explicit approval; commits contain no
  `Co-Authored-By` trailer.
