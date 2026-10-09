# #1702 Image Metadata Sanitization Implementation Outline

> Revised 2026-10-09 at the user's direction: prevent accidental disclosure of
> ordinary photo metadata, especially GPS. Use established tooling and a small
> practical test set, not forensic validation or custom codecs. Contract:
> `../specs/2026-10-07-issue-1702-image-metadata-sanitization.md`.

## Scope

In: shared web/AtomPub metadata removal, preserved presentation, sanitized-byte
identity, ordinary execution/cleanup safeguards, focused consumer tests and a
one-off replacement procedure. Out: compressed-stream validators, custom ICC
scrubbing, exhaustive hidden-data proofs, ISO syntax research, production
access, automatic historical rewrites, format conversion and unrelated UX
refactors.

The previous research is historical context. It is not the production
implementation or a prerequisite to shipping this narrower feature. Do not
package its custom JPEG/GIF/HEVC parsers or run its exhaustive proof campaigns
as feature gates.

## Task outline

- [ ] **1. Check conventional metadata editing on a small fixture set.**
  - Start with pinned ExifTool. Exercise actual JPEG, PNG/APNG, GIF, WebP and
    HEIC/HEIF files with planted GPS and common descriptive fields. Inspect the
    output in a separate metadata-reading invocation.
  - Preserve orientation and color profiles instead of using an indiscriminate
    delete-all operation. Check one oriented image, one profiled image and
    representative animation behavior with existing test consumers. Include one
    owned representative HDR-signaling case and compare its signal before/after
    editing; no arbitrary HDR/display certification is required. No production
    pixel decoding/re-encoding or bespoke profile scrubbing.
  - Check image data preservation, repeated processing and already-sanitized
    reupload. Use generated/owned fixtures already available where useful;
    camera-original samples are additional coverage, not an external-input
    barrier to integration.
  - Record the chosen invocation and ordinary unsupported/error cases. A missing
    ISO excerpt, unproven arbitrary profile or lack of a custom codec validator
    is not a reason to stop this task.

- [ ] **2. Provide a small host-owned sanitizer service.**
  - One `ImageSanitizer` service receives explicit runtime dependencies and
    execution policy, not a storage bundle (ADR-0016). Use a pinned executable
    with fixed arguments, private input/output paths and no shell invocation.
  - Identify covered image formats from bytes using established tooling;
    successful detection controls stored MIME while filenames stay unchanged.
    Preserve SVG/non-image behavior. Claimed raster images that cannot be
    processed do not bypass sanitization or receive an original-byte fallback.
  - Reuse the existing configured maximum for received and output file sizes.
    Limit active jobs to two and processing to 30 seconds. On cancellation or
    timeout, stop and reap the exact child before removing its workspace. Keep
    slow work outside DB transactions and content locks.
  - Use ordinary subprocess isolation appropriate to the application: private
    temporary files, fixed executable/arguments and no sensitive metadata in
    diagnostics. Do not introduce a cross-platform sandbox certification, custom
    per-allocation accounting or container-record/pixel-count parser.
  - Package the runtime through the existing Nix/deployment mechanisms and check
    its license and supported platforms. Missing runtime fails initialization;
    runtime/I/O failures remain internal errors, not successful removal.
  - Test success, invalid input, tool failure, timeout/cancellation and
    temporary cleanup. Return private edited output and detected MIME; do not
    mint a Media identity in this service.

- [ ] **3. Integrate at shared MediaManager ingress.**
  - Composition roots explicitly inject the sanitizer. Web, AtomPub and seed
    uploads converge on private spooling, metadata editing, output size/hash
    measurement and existing finalization. Only edited bytes become public.
  - Hash/URL/ETag/dedup/size/quota describe served bytes. Preserve filename,
    capability admission, per-user Media Records, content locking and reclaim
    behavior. Never alter the author's local original.
  - Rejection leaves no record, public original or quota charge. Preserve the
    existing invalid-upload, oversized and internal-error transport mappings.
  - Use focused backend-parametric manager/HTTP tests for both ingress paths,
    misleading labels, actual metadata removal, stable reupload, quota/size and
    cleanup. Replace fake image fixtures where needed rather than adding a
    test-only bypass.

- [ ] **4. Verify normal consumers and prepare the reviewed PR.**
  - Extend the existing browser Media upload flow with one successful metadata
    removal and one clear failure. Inspect served bytes; retain the existing
    presentation and one-boot/wait discipline.
  - Extend the Emacs upload/pull/republish check for unchanged author originals,
    verified Local Media Copies and sanitized-byte identity. Do not weaken
    existing hash/ETag checks.
  - Run focused host tests and `cargo xtask e2e-local media.spec.ts` first.
    Review at deliverable boundaries, use the enforced commit hook, and reserve
    broad/hermetic pre-PR checks for the shipping boundary. No repeated broad
    gates or mutation campaigns while choosing metadata flags.
  - Remove unused experimental codec machinery from the shipping change where it
    has no role in the practical implementation. Keep any genuinely useful small
    fixtures/checks; do not treat accumulated research as required code.

- [ ] **5. Rehearse the one-off existing-image replacement procedure.**
  - Prepare a procedure for the at-least-two sensitive existing images without
    accessing production. Separately approved execution identifies exact
    targets/owners and uses supported upload/Post update/delete operations.
  - Create new sanitized identities and update current references; never
    overwrite an old hash-addressed file. Respect shared ownership, retained
    owner history and global reclaim guards.
  - Rehearse replacement, independent-owner sharing and legacy/global-safety
    refusal on disposable backends. Report owner Member deletion separately from
    public-byte availability after history override. Document local
    sources/copies, caches/backups and the inability to recall downloads.
  - Owner-approved amendment (2026-10-09): actual original retirement is
    deferred to remediation-script issue
    [#1714](https://github.com/jaunder-org/jaunder/issues/1714). Do not bypass
    retained-history reclaim guards or claim historical privacy erasure as part
    of #1702.

## Working constraints

- Ordinary task order remains 1 → 2 → 3 → 4 → 5. Task1 is a focused tool check,
  not the previous research/conformance barrier. One checkout writer.
- Preserve ADR-0016 injection, ADR-0084 filenames, ADR-0160 Local Media Copies,
  ADR-0176 admission and ADR-0183 ownership/force/reclaim safety.
- No original public fallback, hidden-data-erasure claims, production mutation,
  unapproved lint suppression or weakening of existing storage contracts.
- The proposed ADR and architecture/design projections describe the narrowed
  target until the actual implementation lands; they must not claim it already
  protects uploads.
