# Spec — #692/#1046: typed media boundaries and strict serve addresses

- Issues: [#692](https://github.com/jaunder-org/jaunder/issues/692),
  [#1046](https://github.com/jaunder-org/jaunder/issues/1046)
- Milestone: Domain-value type safety (newtypes)
- Governing decisions:
  [ADR-0063](../../adr/0063-domain-value-newtype-convention.md),
  [ADR-0080](../../adr/0080-media-path-naming-correspondence.md),
  [ADR-0084](../../adr/0084-media-filename-encoded-canonical.md),
  [ADR-0101](../../adr/0101-infallible-kind-is-invariant-first.md), and
  [strict media-address extraction](../../adr/0140-strict-media-address-extraction.md)
- Date: 2026-08-15

## Problem

`ContentType`, `Filename`, and `MediaSource` already express the media path's
value invariants, but several post-validation APIs flatten them back to `&str`.
The most important remaining chain accepts raw content types through
`MediaManager::upload` and `upload_bytes`, even when the HTTP caller has enough
information to validate once. `detect_content_type` and `content_disposition`
accept the domain values whose semantics they use; `should_inline` consumes only
the content type's borrowed media-type spelling.

The issue inventory is partly stale. #675 and #720 already typed `media_path`
and `media_url` as `MediaSource`/`ContentHash`/`Filename`; ADR-0089 removed the
old Atom XML accumulator; and production `MediaLinkEntry.content_type` is
already `ContentType`. Those are completed prerequisites, not work to repeat.

One stale site exposes a separate decision. The public media serve route uses
`SoftPath<T>` so malformed segments reach `serve_handler` as `None` and
return 404. The project owner has reversed #504's media-route policy: malformed
media addresses are bad requests, while only syntactically valid but absent
resources are not found. Because strict extraction and downstream value
propagation touch the same route signature, filename conversion, tests, ADR-0084
consequence, and static gate, #692 and #1046 are one coupled delivery and one
PR.

The route also emits `jaunder.media.served{result=…}` for HTTP outcomes. The
front proxy is the authoritative HTTP request/outcome telemetry source. A
partial in-app counter—especially one strict extractor failures bypass—would be
misleading, so the serve-outcome metric is removed. Upload-domain metrics remain
because a proxy cannot observe stored/deduplicated/quota semantics or stored
byte counts.

## Decisions

### 1. Validate `ContentType` at each external HTTP intake

The multipart and AtomPub HTTP boundaries parse or construct `ContentType`
before calling storage:

- Multipart upload converts its parsed `mime::Mime`, when present, into an owned
  `ContentType`. Absence remains `None` so storage may detect from the filename.
- AtomPub media POST distinguishes absence from invalid presence. An absent
  header becomes the canonical `application/octet-stream` value. A present
  header that cannot become `ContentType` returns HTTP 400; it never silently
  defaults.

`MediaManager::upload` consumes `Option<ContentType>`.
`MediaManager::upload_bytes` consumes `ContentType`. Their inner helpers carry
the same owned typed values and never re-parse or flatten them. Ownership is
intentional: the selected type lands in `UploadMetadata`, so borrowing and
cloning it would add work with no benefit.

The typed storage resolver is infallible: a supplied `ContentType` is already
valid; absence calls filename-based detection. The former
`get_content_type(Option<&str>, &str) -> Result<ContentType>` validation door is
removed rather than retained beside the HTTP doors.

### 2. Helpers accept the values whose semantics they use

- `should_inline` accepts `&str`: content disposition passes its already-validated
  `ContentType` as a borrowed media-type spelling, which is the sole value the
  allowlist compares.
- `detect_content_type` accepts `&Filename` and performs the single
  `Filename::decoded()` display-view conversion internally before examining the
  extension.
- `content_disposition` accepts `(&ContentType, &Filename)` and performs
  `Filename::decoded()` internally before building the fallback and RFC 5987
  forms.

These APIs prevent callers from passing an encoded filename where a decoded
display spelling is required. The conversions remain allocation-conscious:
`ContentType` is borrowed as `&str` only at the inline allowlist, and
`Filename::decoded()` remains a `Cow` consumed in the helper that needs it.

`media_path` and `media_url` remain unchanged: both already accept
`&MediaSource`, `&ContentHash`, and `&Filename`, and `Filename` remains the
canonical encoded path segment under ADR-0084.

### 3. One strict extracted media address owns every route invariant

The public route keeps its existing five-segment URL shape:

`/media/{source}/{p1}/{p2}/{hash}/{filename}`

Axum extracts it through one private validated address type. Its boundary
conversion:

1. parses `MediaSource`, `ContentHash`, and `ProfferedFilename` from the
   percent-decoded route segments;
2. verifies `p1 == hash[0..2]` and `p2 == hash[2..4]` only after `ContentHash`
   has proved the canonical 64-byte lowercase-hex invariant;
3. converts `ProfferedFilename` infallibly into canonical `Filename`; and
4. exposes only `{ source: MediaSource, hash: ContentHash, filename: Filename }`
   to `serve_handler` and lower helpers.

Malformed source, hash, filename, or mismatched prefixes fail Axum extraction
with HTTP 400. No `SoftPath`, optional parse result, raw prefix, or revalidation
branch reaches `serve_handler`. A fully valid address whose on-disk file is
absent still returns HTTP 404. A present file with no database row retains the
existing 200 fallback, deriving `ContentType` from its typed filename.

This supersedes #504 only for the public media route. Projector and Syndication
Feed routes retain their established `SoftPath` behavior.

### 4. Boundary behavior is tested at the boundary

Malformed route strings are not injected into post-extraction unit helpers.
Router-level requests exercise the real Axum URL decoding and extraction
boundary for:

- invalid `MediaSource`;
- short and non-hex `ContentHash`;
- unsafe/noncanonical decoded filename;
- first-prefix mismatch;
- second-prefix mismatch; and
- valid-but-absent media.

Post-extraction unit tests construct only valid typed addresses. The
five-adjacent-`&str` `params` helper is deleted.

Exact `Content-Disposition` tests construct typed `Filename` fixtures and
continue pinning ordinary names, spaces, quotes, non-ASCII, control characters,
RFC 5987 encoding, no double-encoding, and inline-versus-attachment selection.
Control characters are a valid canonical encoded `Filename` state; the typed
test proves their decoded display form is stripped from the ASCII fallback and
the resulting header remains valid.

### 5. Remove HTTP serve-outcome instrumentation, keep domain upload metrics

Delete `ServeResult`, the `media_served` counter/instrument, its public emission
function, `serve_result`, and their tests. `serve_handler` returns
`serve_response` directly.

Keep `jaunder.media.uploads`, `jaunder.media.upload_bytes`, and their bounded
domain outcomes unchanged. They describe application operations the proxy cannot
infer.

### 6. Record and enforce the changed route decision

A new ADR draft records strict media-address extraction, the 400/404
distinction, the decision to leave other soft routes unchanged, and proxy
ownership of HTTP outcome counts. It supersedes #504's media-specific policy but
does not supersede ADR-0084's canonical-filename decision.

ADR-0084 receives a dated addendum pointing its obsolete
`SoftPath<ProfferedFilename>` consequence at the new decision. The
`proffered-filename-position` gate is changed to permit the new validated Axum
extraction type while continuing to reject `ProfferedFilename` in DTOs, storage,
returns, server-function parameters, and ordinary helper parameters.
`docs/ARCHITECTURE.md` projects the strict media address and telemetry
ownership, citing the draft path so ADR promotion can rewrite it at ship.

`CONTEXT.md` does not change: this is an HTTP/type boundary decision, not new
ubiquitous language. `CONTRIBUTING.md` and user-facing design documentation do
not change.

## Reviewed sibling values

- `common::media::media_path` and `media_url`: already fully typed after
  #675/#720; unchanged.
- `common::atompub::MediaLinkEntry.content_type`: already `ContentType`; the
  external `atom_syndication` serializer receives text only at its library
  boundary.
- The former `common::atompub::entry::Acc.content_type`: no longer exists after
  ADR-0089; no raw accumulator remains to migrate.
- Multipart and HTTP header values: intentionally raw only until their endpoint
  validation door.
- Projector and Syndication Feed `SoftPath` uses: separate public-route
  behavior, explicitly unchanged.
- `ContentType::from_trusted`: remains crate-private for fixed or otherwise
  proven producer values; this work adds no trust door.

No storage schema, stored bytes, media URL shape, response DTO shape, serialized
content type, canonical filename spelling, or successful response header
changes.

## Acceptance criteria

1. Every post-intake manager signature carries `ContentType`:
   `upload`/`upload_inner` consume `Option<ContentType>`, and
   `upload_bytes`/`upload_bytes_inner` consume `ContentType`. No production call
   passes a raw content-type string into storage.
2. Multipart upload parses its optional MIME value before storage. AtomPub media
   POST defaults only an absent `Content-Type` to `application/octet-stream`.
   Router tests require 400 for both a present ASCII value rejected by
   `ContentType` and a present opaque `HeaderValue` whose `to_str()` conversion
   fails.
3. `detect_content_type` and `content_disposition` accept the applicable domain
   types; `should_inline` compares the borrowed spelling from `ContentType`.
   `detect_content_type` and `content_disposition` own the only display decode
   they require; callers cannot swap encoded filename, source, or hash values
   through same-typed parameters.
4. The public media route contains no `SoftPath` field or parameter. Its
   extraction type proves source/hash/filename validity and both hash-prefix
   relationships before `serve_handler` runs. Router tests assert malformed
   source, hash, filename, and either prefix return 400; a valid address with a
   missing file returns 404; and a present file without a database row retains
   its extension-derived 200 response.
5. A dual-backend router test asserts the complete successful serve contract:
   exact `Content-Type`, `Content-Disposition`, ETag, cache-control, body bytes,
   encoded URL filename segment, and decoded disposition filename. Focused exact
   helper tests additionally cover inline/attachment selection, RFC 5987
   encoding, quotes, non-ASCII, control characters, and no double-encoding.
6. `jaunder.media.served` and `ServeResult` no longer exist. Upload outcome and
   byte metrics remain and their existing tests pass.
7. The raw five-`&str` media route fixture is gone. Invalid route values are
   constructed only as URL text above Axum extraction; post-extraction helpers
   accept valid typed values.
8. The new ADR draft, ADR-0084 addendum, architecture projection, and
   `proffered-filename-position` gate describe and enforce the same strict
   extraction boundary. The gate's tests accept only the intended extractor
   shape and reject every prohibited `ProfferedFilename` position.
9. No database migration, wire-format change, compatibility shim, deprecated
   alias, duplicate validation door, or new domain vocabulary is introduced.
10. Focused common/storage/server/web tests pass for both SQLite and PostgreSQL
    where the affected suite is backend-parameterized; the repository
    `cargo xtask check` gate passes at each task commit, and
    `cargo xtask validate` passes before shipping.

## Verification seams

- Common media unit tests: typed detection, inline policy, exact filename/header
  representation.
- Storage media-manager tests: supplied typed content type, absent detection,
  streaming upload, in-memory upload, size/quota behavior.
- Web multipart integration tests: typed supplied and detected content types.
- AtomPub router tests: absent default, ASCII-invalid and opaque-invalid present
  headers returning 400, and successful media upload.
- Media router integration tests: strict malformed-address 400 matrix,
  missing-file 404, no-database-row fallback 200, and a dual-backend exact
  successful response contract covering four headers, body bytes, and both
  encoded URL and decoded disposition filename spellings.
- Host metrics tests: serve counter removed; upload instruments unchanged.
- xtask unit tests: accepted strict extractor population and prohibited position
  matrix.
