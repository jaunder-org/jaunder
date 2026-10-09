# ADR-DRAFT: Sanitize image metadata before minting uploaded Media identities

- Status: proposed
- Date: 2026-10-07
- Issue: [#1702](https://github.com/jaunder-org/jaunder/issues/1702)

## Context

Device photos can expose precise locations and identifying descriptions through
embedded metadata even when their visible content is innocuous. Web and AtomPub
uploads share one Media ingestion boundary, while public retrieval serves stored
bytes directly. A private derivative beside a publicly reachable original would
not close the disclosure.

Media URLs and strong ETags identify exact bytes. The Emacs Protocol Client
harvests the server-assigned URL and verifies downloaded Local Media Copies
against that identity ([ADR-0084](../0084-media-filename-encoded-canonical.md),
[ADR-0160](../0160-emacs-pulled-media-local-copies.md)). Rewriting existing
bytes beneath a hash URL would violate that trust chain and immutable-cache
semantics.

Indiscriminate metadata deletion can remove orientation or a color profile;
decode/re-encode can degrade the photo. The practical response is conventional
metadata editing that retains rendering information, backed by focused output
checks, not a custom image-codec validator.

The user narrowed the proposed scope on 2026-10-09: prevent inadvertent
disclosure of ordinary photo metadata such as GPS. Forensic hidden-data removal,
custom ICC scrubbing, compressed-stream validation and normative-syntax research
are not part of this feature.

## Decision

Mandatory shared ingestion sanitizes new JPEG, PNG/APNG, GIF, WebP and HEIC/HEIF
uploads before public placement and Media Record creation. Identification uses
bytes, not caller labels, and determines the stored/served Content-Type of
covered images even when labels disagree; canonical filenames remain unchanged.
Non-image Content-Type behavior remains unchanged. Established metadata tooling
removes ordinary GPS/location, camera/device identifiers, capture timestamps,
author descriptions, comments, EXIF/XMP/IPTC descriptive fields and metadata
thumbnails/previews it supports. Rendering information needed for orientation,
color/HDR, transparency and animation remains.

Source formats and image/frame data are preserved without lossy re-encoding.
Color profiles remain intact, including their descriptive text; bespoke ICC
canonicalization is outside this ordinary-metadata policy. Processing or
post-edit metadata-check failures reject the upload, never publish the original
as a fallback. Runtime, input/output sizes, concurrency and child lifetime have
ordinary application limits. A small fixture set checks actual removal and
preserved presentation; tool exit status alone is not proof. No exhaustive
codec/conformance or ignored-byte ownership proof is required.

SVG remains accepted unchanged and outside this guarantee, as does non-image
Media. The policy does not promise removal of visible PII, PII in filenames,
descriptive text in retained rendering profiles, arbitrary steganography or
forensically recoverable ignored/padding bytes. Neither User nor operator has a
metadata opt-out.

Stored-byte hash, URL, ETag, deduplication, size and quota describe the
sanitized output. Sanitization is deterministic and byte-idempotent, preserving
the upload/pull/republish cycle. Local author originals remain untouched;
unsanitized server working input stays private and temporary. Existing
identities and Local Media Copies are not rewritten automatically.

Sensitive existing production Media is remediated with new sanitized identities,
current Post reference updates and authorized retirement of originals. Existing
per-user ownership, global-safety and reclaim guards remain authoritative
([ADR-0183](../0183-per-user-media-records-from-local-post-references.md)). The
explicit owner-history override may leave broken retained-history references; it
does not authorize rewriting immutable revisions or overriding another User's
ownership. Production execution needs separate approval.

## Consequences

Sanitization becomes a cross-protocol privacy boundary, not an optional UI
feature. Fidelity, ordinary metadata removal, failure cleanup, execution limits
and reupload identity receive focused output and consumer tests on both
backends. HEIC/HEIF must actually work in the fixture check rather than be
rejected wholesale or assumed from a format table. Native-device samples are
useful additional coverage, not a prerequisite to integration or an excuse for
another standards-research project.

We reject public originals plus sanitized derivatives, metadata opt-outs, lossy
format conversion and in-place historical rewrites. Remaining optional editing
capabilities are intentionally subordinate to privacy and presentation. The
chosen tool/runtime and its maintenance cost are implementation decisions
requiring pinned packaging and license review before delivery.

A one-off procedure documents shared ownership, historical link breakage, stale
local upload sources, immutable caches and backups. Retiring current binaries
cannot recall earlier downloads or erase sensitive copies in retained backups.

No new domain entity or glossary term is introduced: Media Records, Media Upload
Capability and Local Media Copies retain their meanings.
