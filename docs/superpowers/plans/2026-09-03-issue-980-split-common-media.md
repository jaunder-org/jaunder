# Split Common Media Types Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` where useful. This
> outline exists because the refactor crosses a durable module boundary while
> preserving a broad public API and ADR-coupled filename/layout invariants.

## Scope

In:

- Replace `common/src/media.rs` with the approved six leaves and assembly-only
  `common/src/media/mod.rs`.
- Preserve every public/test interface, behavior, test case, and wire/SQL trait
  implementation.
- Make common-internal ownership paths explicit and update stale architecture
  paths.

Out:

- Renames, API additions, compatibility aliases, behavior or policy changes.
- Changes to storage, filesystem, server, web, or AtomPub behavior.
- New filename/address representations or ADRs.

## Task outline

- [x] Task 1: Extract the six media concerns behind the stable module surface
  - Contract: create `hash.rs`, `filename.rs`, `storage.rs`, `references.rs`,
    `mime.rs`, and `values.rs`; `mod.rs` explicitly re-exports the complete
    pre-split public surface and contains assembly only. `storage.rs` owns both
    `/media/` layout emission and a crate-private recognition seam;
    `references.rs` consumes that seam and separately owns AtomPub member
    recognition. `filename.rs` alone owns encoding/intake policy.
  - Verification: the focused `common` unit and doctest surface passes with all
    existing cases moved beside their owning implementation.
- [ ] Task 2: Migrate owner paths and prove repository-facing compatibility
  - Contract: common-internal callers and `common::test_support` import their
    crate-private leaf owners; cross-crate callers retain `common::media::*`.
    Compare the pre-split public declarations against explicit re-exports and
    remove any accidental new or missing surface. Update only stale media owner
    paths in `docs/ARCHITECTURE.md`.
  - Verification: focused media contracts in `common`, `storage`, and `jaunder`
    pass; the final commit gate validates formatting, clippy, doctests, and the
    unchanged public/test surface.

## Risk checks

- `Filename` remains the canonical encoded database/disk/URL spelling; only its
  intake doors sanitize, truncate, or encode.
- `path` and `url` remain byte-identical layout projections and never re-encode
  a `Filename`; the `/media/` parser shares their storage-owned grammar.
- AtomPub member parsing remains a distinct references-owned grammar.
- Hash, filename, reference, MIME, size/quota, and `UploadedMedia` derives,
  errors, serde keys, SQL bridges, ordering, and equality remain unchanged.
- Every existing test and compile-fail example moves intact; no test is narrowed
  or silently dropped.
- `mod.rs` contains only module documentation, declarations, attributes, and
  explicit re-exports per ADR-0128.
- No `#[allow(...)]` or `#[expect(...)]` is introduced without explicit user
  approval. Commits contain no `Co-Authored-By` trailer.
