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

Removing all metadata indiscriminately can change orientation or color and break
animation; arbitrary color profiles can themselves carry descriptive data.
Decode/re-encode can degrade the photo. These are real fidelity/privacy
tradeoffs, not reasons to return a successful upload with its identifying
metadata intact.

## Decision

Mandatory shared ingestion sanitizes new JPEG, PNG/APNG, GIF, WebP and HEIC/HEIF
uploads before public placement and Media Record creation. Identification uses
bytes, not caller labels, and determines the stored/served Content-Type of
covered images even when labels disagree; canonical filenames remain unchanged.
Non-image Content-Type behavior remains unchanged. GPS, device
identifiers/descriptions, timestamps, author/copyright descriptions, comments,
descriptive profile fields and embedded previews are removed. Only validated
presentation data needed for orientation, color/HDR, transparency and animation
remains. Optional device-editing extras other than the always-removed embedded
thumbnails/previews may be discarded; required rendering data and displayed
frames may not.

Source formats and compressed image payloads are preserved without lossy
re-encoding. ICC descriptive fields are scrubbed without changing their color
transforms. Covered input that cannot meet this contract is rejected, not served
unsanitized or silently degraded. Resource execution is bounded. The exact
implementation, supported variants and limits require fixture-backed proof;
generic metadata-editor success is not the privacy guarantee.

SVG remains accepted unchanged and outside this guarantee, as does non-image
Media. The policy does not promise removal of visible PII, PII in filenames, or
arbitrary steganography. Neither User nor operator has a metadata opt-out.

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
feature. Fidelity, metadata removal, failure cleanup, resource limits and
idempotence require independent output and consumer tests on both backends.
HEIC/HEIF acceptance needs successful representative device-photo proofs, not
blanket rejection or an assumed capability from a library format table.

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
