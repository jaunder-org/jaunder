# ADR-DRAFT: Prepare image uploads privately before assigning stored identity

- Status: proposed
- Date: 2026-10-19
- Issue: [#1702](https://github.com/jaunder-org/jaunder/issues/1702)

## Context

An image can carry ordinary personal/location metadata independently of its
encoded picture. Removing it changes the stored bytes, so hashing, quota,
deduplication, public placement and Media Record creation must describe the
prepared result, not an author original. Web and AtomPub converge on the same
MediaManager; a transport-specific option would create a policy bypass.

A native job in a blocking thread cannot be stopped by dropping an async waiter.
Cancellation can also arrive after filesystem placement but before registration
or commit acknowledgement. Private input, process ownership and finalization
therefore need lifetimes independent of a request future, without extending a
database WriteScope across native or filesystem work.

## Decision

Use upstream `exiftool-rs` **v0.13.0**, following the user's explicit release
switch decision, from `https://github.com/Le-Syl21/exiftool-rs` at the tag's
immutable commit `863f9b8eb47f995eddef657e006e1a22b3a28334`, with its empty
default feature set and no GUI features. Normal Cargo/Nix resolution carries
this GitHub release source across all three workspace locks; this is not a claim
of crates.io publication or a registry checksum. The release directly follows
the previously approved unreleased PNG fix
`b60a78446730bb7d25fdb367df4e1a7d9cb3c461` (package version 0.12.0), which
followed release commit `c29c4b7a794de4b8d48fb3cabb1c22e10abf72ce`. The release
changes only version/changelog/lock metadata, not native source or tests. Future
source/release switches require a separate decision. The shared
[MediaManager](../../../storage/src/media_manager.rs) requires an explicitly
injected upload preparer at composition; streamed Web and in-memory AtomPub
uploads cannot select an unprepared path. Composition roots create it through
`StorageFactory::media_manager`, which mints its three storage ports and
WriteScope and requires explicit content locks, serving identity, ownership
resolver and preparer. One private initialization seam consumes all eight
required dependencies into individual manager fields; neither a factory nor a
parts bundle is retained or used for lazy lookup. The existing test-support
boundary exposes explicit required parts only for test composition with
overridden ports/scopes. This follows
[constructor injection](../0016-dependency-injection-and-appstate.md).

Detect covered image formats from controlled extensionless private bytes rather
than an author's filename or supplied MIME. Ordinary EXIF/XMP/IPTC personal,
location, device, date, description, comment and supported preview metadata are
removed. PNG group removal clears ordinary text (including language-coded text)
and tIME; in the same native write, explicitly restore only Gamma and sRGB
intent present in the BEFORE read. Fresh verification preserves the
library-supported numeric gamma representation and exact named sRGB intent,
alongside the existing ICC/orientation/dimension checks. Unrestorable rendering
controls reject through the typed processing/verification failure path, never an
original fallback. Other EXIF/XMP removal stays selective. Encoded image/frame
content, source format, orientation, dimensions, intact ICC/color profiles, HDR
signaling, animation and transparency must be preserved: no lossy re-encoding,
resizing, automatic HEIF conversion or indiscriminate XMP deletion. The
authoritative selection and post-read policy live in the
[native adapter](../../../host/src/image_privacy/native.rs), not a duplicated
tag table in this decision. SVG/non-image behavior is outside this raster
policy. Malformed/unrecognized image-labelled raster input fails closed.

Receive privately, enforce the positive input/output Media size policy, edit and
perform supported ordinary post-edit reads before assigning authoritative
hash/URL/ETag/size/quota, public placement or a Media Record. Already-absent
edits and `Ok(0)` are legitimate, never privacy certificates. Legacy
zero-length-as-empty compatibility is accepted without forensic recovery of
ignored bytes; retained ICC profile text, pixels and filenames are not redaction
targets. Never fall back to publishing an original. Successful ordinary removal
for **every** covered format remains an acceptance condition; editor errors,
residual metadata or failed checks reject, and blanket-family rejection is not
delivery. A pinned editor limitation does not authorize an exclusion, fork,
extra editor or weaker check.

The [process owner](../../../host/src/image_privacy/process.rs) admits exactly
TWO shared active attempts immediately or returns HTTP 503 Busy, without a
waiting queue, including non-image/SVG attempts. It owns 0700 attempt
directories and 0600 files under `media/tmp`, with input/output on the placement
filesystem. Launch only the SAME deployed Jaunder executable's private worker,
dispatched before ordinary server/database/logging startup. No shell, separate
installed worker, runtime tool lookup, ExifTool/Perl or network acquisition
participates in production preparation.

ONE 30-second deadline begins before child launch and includes processing,
verification, bounded status and exit; the worker response is limited to 128 KiB
before decoding. The owner kills when necessary and WAITs/reaps before private
cleanup or slot release on cancellation, timeout, crash or failure. Success also
requires child exit acknowledgement. Bounded
[typed status/errors](../../../host/src/image_privacy/wire.rs) retain underlying
processing/I/O causes without private metadata or path-bearing diagnostics.

After preparation, tracked MediaManager finalization owns the attempt, content
locks, placement, short registration WriteScope, cleanup and result handback
even when its caller disappears. Only fresh validated public files become 0644;
existing deduplicated files and author originals are not modified. Confirmed
operation failure may clean an owned fresh placement; committed or
CommitIndeterminate results must not be treated as rollback-confirmed failure
and unlinked. This preserves
[structural mutation outcomes](../0164-structural-write-scopes-and-mutation-outcomes.md).
The [serve composition root](../../../server/src/commands/lifecycle.rs) refuses
new work and drains private owners and finalizers before releasing its runtime
lock.

## Consequences

New stored identities describe sanitized bytes; existing immutable objects and
Local Media Copies are not rewritten. Real transport/public-GET tests,
independent TEST-only metadata/payload observers and real-worker lifecycle
controls establish behavior; edit counts or helper-only success do not. Test
oracles are not additional production editors.

Current-reference replacement is separate from historical-byte retirement.
Follow the
[operator preparation/rehearsal procedure](../../media-replacement.md) without
bypassing owner history, independent Media Records or global safety. Retained
revisions, caches, backups and remote copies remain distinct from a record
deletion. Dependency capability, licensing and production `x86_64-linux`
packaging remain acceptance obligations. ARM Linux is not a delivery
requirement; Apple Silicon macOS targets only the development environment,
checked periodically from committed changes rather than as a per-WIP production
package/worker or merge prerequisite. This decision grants no production
remediation or retention waiver. Update this decision's architecture projection
when its preparation, ownership or identity boundaries change.
