# Issue #619: Eliminate the ceremonial post read row

## Outcome

Post read queries decode directly into the public `PostRecord` returned by the
storage API. The one remaining aggregate conversion—the JSON `tags` column—stays
localized at the SQL decoding boundary, while callers no longer map an
intermediate `PostRow` into the same record.

## Load-bearing decisions

- Eliminate `PostRow` and its forwarding conversion. `PostRecord` is a
  storage-owned public result record, so its handwritten SQL row decoder may own
  the exceptional aggregate projection.
- Decode every named post column into its existing domain type. Decode `tags` as
  text and parse it through the existing fallible tag JSON parser; malformed
  aggregate data remains an SQL decode error.
- Follow ADR-0123: the reviewed `rendered_html` column decodes directly into
  `RenderedHtml`, with its required local review marker. Do not restore the
  superseded write-only bridge premise from the original issue text, and do not
  introduce `from_trusted` on the storage read path.
- Keep query column names and `PostRecord`'s public field shape unchanged so
  generic, SQLite, and PostgreSQL reads retain the same contract.

## Acceptance

- Every post read query, including the hybrid-window and dialect-specific update
  readbacks, returns `PostRecord` directly; no production `PostRow` mapping
  remains.
- The post decoder returns the same domain record for valid rows and reports
  malformed tag JSON or invalid tag values as `sqlx::Error::Decode`.
- Existing direct-decode review protection covers `rendered_html` in its new
  `FromRow` location, as required by ADR-0123.
- Both storage backends retain the current post read and malformed-column test
  coverage.

## Boundaries

- No wire, schema, migration, `PostRecord` API, HTML sanitization, or
  `RenderedHtml` SQL-bridge policy change.
- No unrelated consolidation of other query-specific row types or conversions.
