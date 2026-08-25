# Issue #1103 — Media filesystem usage

## Outcome

When OpenTelemetry export is configured, Jaunder periodically reports the
logical filesystem usage of its configured media tree. Operators can distinguish
this physical-drift diagnostic from the existing database-declared upload-byte
accounting metric.

## Load-bearing decisions

- Keep `jaunder.media.storage_bytes` unchanged: it remains the database-declared
  total of upload Media bytes, with quota/accounting semantics.
- Add `jaunder.media.filesystem_bytes`, measured in `By`, for the complete
  `<storage_path>/media` tree. It includes `upload`, `cached`, `tmp`, orphaned
  files, and future descendants.
- Measure the logical length of every regular directory entry. Each hard link
  contributes its own entry length; this is namespace-level logical usage, not
  allocated-block or filesystem-quota usage.
- Reuse the existing serve-owned, OTLP-gated 30-second saturation sampler and
  its immediate first collection. Collection must not run on a request path.
- A sample is all-or-nothing. A missing or unreadable path, traversal error,
  symlink, or non-regular non-directory entry fails the collection, reports the
  established bounded diagnostic, and publishes no datapoint. Such entries need
  operator intervention; they are not silently skipped.
- Keep observable callbacks synchronous snapshot reads. Run filesystem walks on
  Tokio's blocking-work facility from the periodic collection path; await each
  result before beginning the next scan, so only one walk is in flight.

## Acceptance

- With an OTLP endpoint configured and regular files below the media root, the
  filesystem metric reports the sum of their logical lengths and remains
  distinct from the database-declared upload-byte metric.
- A file in `cached` or `tmp`, or an orphaned regular file, contributes to the
  filesystem metric even without a corresponding Media row.
- Multiple hard-linked regular entries each contribute their logical length.
- A symlink, special file, missing/unreadable path, or traversal failure records
  the fixed `server.metrics.media_filesystem_bytes` diagnostic with its
  collection error and causes the filesystem metric to emit no datapoint for
  that sample; it never reports zero or a partial value.
- A deliberately blocking filesystem traversal does not occupy an async runtime
  worker, and the sampler never overlaps two filesystem walks.
- The metric is absent and no collector runs when OTLP export is unconfigured.
- Targeted coverage exercises all collection outcomes without putting a
  recursive filesystem walk on an HTTP request path.

## Boundaries

- Do not change DB-declared upload-byte accounting, quota behavior, or its
  existing metric identity.
- Do not report allocated blocks, filesystem quota, deduplicated physical
  storage, or an arbitrary parent storage root.
- Do not add automatic cleanup, symlink handling, repairs, or operator-facing
  management UI.
