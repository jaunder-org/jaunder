# ADR-DRAFT: Emacs pull stores durable local media copies

- Status: proposed
- Date: 2026-08-27
- Issue: [#80](https://github.com/jaunder-org/jaunder/issues/80)

## Context

The Emacs Protocol Client publishes local media without rewriting the author's
buffer: it uploads bytes, harvests the server-assigned `<content src>`, and
substitutes that URL only in the AtomPub Entry. The inverse pull path therefore
receives native Post source containing server media URLs. It currently preserves
those URLs, so a pulled Post is not previewable offline.

[ADR-0024](../0024-server-side-org-canonicalization.md) requires previewable
local links while keeping the server's Post body canonical and metadata-free.
[ADR-0045](../0045-emacs-media-content-src.md) makes the server's binary URL
authoritative, while [ADR-0140](../0140-strict-media-address-extraction.md) and
[ADR-0084](../0084-media-filename-encoded-canonical.md) define the canonical
public route and percent-encoded filename.
[ADR-0154](../0154-media-reference-live-ownership.md) defines
`X-Jaunder-Instance` as exactly one canonical UUID-valued response header.
AtomPub Member requests require an App Password, but public media bytes do not.

Downloading arbitrary author URLs would turn pull into a general network fetcher
and could leak credentials. Treating downloaded bytes as an evictable cache
would break local links because reconciliation does not repair media for matched
Posts. Installing the Post before all media succeeds would expose a partially
offline result that retry cannot classify as server-only.

## Decision

For a server-only pull, the Emacs client localizes link destinations in Org,
Markdown, and HTML only when they identify canonical public media on the active
Jaunder origin and contain neither user information nor a query. Other URLs and
non-link text remain source-faithful.

The authenticated Member response and every media response must carry exactly
one canonical `X-Jaunder-Instance` UUID, and the values must match. Each media
GET is anonymous and direct; redirects are not followed. A media response is
accepted only on `200` with a strong `"sha256-<64-lowercase-hex>"` ETag and
bytes whose computed SHA-256 equals both the ETag's hash and canonical URL hash.
The App Password is never sent to a media URL.

Verified bytes become a **Local Media Copy** under the configured root at
`local-media/<sha256>/<decoded-filename>`. The server's canonical
percent-encoded filename is decoded once and validated as a safe local leaf;
native link targets retain that canonical encoding exactly once so they resolve
to the decoded file. Local Media Copies are durable managed content with no
eviction promise. The configured root is trusted, author-owned local state: path
creation and immediate mutations reject symlinks, non-directory components, and
overwrites. The client cannot prevent malicious concurrent replacement after its
final check because Emacs Lisp has no dirfd-anchored mutation; that race is
explicitly out of scope. Existing copies are reused only after their bytes hash
to the expected digest; mismatches fail and are never overwritten.

A pull stages and verifies every distinct media object, installs Local Media
Copies without overwrite, rewrites native-format link destinations to relative
local paths, and atomically installs the Post file last. Failure leaves the Post
absent, so rerunning reconciliation is the retry mechanism. Copies installed
before any ordinary late failure are retained, just as after a process crash,
and are safe to reuse; the client does not promise rollback, a multi-file
transaction, or orphan collection.

## Consequences

Pulled Org, Markdown, and HTML Posts are offline-previewable. A pulled Org Post
republishes through the existing local media upload path, which server-side
content hashing deduplicates. Markdown and HTML republish remain the existing
Org-only publish path's limitation and are not expanded by this decision. The
three pull formats need separate syntax-aware candidate and substitution logic
behind one pull-localization policy.

The active Jaunder origin, public media route, canonical instance identity,
strong hash ETag, and canonical filename become the trust chain for Local Media
Copies. A proxy that strips, duplicates, malforms, or changes those signals
makes pull fail closed.

`local-media/` is user-visible durable state inside each configured root.
Backup, synchronization, and manual directory moves must carry Local Media
Copies with the Post files. No automatic cleanup exists; verified unreferenced
copies may accumulate after ordinary failures, crashes, or later Post deletion.

This decision does not broaden reconciliation beyond server-only pull, download
external author media, redesign reconcile reports, or add server behavior.
