# Issue #721 — Reclaim unreferenced media files on delete

## Outcome

Deleting a media item removes its database row and, when the same on-disk media
entry is no longer named by live content or another media row, unlinks the
stored file. A URL for successfully reclaimed media stops serving instead of
falling through to the extension-derived 200 path.

## Load-bearing decisions

- Reclaim the on-disk entry named by the canonical `MediaRef` triple
  `(source, sha256, filename)`, using `common::media::media_path`; do not
  reclaim by hash alone. Identical bytes stored under different filenames are
  different entries, even when the filesystem hard-links them.
- A successful delete must not create a referenced, rowless file. The storage
  decision may remove the caller's `media` row only when either no live
  `post_media` reference names the entry, or some other `media` row still names
  the same `(source, sha256, filename)` and therefore still accounts for the
  on-disk entry.
- A file is reclaimable only after the caller's `media` row delete succeeds and
  no remaining `media` row and no live `post_media` reference anywhere names the
  same `(source, sha256, filename)` entry.
- `force` may override the caller's own reference guard only when the row delete
  will not leave a referenced file with no media row. Otherwise it reports a
  referenced refusal and leaves row and file intact.
- Reclamation runs synchronously in the delete path after the storage decision.
  `RefusedReferenced` and `NotFound` do not touch the filesystem.
- Missing files count as already reclaimed. Unexpected filesystem failures are
  surfaced as delete failures with the storage decision already made, not hidden
  as success.

## Acceptance

- Deleting an unreferenced media item removes its row, removes the file at
  `storage/media/<media_path>`, and the public media URL returns not found.
- Deleting a media item still referenced by a live Post without `force` refuses
  and leaves both row and file intact.
- Forced delete of a still-referenced media item with no remaining media row
  refuses and leaves both row and file intact.
- Forced delete of a still-referenced media item with another remaining media
  row for the same `(source, sha256, filename)` removes the caller's row but
  leaves the file in place.
- Deleting one of multiple media rows for the same `(source, sha256, filename)`
  leaves the file in place until the last row is gone.
- Deleting one filename for a content hash does not remove a different filename
  entry, even if both entries are hard links to the same bytes.
- Upload quota accounting and the media directory agree after successful delete:
  retained paths are still named by at least one media row, and fully
  unreferenced paths are absent.
- Web media deletion and AtomPub media member `DELETE` both use the same
  reclamation behavior.

## Boundaries

- No schema migration and no `post_media` key changes.
- No change to upload intake, media URL layout, canonical filename rules, or
  content-type detection.
- No background sweeper or audit trail in this issue.
- No change to the advisory list of referencing posts shown by the web delete
  UI.
