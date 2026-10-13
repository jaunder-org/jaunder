# #1702 — Native image metadata correction outline

> Execution resumed on the owner's direction to continue the native correction.
> Execute with `jaunder-iterate` and `jaunder-dispatch`. This outline exists
> because native parsing of untrusted images and enforceable cancellation carry
> security and process-ownership risks. Fork publication, deployment and merge
> remain outside this authorization.

## Scope

In: uphold [single-binary deployment](../../adr/0008-deployment-model.md),
retain the approved conventional
[image privacy policy](../../adr/drafts/image-upload-metadata-privacy.md), and
replace the external runtime editor with linked Rust metadata libraries. The
approved spec remains in Git history at
`2147d7f2ef704320c589b508d36df00fe204f537^:docs/superpowers/specs/2026-10-07-issue-1702-image-metadata-sanitization.md`;
the owner's additional requirement is one deployable executable, with no
ExifTool/Perl installation or runtime fallback.

Out: custom codecs, transcoding, exhaustive compressed-stream validation,
forensic/padding cleanup, ICC-description scrubbing, new privacy settings,
historical migration, production mutation, deployment, and merge without
explicit PR-specific approval. No format or fidelity requirement is dropped.

## Task outline

- [x] Demonstrate a complete native editing engine before changing ingress.
  - Contract: byte-detected covered input produces edited bytes and its MIME, or
    a typed rejection; descriptive metadata is removed while rendering data
    remains. Metadata-free inputs and reuploads have stable identities. Use
    native readers/container writers, not new image codecs.
  - Candidate foundations: `exiftool-rs` 0.9.0's lower-level EXIF/JPEG/WebP
    writers, `img-parts` 0.4.0 for PNG chunks, `gif` 0.14.2's decoder events for
    retaining encoded GIF blocks, and a reviewed `little_exif` 0.6.23 HEIF
    metadata-extent view. Replace EXIF/XMP within owned extents without changing
    encoded image items, layout or rendering properties; do not use its generic
    container rewrite or assume high-level deletion implements sanitization. The
    study needs stricter container/extent handling, metadata item names,
    FILE/IDAT and Adobe XMP UUID carriers, and direct native TIFF reading for
    rendering fields. Preserve primary-TIFF ColorSpace/Gamma, chromaticity,
    transfer-function and YCbCr signaling, and interoperability fields alongside
    Orientation; do not classify these as private metadata. Upgrade its XML
    dependency to at least `quick-xml` 0.41.0 to close RUSTSEC-2026-0194/0195;
    worker isolation is not an advisory waiver.
  - Keep any required dependency patch narrow, licensed, pinned and reviewable.
    Do not publish a fork or change another repository without owner approval.
    Keep Jaunder's first-party grant at `GPL-3.0-only`; no first-party
    relicensing or additional contributor grants are implied. The linked
    `exiftool-rs` dependency is granted `GPL-3.0-or-later`, which permits use
    under GPL version 3 compatibility; `deny.toml` allows that dependency grant.
    Preserve third-party grants and notices, and keep the license policy aligned
    with the implemented choice rather than skipping checks. A patch cannot
    silently accept parser errors or retain original private metadata. If a
    library gap cannot be closed within this scope, report it rather than
    introduce an external fallback or another codec project.
  - Verification: complete owned nine-format corpus plus representative HDR;
    independent metadata and rendering observers; full ICC bytes, orientation,
    all frames/timing/transparency, deterministic/idempotent output. Exercise
    XMP as well as EXIF in HEIC, truncated/container-error cases and deceptive
    labels. Prove metadata previews removed and retained rendering profiles
    unchanged. Goldens change only after independent removal/fidelity proof.
- [ ] Integrate native execution with enforceable limits and shared ingress.
  - Contract: Jaunder launches a private internal worker from the **same
    deployed executable**, which links the native engine. No shell, secondary
    installed executable, runtime dependency search, or network acquisition.
    Explicit composition-root injection and the upload-ready `MediaManager` type
    remain mandatory; test-only observers are not production dependencies.
  - Keep two shared active jobs and one 30-second processing deadline. The owner
    kills/reaps a canceled, timed-out, crashed or failed worker before releasing
    its permit or private files. Do not replace this with `spawn_blocking`
    behind an async timeout: that cannot stop the native work.
  - Bound input/output using the current Media size policy and worker responses
    using the existing 128 KiB bound. Reject unusable worker responses and
    processing failures without logging metadata or serving the original.
    Preserve typed underlying processing/I/O failure sources at the owning
    boundary; the prototype's stage categories alone are not production error
    propagation.
  - Output stays in the owned input spool for same-filesystem placement. Private
    files stay 0600; only fresh validated public placements become 0644. Hash,
    ETag, dedup, size, quota and Media Records depend on the sanitized bytes.
  - Verification: real-worker timeout/cancellation/crash and reap-before-release
    proofs; private-spool and failure cleanup; dual-backend web/AtomPub,
    authorization, MIME, accounting, transport and public-byte proofs. Preserve
    Emacs original/pull/reuse/republish and replacement/refusal behavior.
- [ ] Prove single-executable packaging and corrected acceptance.
  - Contract: remove production ExifTool paths/environment/startup configuration
    and package closure requirements; ExifTool/Pillow/ImageMagick remain only
    independent test tooling. Project the native boundary into the privacy
    draft, architecture view, design and fixture/operator documentation.
  - Verification: run a copied Jaunder executable with ExifTool/Perl unavailable
    to the application and exercise sanitized upload/retrieval. Verify every
    worker executes that copy; distinguish the application's existing platform
    shared-library requirements from an external metadata-tool requirement.
    Check Cargo licenses/advisories and supported native platform packaging.
  - Independently review Standards/Spec, then run applicable focused,
    backend/browser, coverage, hermetic, staged-tree hook and exact-head CI
    proofs. Previous external-runtime receipts certify only that implementation.
    PR #1727 remains unmerged until new acceptance and explicit approval.

## Risk checks

- Native read parity is not removal or malformed-input safety. Successful write
  status is not privacy proof. Validate the edit independently in tests.
- Preserve source image/frame data and ICC/HDR semantics; no GIF or HEIC
  decode/re-encode shortcut. Native decoder events may identify GIF boundaries;
  they must not cause replacement of its encoded frame data.
- Preserve ordinary SVG/non-image passthrough and rejection of malformed or
  unsupported claimed rasters. Do not turn native parsing errors into absence.
- Self-worker lifetime is explicit, local and request-owned; no daemon or helper
  deployment. Avoid extra worker descendants and metadata-bearing diagnostics.
- No storage schema, public HTTP API, immutable identity, ownership/reclaim,
  backup-retention or Local Media Copy trust contract changes.
