# #1702 — Sanitize descriptive metadata in uploaded images

Issue: https://github.com/jaunder-org/jaunder/issues/1702

## Outcome

New device-photo and other covered raster uploads cannot expose their embedded
GPS or other descriptive metadata through Jaunder's public Media URLs. Web and
AtomPub uploads share mandatory sanitization while preserving the displayed
image and the content-addressed identity of the bytes actually served.

## Load-bearing decisions

### Privacy and image fidelity

- Cover JPEG, PNG/APNG, GIF, WebP, and HEIC/HEIF. Successful sanitization of
  representative device-native HEIC/HEIF is required; rejecting every HEIF file
  does not satisfy this spec.
- Detect covered image content from bytes, not the filename or supplied
  Content-Type. Relabeling a covered image cannot bypass sanitization.
  Image-labeled malformed or unrecognized raster input cannot become an
  unsanitized successful upload merely because detection fails. For covered
  images, detected format determines stored/served Content-Type, overriding
  mismatched labels while preserving the filename; other Media keeps its current
  Content-Type behavior.
- Sanitization is unconditional for new covered uploads, with no user or
  operator opt-out. Both upload transports enforce the same policy.
- Remove embedded GPS/location, device/camera descriptions and identifiers,
  timestamps, author/copyright descriptions, comments, XMP/IPTC and equivalent
  descriptive metadata, and embedded thumbnails/previews. This is a metadata
  guarantee, not removal of visible sensitive content or arbitrary
  steganography.
- Preserve source format and compressed image/frame payloads: no lossy
  re-encoding, resizing, or automatic conversion of HEIC/HEIF to another format.
- Preserve orientation, displayed dimensions, color/HDR appearance,
  transparency, displayed frames, timing, looping, blend, and disposal behavior.
  Retain only validated presentation metadata needed for those properties.
- Scrub descriptive ICC fields while preserving their color transforms. Neither
  discarding a necessary profile nor retaining arbitrary descriptive profile
  data satisfies the policy.
- Embedded thumbnails/previews must be removed. Other optional camera editing
  extras, including portrait depth data, may be discarded. Required alpha/HDR/
  color information or displayed images/animation must not be discarded as an
  editing-extra shortcut.
- Unsafe, malformed, unsupported covered variants and sanitization/verification
  failures reject the upload; never publish the original as a fallback.
  Unsupported variants must be documented rather than silently accepted.
- SVG remains accepted unchanged and is explicitly outside the guarantee.
  Non-image Media is also outside scope; this must not become a new general
  attachment allowlist.

### Storage, limits, and Protocol Clients

- Complete sanitization before public placement or creation of a Media Record.
  Unsanitized working bytes are private temporary input, not retained originals
  on the server, and are cleaned up on success and failure.
- Hash, deduplication identity, returned size, URL, ETag, and quota accounting
  describe the sanitized bytes actually stored and served. Existing canonical
  filename/path and independent per-user Media Record contracts remain intact.
- Repeated sanitation is deterministic and byte-idempotent: reuploading the same
  input, or republishing its already-sanitized Local Media Copy under the same
  filename, does not manufacture another byte identity.
- Preserve existing upload-capability admission and positive limits. Enforce
  maximum file size on both received and sanitized bytes; charge user quota for
  stored bytes. Sanitization introduces bounded parser/resource execution, with
  explicit documented limits and rejection behavior in the approved outline.
- Rejection leaves no Media Record or public file and consumes no user quota.
  Unexpected tooling/storage/I/O failures propagate rather than becoming success
  or an ordinary unsupported-format result. Diagnostic evidence contains no PII.
- Do not modify authors' original local files. AtomPub continues returning the
  authoritative server URL; Emacs pull verifies the sanitized bytes against that
  URL's hash and strong ETag and creates an ordinary durable Local Media Copy.
- Existing Media URLs/ETags and existing Local Media Copies remain unchanged.
  There is no automatic retrospective rewrite or background migration.

### One-off production remediation

- Deliver a reviewed operator procedure for the at-least-two sensitive existing
  production images; running it against production requires separate approval.
- Inventory exact old identities, affected current Posts and shared ownership;
  create sanitized replacements with new identities and update current Post
  references through supported writes. Never overwrite an old hash-addressed
  file.
- Retire sensitive original records/files using existing ownership/reclaim
  safety checks. The owner explicitly accepts broken old links in retained Post
  Revisions/Deleted Posts where an authorized force deletion overrides owner
  history. Global safety and other Users' ownership are not bypassed.
- Verify whether old binaries are actually unavailable; removing one owner's
  record alone is not proof when another record retains the bytes. Refusal or
  remaining ownership is an explicit incomplete-remediation outcome.
- Address stale local upload sources, durable local copies, immutable caches and
  backups in the procedure. Do not claim remote downloads or caches can be
  recalled, or that deleting current files purges backup copies.

## Acceptance

1. Backend-parametric integration tests exercise both web and AtomPub upload
   ingress, public retrieval, rejection, quota and record/file cleanup. A
   browser upload flow proves successful sanitation and a clear failure; retain
   existing presentation rather than adding a privacy-settings UI.
2. A versioned synthetic/non-personal fixture corpus covers every accepted
   format, representative real-device HEIC/HEIF structures, misleading
   MIME/name, descriptive metadata classes, ICC descriptions, previews,
   malformed input, and unsupported structures. Use independent output
   inspection, not merely the sanitizer's exit status or its own report.
3. Fixture proofs compare compressed payloads, orientation and rendering,
   color/HDR transforms, transparency, and animation semantics as applicable.
   Metadata inspection proves descriptive fields and embedded
   thumbnails/previews gone. Both ingress paths serve detected MIME for
   misleading labels. HEIF's retained item/reference graph is verified, not just
   its primary Exif.
4. Same-input and already-sanitized reupload tests prove
   deterministic/idempotent identities, truthful serving hash/ETag/size, and
   existing dedup semantics. Emacs consumer proof covers sanitized upload,
   verified pull and republish without touching original files or weakening
   Local Media Copy checks.
5. Failure proofs include resource-limit breach and tooling/write/verification
   failure; assert no public original, persisted record, quota charge or leaked
   temporary input. Cover SQLite and PostgreSQL persisted behavior equivalently.
6. A disposable-instance rehearsal of the one-off procedure proves replacements,
   current references and original retirement, including owner-history override
   and a shared/global-safety refusal. Document caches/backups/local-copy
   limits.
7. Pin chosen dependencies through normal Cargo/Nix mechanisms and demonstrate
   their license, supported-platform packaging, and hermetic gate compatibility.
   Source-backed research is not a substitute for fixture and consumer proof.

## Boundaries

No pixel redaction, filename-PII sanitization, SVG sanitization, audio/video/PDF
metadata policy, browser-format conversion, new privacy controls, automatic
historical migration, rewriting immutable Post Revisions, or production mutation
is authorized by this implementation cycle. No storage identity, deletion
safety, backup-retention or Local Media Copy trust contract is weakened.

Policy: `docs/adr/drafts/image-upload-metadata-privacy.md`. Feasibility
evidence:
`docs/superpowers/research/2026-10-07-issue-1702-image-metadata-sanitization.md`.
