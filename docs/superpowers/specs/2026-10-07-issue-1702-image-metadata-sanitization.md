# #1702 — Sanitize descriptive metadata in uploaded images

Issue: https://github.com/jaunder-org/jaunder/issues/1702

## Outcome

Prevent accidental publication of GPS/location and common personal metadata when
someone uploads a picture. Web and AtomPub uploads share mandatory metadata
removal while preserving the displayed image and the content-addressed identity
of the bytes actually served.

**Scope revised 2026-10-09 at the user's direction:** use conventional metadata
editing and practical regression tests. This is not forensic sanitization,
compressed-stream validation, or proof that every byte is free of hidden data.
The earlier research remains historical evidence, not a delivery prerequisite.

## Load-bearing decisions

### Privacy and image fidelity

- Cover JPEG, PNG/APNG, GIF, WebP, and HEIC/HEIF using established metadata
  tooling. Demonstrate actual successful metadata removal for each format;
  rejecting every HEIF file does not satisfy this spec. A camera-original sample
  is useful additional coverage, not a prerequisite to integration.
- Detect covered image content from bytes, not the filename or supplied
  Content-Type. Relabeling a covered image cannot bypass sanitization.
  Image-labeled malformed or unrecognized raster input cannot become an
  unsanitized successful upload merely because detection fails. For covered
  images, detected format determines stored/served Content-Type, overriding
  mismatched labels while preserving the filename; other Media keeps its current
  Content-Type behavior.
- Sanitization is unconditional for new covered uploads, with no user or
  operator opt-out. Both upload transports enforce the same policy.
- Remove ordinary embedded GPS/location, camera/device identifiers, capture
  timestamps, author descriptions, comments, EXIF/XMP/IPTC descriptive fields,
  and metadata thumbnails/previews supported by the chosen editor. Preserve
  rendering-relevant fields rather than indiscriminately deleting everything.
- Preserve source format and image/frame data: no lossy re-encoding, resizing,
  or automatic conversion of HEIC/HEIF to another format.
- Preserve orientation, displayed dimensions, color/HDR signaling, transparency
  and animation. Retain color profiles intact; do not implement bespoke
  ICC-description scrubbing or reject ordinary profiles because their internal
  fields have not been independently certified. Descriptive text within retained
  rendering profiles is outside this ordinary-metadata policy.
- A tool error, unsupported covered format, or failed post-edit metadata check
  rejects the upload; never publish the original as a fallback. Use the editor's
  supported formats and ordinary error handling, not custom codec grammars,
  universal conformance validation or a forensic accepted envelope.
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
- Verify repeated processing and already-sanitized reupload with the chosen
  tool: the same input and filename should retain a stable sanitized identity.
  Do not add custom format canonicalizers to obtain this property.
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
- Rehearse the supported ownership/reclaim outcomes without claiming original
  retirement: retained Post history can keep a public file accessible after an
  owner's Media Record is deleted. Global safety and other Users' ownership are
  not bypassed.
- Owner-approved scope amendment (2026-10-09): actual historical-original
  retirement and its operator remediation script are a separate follow-up,
  [#1714](https://github.com/jaunder-org/jaunder/issues/1714), not acceptance
  for #1702. That issue must obtain an approved retention policy and separate
  production execution approval.
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
2. A small owned/non-personal fixture set covers each format, planted GPS and
   common personal fields, orientation, a color profile, one representative
   HDR-signaling case, animation where applicable, misleading MIME/name, and an
   ordinary invalid-file/tool-failure case. Inspect edited output in a separate
   metadata-reading invocation; the editor's successful exit alone is not
   evidence of removal.
3. Focused tests show location/personal fields and metadata previews removed,
   image data preserved, correct orientation and retained color/animation
   behavior. Compare the representative HDR signal before and after editing;
   this is a focused signaling check, not certification of arbitrary HDR
   displays or device variants. Both ingress paths serve detected MIME for
   misleading labels. No CABAC/LZW/JPEG entropy validator, exhaustive HEIF graph
   census, mutation campaign or published ISO field-layout proof is required.
4. Same-input and already-sanitized reupload tests prove
   deterministic/idempotent identities, truthful serving hash/ETag/size, and
   existing dedup semantics. Emacs consumer proof covers sanitized upload,
   verified pull and republish without touching original files or weakening
   Local Media Copy checks.
5. Failure proofs include resource-limit breach and tooling/write/verification
   failure; assert no public original, persisted record, quota charge or leaked
   temporary input. Cover SQLite and PostgreSQL persisted behavior equivalently.
6. A disposable-instance rehearsal proves sanitized replacements and updated
   current references, owner-history refusal/record override, independent-owner
   sharing and legacy/global-safety refusal. Verify and report whether old
   public bytes remain; record deletion is not proof of retirement. Document
   caches/backups/local-copy limits. Per the owner's 2026-10-09 approval, actual
   original retirement is deferred to remediation-script issue
   [#1714](https://github.com/jaunder-org/jaunder/issues/1714).
7. Pin chosen dependencies through normal Cargo/Nix mechanisms and demonstrate
   their license, supported-platform packaging, and hermetic gate compatibility.
   Use practical fixture and consumer checks, not another standards-research or
   codec-conformance project.

## Boundaries

No pixel redaction, filename-PII sanitization, SVG sanitization, audio/video/PDF
metadata policy, browser-format conversion, new privacy controls, automatic
historical migration, rewriting immutable Post Revisions, or production mutation
is authorized by this implementation cycle. Arbitrary hidden-data detection,
steganography, forensic recovery of ignored/padding bytes, bespoke ICC
canonicalization and custom compressed-stream validators are also out of scope.
No storage identity, deletion safety, backup-retention or Local Media Copy trust
contract is weakened.

Policy: `docs/adr/drafts/image-upload-metadata-privacy.md`. Feasibility
evidence:
`docs/superpowers/research/2026-10-07-issue-1702-image-metadata-sanitization.md`.
