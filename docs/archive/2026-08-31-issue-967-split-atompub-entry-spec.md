# Split AtomPub Entry concerns

## Outcome

The host-owned AtomPub Entry implementation is organized into cohesive leaf
modules while retaining its existing public/test interfaces and wire behavior.
Each marker or rendering contract and its tests live with the concern that owns
them.

## Load-bearing decisions

- Replace `host/src/atompub/entry.rs` with a wiring-only
  `host/src/atompub/entry/mod.rs` containing module documentation, attributes,
  private declarations, and explicit re-exports only, per ADR-0128.
- Preserve every existing `host::atompub::entry::*` path and the parent
  `host::atompub::*` re-export surface; the issue's `common` path predates the
  completed #855 host relocation.
- Split implementation into private `foreign_markers.rs`, `entry_document.rs`,
  `collection.rs`, `media_member.rs`, and `render.rs` leaves.
- Keep foreign namespace resolution, prefix ownership, pruning, `app:draft`, and
  `j:slug` read/write helpers together in `foreign_markers.rs`.
- Keep standalone Member Entry serialization and `entry_to_xml` in
  `entry_document.rs`.
- Keep `FeedMeta` and Collection Feed rendering in `collection.rs`; keep
  `MediaLinkEntry` and media Member rendering in `media_member.rs`.
- Keep shared upstream Atom XML serialization, typed error mapping, and
  relation-link construction private in `render.rs`; do not expose a new helper
  path.
- Move each existing unit test beside the implementation or contract it proves;
  cross-contract marker/serialization tests belong to the marker behavior they
  observe.
- Preserve upstream `atom_syndication` document I/O and byte output,
  Jaunder-owned extension-map semantics, typed URL roles, and existing error
  behavior under ADR-0023 and ADR-0089.
- Do not broaden foreign-extension normalization or fix the element-scoped
  namespace bug tracked by #813.

## Acceptance

- Every implementation leaf has one named responsibility; `mod.rs` only
  assembles and documents the module surface.
- Existing host and server callers compile without import-path migration,
  compatibility aliases, or new public exports.
- Existing standalone Entry, Collection Feed, media Member, `app:draft`, and
  `j:slug` contracts remain unchanged: parsed values, emitted elements and
  attributes, namespace ownership, link roles, timestamps, and typed errors.
- All existing unit tests move with their owning contracts and retain their
  names, assertions, fixtures, and error coverage.
- The test-enabled repository gate (`cargo xtask check`) passes on the complete
  split.

## Boundaries

- No AtomPub protocol, mapping, HTTP route, upstream-library, error-policy, or
  domain-vocabulary change.
- No new public helper, interface rename, deprecation, compatibility shim, or
  unrelated stale-path cleanup.
- No ADR change: the refactor implements existing serializer, wire-extension,
  ownership, and module-assembly decisions.
