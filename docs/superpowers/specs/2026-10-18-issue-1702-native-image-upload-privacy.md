# #1702 — Mandatory native image-upload metadata privacy

Status: APPROVED — JAU-1702-APP-SPEC-001. Issue:
https://github.com/jaunder-org/jaunder/issues/1702

## Outcome

New Web and AtomPub image uploads remove ordinary location and personal metadata
before publication while preserving image fidelity. Stored identity describes
the sanitized bytes; author originals and existing immutable Media remain
unchanged.

## Load-bearing decisions

- One mandatory shared ingress policy covers byte-detected JPEG, PNG/APNG, GIF,
  WebP and HEIC/HEIF. Demonstrate successful removal for every covered format;
  blanket format rejection is not delivery. No user/operator opt-out.
- Misleading filenames or supplied MIME cannot bypass preparation. Detected
  image format governs stored/served MIME while canonical filename/path behavior
  stays unchanged. Malformed or unrecognized image-labelled raster input fails
  closed. SVG and non-image Media remain unchanged, including their current MIME
  behavior.
- Remove ordinary EXIF/XMP/IPTC location, camera/device identifiers, capture
  dates, authors/descriptions/comments and supported metadata
  thumbnails/previews. Preserve rendering fields rather than indiscriminately
  deleting all metadata.
- Preserve source format, encoded image/frame content, orientation/displayed
  dimensions, intact ICC/color profiles, representative HDR signaling, animation
  and transparency. No lossy re-encoding, resizing or automatic HEIF conversion.
- Following the user's explicit release switch decision, use upstream
  `exiftool-rs` v0.13.0 from `https://github.com/Le-Syl21/exiftool-rs`, pinned
  to the release tag's immutable commit
  `863f9b8eb47f995eddef657e006e1a22b3a28334` through normal Cargo/Nix
  resolution, with its empty default feature set and no GUI features. This
  GitHub release directly follows the previously approved unreleased PNG fix
  `b60a78446730bb7d25fdb367df4e1a7d9cb3c461` (package version 0.12.0), itself
  following release commit `c29c4b7a794de4b8d48fb3cabb1c22e10abf72ce`; only
  version/changelog/lock metadata changed. The historical registry checksum
  `84c238979b31bd4e0fb6d4d56c0128f4c71cf1d751872b40474a7e865a549733` does not
  identify this Git release, and no crates.io publication is claimed. Source
  identity is not execution proof; future automatic release switches remain
  unauthorized.
- Accept #21's zero-length-as-empty compatibility, not ignored legacy-empty byte
  recovery. Already-absent deletion `Ok(0)` is legitimate; no request count
  proves privacy. Perform supported post-edit ordinary-metadata reads; failed
  checks or editor errors reject the upload.
- Preserve historical failures as evidence, not forensic acceptance criteria. No
  custom codec grammar/ICC-text scrubbing; retained profile text is out of
  scope.

- Receive and prepare privately before public placement, Media Record creation
  or authoritative hash/dedup/URL/ETag/size/quota accounting. No original
  fallback. Enforce current positive Media maximum size on received AND
  sanitized bytes; quota describes stored bytes. Same input/filename and
  sanitized reupload retain truthful deterministic/idempotent identity without a
  custom canonicalizer.
- Owned attempt spools/files stay private (files 0600); output stays in its
  input spool for same-filesystem placement. Only fresh validated public
  placements become 0644; never mutate author originals or existing objects.
- Preserve explicit composition-root dependency injection and an upload-ready
  MediaManager. Both transport paths must cross the same preparation seam. Keep
  content locks, short WriteScopes and CommitIndeterminate honesty intact;
  native processing and filesystem preparation stay outside database write
  scopes.
- Preserve TWO shared active jobs, ONE 30-second processing deadline and a 128
  KiB worker-response bound. Input/output follow current positive Media size
  policy. Launch the native worker from the SAME deployed executable: no shell,
  separate installed executable, runtime tool search, network acquisition or
  ExifTool/Perl.
- The execution owner kills/reaps canceled, timed-out, crashed or failed workers
  before releasing permits or private files. Clean owned private
  inputs/candidates on success, failure and cancellation; no leaked private
  input on rejection. An async timeout around spawn_blocking is not enforceable
  cancellation.
- Reject unusable worker responses, resource breaches, editor/write/check
  failures without public original, Media Record or quota charge. Preserve
  underlying typed processing/I/O/storage causes, not stage-only masking;
  diagnostics contain no PII. The short outline must specify
  admission/deadline/cleanup behavior and its proof.

- AtomPub returns the authoritative sanitized URL. Preserve Emacs original
  files, verified pull/reuse/republish and durable Local Media Copy
  hash/strong-ETag and serving-instance trust. Existing URLs, ETags and Local
  Media Copies do not change.
- Deliver a reviewed operator procedure for at least two sensitive existing
  images: inventory exact identities/current Posts/shared ownership, create
  replacements with new identities and update current references through
  supported writes.
- Disposable rehearsal covers owner-history refusal/record override, independent
  sharing and legacy/global-safety refusal; report actual old-byte availability.
  Record deletion is not retirement. Explain stale local sources/copies,
  immutable caches/backups and unrecallable remote copies. Retirement/script is
  #1714; no production writes.

## Acceptance

1. Small owned/non-personal fixtures prove every covered format's actual
   ordinary removal and encoded-content/rendering preservation, including
   orientation, ICC, representative HDR, applicable animation/transparency,
   misleading labels and ordinary invalid input. Separate TEST-only ExifTool
   reads independently inspect outputs; editor success/count alone is
   insufficient. Camera originals are optional.
2. Backend-parametric Web AND AtomPub tests retrieve public bytes and prove
   detected MIME, authorization/capability admission, sanitized
   hash/ETag/size/dedup/quota, same-input/reupload identity and equivalent
   SQLite/PostgreSQL persisted behavior.
3. Real-worker timeout/cancellation/crash and reap-before-release proofs
   exercise response/resource bounds, private modes and spool cleanup.
   Tool/write/check failures prove no original publication, record, quota charge
   or leaked input.
4. Browser success and clear failure preserve existing presentation; real Emacs
   upload/verified pull/reuse/republish preserve originals and Local Media Copy
   trust. Disposable replacement/refusal rehearsal and reviewed procedure
   satisfy the above.
5. Verify dependency bytes/license, Cargo/Nix pinning, production `x86_64-linux`
   packaging, focused proofs, hooks and current exact-head hermetic CI. ARM
   Linux is not a delivery requirement. Apple Silicon macOS is a
   development-environment target checked periodically from committed changes,
   not a per-WIP production package/worker or merge prerequisite. Host checks do
   not assert foreign-native execution. Landing requires scoped human PR
   approval.

## Boundaries

No extra metadata editor/library, vendor/path crate or framework without
conclusive pinned-interface incapability and specific qualified approval. No
rejected source, prototype/worker/tests/helpers/generator or AFTER reuse; admit
only manifested BEFORE data. No native-fork revival, forensic/codec campaign,
pixel/filename redaction, SVG/audio/video/PDF policy, privacy UI, migration,
Revision rewrite or retention waiver. Historical resource clauses do not
authorize architecture copying. No implementation: separate spec approval
precedes short outline/review/approval; no merge/production grant.
