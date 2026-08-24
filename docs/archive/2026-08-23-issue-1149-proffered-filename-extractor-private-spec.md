# Issue #1149 — Make `ProfferedFilename` extractor-private

## Outcome

Normal repository code can no longer name `ProfferedFilename` as a cross-crate
domain value. The decoded route-segment door remains available only inside the
server extractor layer, and every route converts to canonical `Filename` before
handler/domain/storage code sees the value.

## Load-bearing decisions

- `Filename` remains the only media filename type allowed beyond extraction. It
  continues to mean the canonical percent-encoded safe leaf defined by ADR-0084.
- The decoded-segment parser keeps the ADR-0084 behavior: Axum has already
  percent-decoded the segment, the door applies the safe-leaf oracle to that
  decoded text, percent-encodes once, rejects over-budget values, and converts
  infallibly to `Filename`.
- The extractor-private decoded intermediate should live in `server`, not
  `common`, because only server route extraction needs the decoded twin.
  `common::media` should expose `Filename` plus a validating common-owned
  decoded-segment conversion that preserves the current safe-leaf, encoding, and
  budget checks; it must not expose an unchecked `Filename` constructor.
- Public media serving and AtomPub media member `GET`/`DELETE` must all use
  extractor-owned wrappers or extractors whose public handler signatures name
  `Filename`/validated address types, not `ProfferedFilename` or any renamed
  decoded filename intermediate.
- The `proffered-filename-position` gate should stop defending ordinary type
  positions for a public cross-crate type. It is deleted if privacy makes the
  leak impossible, or reduced to a migration detector that fails on any
  remaining public `common::media::ProfferedFilename` exposure or
  decoded-intermediate use outside extractor implementation code.
- ADR-0084 and ADR-0140 documentation must be amended so they no longer claim
  `ProfferedFilename` must be public for route signatures. They should explain
  the extractor-private decoded-segment seam and reaffirm that `Filename` is the
  only type past extraction.

## Planning note

This spec requires a short implementation outline after approval: it changes a
cross-crate type boundary, Axum extractor contracts, a static gate, and accepted
ADR text.

## Acceptance

- No production crate outside extractor implementation code can import, name,
  store, return, accept, or serialize `ProfferedFilename` or any other decoded
  filename intermediate. Handler/domain/storage/web surfaces expose only
  `Filename` or validated address types.
- `server/src/media.rs` still rejects malformed media serve addresses before the
  handler and still recovers canonical stored filenames from decoded URL
  segments.
- AtomPub media member `GET` and `DELETE` still reject malformed filename
  segments as pre-handler 400s and still find/delete records whose stored
  filename contains percent-encoded characters such as spaces.
- The static gate surface no longer permits a broad public `ProfferedFilename`
  cross-crate interface; removed or rewritten checks pass for the new seam.
- Existing media/AtomPub filename behavior tests continue to pass, with any new
  or adjusted tests proving privacy/gate behavior rather than re-testing percent
  encoding by source text.
- Documentation changes cite the accepted ADRs being amended and describe the
  one-hop decoded-segment invariant.

## Boundaries

- Do not change media URL layout, on-disk layout, database representation, or
  `Filename`'s canonical encoded invariant.
- Do not change public HTTP status policy except where an already-malformed
  filename is rejected by the same pre-handler extractor path as today.
- Do not add a storage/sqlx/serde/display trailer for the decoded-segment
  intermediate.
- Do not broaden this work into the remaining milestone type-safety issues.
