# Issue #25 — Broaden Emacs Publish Media Upload — Implementation Outline

> Execute with `jaunder-iterate`, delegating slices with `jaunder-dispatch`
> where useful. This outline exists because the approved spec changes a public
> AtomPub discovery contract and crosses Emacs/server/docs/tests with a
> service-document accept-range type split.

## Scope

In:

- Emacs publish media qualification broadens from image-extension links to
  local-path `file:`/`attachment:` body links.
- Emacs content-type selection uses the shared media extension table and falls
  back to `application/octet-stream`.
- Emacs preflight rejects missing, unreadable, and non-regular local targets
  before any upload.
- AtomPub service document advertises the media collection as accepting the
  media range `*/*`.
- Docs and tests cover the changed Emacs and AtomPub discovery behavior.

Out:

- Markdown/HTML authoring-buffer converters.
- Pull/download/localize media behavior.
- Inline/data URI uploads.
- Media Library or other UI work.
- Server media storage, URL layout, filename canonicalization, or publication
  lifecycle changes.
- New blocking policy for untracked local attachments.

## Task outline

- [x] Task 1: Introduce AtomPub accept-range discovery model.
  - Contract: Service-document collection accept values represent AtomPub
    `app:accept` media ranges, not uploaded-media `ContentType` values. Keep
    concrete upload content types on `common::media::ContentType`; introduce or
    rename a service-document-only value type/helper for
    `application/atom+xml;type=entry` and `*/*` so wildcard accept ranges do not
    pass through `ContentType`'s concrete media invariant.
  - Verification: focused Rust tests for service-document rendering and existing
    AtomPub media upload non-image behavior.

- [x] Task 2: Broaden Emacs media candidate qualification and preflight.
  - Contract: `jaunder--media-link-p` or its replacement tests local-path link
    candidacy only; filesystem checks belong to preflight.
    `jaunder--media-preflight` rejects missing, unreadable, and non-regular
    paths in one error before upload. `file:` and `attachment:` resolution
    remains live-buffer/body-scoped and position-ordered.
  - Verification: pure ERT cases for non-image `file:` and `attachment:`
    collection, non-local pass-through, source-buffer non-mutation,
    description-preserving substitution, per-resolved-file upload deduplication,
    and preflight abort before upload for missing/unreadable/non-regular
    targets.

- [x] Task 3: Align Emacs content-type mapping with shared media table.
  - Contract: Emacs owns a deterministic extension table matching
    `common::media::detect_content_type`: `jpg`/`jpeg`, `png`, `gif`, `webp`,
    `svg`, `mp3`, `ogg`/`oga`, `flac`, `wav`, `mp4`, `webm`, `pdf`; fallback is
    `application/octet-stream`. No mailcap, process calls, or server sniffing.
  - Verification: pure ERT table coverage for every mapped extension plus
    unknown and extensionless names.

- [x] Task 4: Prove live publish/upload behavior.
  - Contract: The publish pipeline order remains ADR-0047: validate → media
    localization/upload → entry send → `JAUNDER_ID`-first write-back → rename.
    Media URL harvesting remains ADR-0045 `<content src>`. The authoring buffer
    is never rewritten.
  - Verification: live Emacs integration uploads a non-image local attachment
    through the real AtomPub Media Collection and publishes a Post whose sent
    body uses the harvested absolute media URL.

- [x] Task 5: Update architecture and historical contract references.
  - Contract: `docs/ARCHITECTURE.md` must replace the image-only Emacs media
    qualification statement with the local-file rule, the explicit deterministic
    content-type table/fallback, and the retained safe-to-resume ordering. Do
    not rewrite archived historical specs except if a current generated/checking
    doc requires a pointer.
  - Verification: doc/link gates through the final precommit path.

## Risk checks

- `*/*` is an AtomPub `app:accept` media range, not an uploaded object
  `Content-Type`; no wildcard value may be stored or parsed as
  `common::media::ContentType`.
- Eligibility and preflight stay separate: missing/non-regular local-path
  candidates must fail before upload, not pass through as ordinary links.
- Position-based substitution remains collision-safe for org-attach links
  resolving differently under identical raw link text.
- Unknown extension fallback is deterministic `application/octet-stream`; no
  host-dependent MIME database or external command enters the publish path.
- The server stays authoritative for media URL layout via response
  `<content src>`.
- Focused verification must include pure ERT, live Emacs integration, Rust
  service-document tests, and `cargo xtask precommit` before commit.
