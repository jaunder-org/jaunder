# Issue #980: Split common media types by concern

## Outcome

Replace the 2,155-line `common/src/media.rs` with six focused leaves under
`common/src/media/` and an assembly-only module surface. Existing
`common::media::*` and test-support paths, wire representations, validation,
layout, and observable behavior remain unchanged.

## Load-bearing decisions

- `common/src/media/mod.rs` declares six crate-private leaves and explicitly
  re-exports the existing public surface. It contains no behavior, types,
  constants, implementations, or tests, per ADR-0128.
- `hash.rs` owns `ContentHash`, `InvalidContentHash`, canonical hash validation,
  and trusted digest construction.
- `filename.rs` owns `Filename`, `InvalidFilename`, filename intake, canonical
  percent encoding, safe-leaf validation, truncation, the encoded-byte budget,
  and `MAX_FILENAME_ENCODED_BYTES`.
- `storage.rs` owns `MediaSource`, `MediaRef`, the content-addressed `path` and
  `url` layout, and recognition of that same `/media/` stored-object grammar,
  including existing shard, decoding, and canonicalization checks.
  `references.rs` consumes a crate-private recognition seam from this owner
  rather than duplicating that layout grammar.
- `references.rs` owns `MediaReferenceKind`, `MediaReferenceForm`, their errors,
  `MediaReference`, `parse_media_url`, URL-form classification, and recognition
  of the distinct `/atompub/<user>/media/...` member grammar. It combines either
  recognized form into the existing `MediaReference` result.
- `mime.rs` owns `ContentType`, `InvalidContentType`, MIME validation, extension
  detection, and inline-versus-attachment policy.
- `values.rs` owns `MaxFileSize`, `UserQuota`, `ByteSize`, and the public
  `UploadedMedia` wire value.
- Dependency direction follows those responsibilities: storage consumes typed
  hashes and filenames; references consumes storage addressing; MIME consumes
  typed filenames; values composes the public typed values. Cycles and duplicate
  validators or parsers are not introduced.
- ADR-0080 and ADR-0084 remain exact: `Filename` is the single canonical encoded
  database/disk/URL spelling; only filename intake uses the private encode set;
  stored `path` and `url` interpolate the typed filename without re-encoding;
  the stored layout has one emitter and one recognition owner.
- Every existing public `common::media::*` item keeps its path, visibility,
  trait implementations, derives, error text, serialization, SQL bridge,
  ordering, and equality behavior. No compatibility aliases or new public leaf
  paths are added.
- Within the `common` crate, consumers name the crate-private leaf owner.
  Cross-crate consumers continue using the stable `common::media::*` API.
- Pure tests and compile-fail documentation move with the item or policy they
  prove. Cross-crate and integration contracts remain in their existing homes.
  Existing test-support exports and helper behavior remain unchanged.
- Issue #782 is already complete: `UploadedMedia` is the current public name and
  its five fields and serde keys remain unchanged. This issue performs no DTO
  rename or shape audit.

## Acceptance

- `common/src/media/` contains exactly `mod.rs`, `hash.rs`, `filename.rs`,
  `storage.rs`, `references.rs`, `mime.rs`, and `values.rs`, each with one named
  responsibility.
- `mod.rs` is assembly-only and uses explicit re-exports; every pre-split public
  and test-support path resolves unchanged, with no new public API.
- Hash, filename, source, reference, content-type, byte-limit, quota, size, and
  upload-wire tests retain every case and assertion under their owning leaves.
- Stored-media path and URL output are byte-identical to the pre-split behavior,
  including hash fan-out, source segment, canonical filename interpolation, and
  `RootRelativeUrl` construction.
- Filename sanitization, truncation, encoded-length limits, decoded-segment
  intake, error messages, and canonicality checks are unchanged.
- Media URL parsing, reference kind/form classification, ordering, and
  deduplication are unchanged. Stored `/media/` URLs reuse storage-owned layout
  recognition; the distinct AtomPub member grammar remains references-owned.
- MIME detection, MIME validation, and content-disposition policy are unchanged.
- `UploadedMedia` retains the same five fields, field types, and wire keys.
- `docs/ARCHITECTURE.md` names the new owners instead of stale
  `common/src/media.rs` line locations.
- Focused `common` tests and the repository pre-commit gate pass.

## Boundaries

- No media semantics, limits, accepted syntax, wire schema, or public names
  change.
- No filesystem, storage-crate, server, web, or AtomPub behavior is redesigned;
  their edits are limited to compilation-preserving import/path migration where
  required.
- No new filename representation, public address type, façade, compatibility
  shim, or generated API is introduced.
- No ADR is needed: this projects existing ADR-0080, ADR-0084, and ADR-0128
  decisions into a more cohesive module layout.
