# #1702 Image Metadata Sanitization Implementation Outline

> Execute with `jaunder-iterate`; delegate bounded tasks with
> `jaunder-dispatch`. Trigger: cross-protocol privacy enforcement, lossless
> HEIF/container handling, bounded untrusted-input execution, and stored-byte
> identity/cleanup correctness. Approved contract:
> `../specs/2026-10-07-issue-1702-image-metadata-sanitization.md`.

## Scope

In: the approved shared upload policy, independent fixture/consumer proofs,
pinned runtime packaging, and a rehearsed one-off remediation procedure. Out:
production execution, historical rewrites, format conversion, SVG/non-image
sanitization, new privacy controls, and unrelated storage/UX refactors.

## Task outline

- [ ] **1. Prove the lossless candidate and freeze its supported envelope.**
  - Deliver a licensed, non-personal fixture corpus and an executable decision
    record demonstrating successful JPEG, PNG/APNG, GIF, WebP and representative
    device-native HEIC/HEIF sanitization. Start by evaluating pinned ExifTool
    13.59 metadata editing; do not treat `-all=` or a format table as proof.
    Select it only where independent container inspection proves the contract;
    use bounded format-specific rewriting where the candidate leaves gaps.
  - Contract: `host` owns format identification, rewrite and independent output
    validation behind one focused `ImageSanitizer` service. Its successful
    result owns a private, verified output and its detected Content-Type; it
    does not mint a Media Record, hash URL or upload-success metric. Routing has
    three outcomes: verified covered image; SVG/non-image pass-through; or
    rejected claimed/plausible raster whose recognition/validation fails. Bytes
    override labels for recognized images; a raster MIME/extension claim
    (including HEIC/HEIF) or plausible covered signature/prefix cannot escape
    via failed detection. Freeze accepted format/HEIF brand canonical-MIME
    mappings and plausible-prefix rules. SVG requires actual SVG content, not a
    mislabeled raster. Other unfamiliar attachments without raster evidence keep
    existing behavior; this is not a new allowlist for general attachments. The
    service receives explicit execution policy and runtime dependencies, never a
    storage bundle.
  - Ownership seam: the manager owns a private upload-workspace guard from first
    spool write. Sanitization consumes the original-input guard, keeps all
    intermediates private, and returns a verified-output guard only after
    removing unsanitized input. Finalization consumes that guard, transferring
    only verified output into the public store. Error/drop/caller cancellation
    retains cleanup responsibility with the current guard owner; neither a
    borrowed path nor a detached worker outlives its workspace. Cleanup errors
    preserve the primary failure and use the owning reporter; inaccessible
    crash-orphans remain confined to the existing startup-swept temp tree.
  - Proof: retain compressed payloads, scrub descriptive ICC data, preserve
    orientation/color/HDR/alpha/frame semantics, remove previews, and
    demonstrate deterministic and byte-idempotent output. HEIF proofs inspect
    retained item references, not just primary Exif. Record supported
    brands/item structures, explicit rejections and exact
    dependency/license/platform closure.
  - Feasibility barrier: no shared-ingress integration until these proofs pass.
    If representative HEIF or ICC fidelity cannot be achieved within the
    approved contract, stop and report evidence; do not redefine delivery as
    rejection of every device photo or silently introduce lossy conversion.

- [ ] **2. Make sanitizer execution bounded and failures evidence-bearing.**
  - Contract: one service instance per composition root limits active jobs to
    two; acquire its permit before reading/retaining extra working data. Never
    wait for processing while holding a DB transaction or Media content lock.
    Input/output file sizes use the existing configured maximum, not a new
    upload setting.
  - Initial fixed safety envelope: at most 8 MiB aggregate descriptive/profile
    data, 65,536 parsed container records, nesting depth 32, 100 million pixels
    per displayed frame, and 4,096 displayed frames per file. Enforce checked
    lengths, offsets and arithmetic before allocation; do not decode pixels in
    production merely to remove metadata. Validate compressed structure to the
    extent required by the supported envelope without claiming steganography
    detection. Every job has a 30-second processing deadline and a 512 MiB
    allocation ceiling, including input/output and working state.
  - Execution-mode boundary: in-process Rust uses checked reservation/accounting
    before every allocation, bounded structural/payload-copy work, and
    cooperative deadline/cancellation checks at bounded work intervals. No
    uninterruptible detached `spawn_blocking` work; ownership stays with the job
    until work stops. No allocator-abort path or async timeout pretending
    synchronous work stopped.
  - Native work, if retained from task 1, uses enforceable filesystem/network/
    environment isolation and OS memory limits: private invocation input/output,
    read-only explicitly admitted runtime closure, no network or inherited
    secret environment. Deadline/cancellation terminates and reaps the exact job
    process tree before workspace cleanup. No shell, ambient executable lookup,
    arbitrary file arguments, `_original` public artifacts or unbounded
    fallback.
  - Package the chosen runtime and independent fixture tools in actual Cargo/Nix
    production/test/devShell closures now. Select and demonstrate the execution
    boundary on x86_64/aarch64 Linux and aarch64 Darwin before task 3: positive
    allowed-operation proof plus denied network/unrelated-file access, effective
    memory/deadline limits and child-tree reaping where native code is used.
    Test missing/bad runtime and unenforceable boundaries fail initialization.
    Evaluation alone is not native isolation proof. If any required platform
    cannot satisfy the chosen boundary, stop; select a conforming mode or seek
    an outline amendment rather than weakening guarantees or platform support.
  - Proof: boundary/exceed cases, malformed offsets/cycles/duplicates,
    concurrency, deadline/cancellation, missing tool, partial output,
    warning/error and verifier mismatch. Preserve typed infrastructure failures
    and the primary error; report ancillary cleanup failures through the owning
    reporter without metadata/PII. Verify the supported corpus fits these
    bounds; changing this envelope or the sandbox/platform contract requires an
    outline amendment before integration.

- [ ] **3. Integrate verified bytes into every MediaManager upload path.**
  - Contract: composition roots inject the same sanitizer explicitly. Streaming,
    AtomPub byte uploads and seed uploads converge on private spooling,
    sanitization, output size/hash measurement, then existing finalization. Only
    output hash/size/detected MIME enter metadata for covered images. Filename,
    capability admission snapshot, content lock and reclaim sequencing stay
    intact.
  - Error contract: claimed/plausible raster detection failure and covered
    malformed/unsupported/unsafe input become the existing invalid-upload
    classification; received/output size violations retain the existing
    oversized classification; quota remains based on stored bytes.
    Native/I/O/storage failures remain internal failures, not unsupported
    images. Both transports retain their existing public status/error mapping
    and emit exactly one upload outcome. No public file/record/quota charge on
    rejection.
  - Proof: `#[apply(backends)]` manager and HTTP tests for both transports,
    omitted and misleading labels, idempotence/dedup, output quota/size and
    truthful GET Content-Type/hash/ETag. Inject controlled sanitizer and I/O
    failures to prove cleanup, capability rejection before work, and no
    original-byte fallback. Update fake image fixtures to valid images rather
    than weakening validation.

- [ ] **4. Prove real consumers and hermetic deployment.**
  - Extend existing `media.spec.ts` and AtomPub consumer coverage for successful
    sanitation and clear rejection, with independently inspected downloaded
    bytes. Preserve one-boot/wait discipline and existing presentation. Extend
    live Emacs upload/pull/republish proof: untouched author-local original,
    verified Local Media Copy, stable already-sanitized reupload identity. Add
    HEIC/HEIF MIME recognition in the client only where necessary for that
    consumer contract.
  - Reuse task 2's already-proven pinned runtime/verification closures and
    native execution boundaries; confirm hermetic consumer lanes use those same
    inputs, not an ambient executable or weaker test-only sanitizer.
  - Verification: focused `cargo xtask test-local` manager/HTTP filters first;
    `devtool run -- cargo xtask e2e-local media.spec.ts` for the browser flow;
    the existing live Emacs lane, then hermetic non-e2e validation and the
    applicable SQLite/PostgreSQL Chromium and Firefox e2e lanes before PR
    handoff.

- [ ] **5. Rehearse remediation and finalize the policy documentation.**
  - Deliver an operator procedure with an individual inventory/remediation
    checklist for each of the at-least-two known sensitive production images:
    exact old/new identities, current Post references, independent owner records
    and retained owner history. Identify both concrete targets before separately
    approved production execution; drafting/rehearsal does not access
    production. Use supported upload/Post writes and owner force-delete/reclaim
    operations, never SQL rewrites, old-path replacement or global-safety
    bypass.
  - Rehearse on disposable SQLite/PostgreSQL instances: successful replacement
    and current-reference update, truthful original unavailability, broken
    retained history after explicit override, and shared/global-safety refusal.
    Document partial-failure recovery and separate approval for each production
    execution.
  - Address stale local publish sources/copies, caches and backup copies
    explicitly. Finalize the proposed ADR/architecture/design projection from
    measured delivered behavior; record format/limit coverage and evidence
    without claiming recall of downloads or universal hidden-data erasure.

## Risk checks and coverage

- Task order is 1 → 2 → 3 → 4 → 5; one repository writer, no worktree switching.
- Spec acceptance 1/5: tasks 2–4; 2/3: tasks 1–2/4; 4: tasks 1/3–4; 6: task 5;
  7: tasks 1–2/4. No schema migration or storage-trait widening is planned.
- Preserve ADR-0016 injection, ADR-0084 filenames, ADR-0160 Local Media Copies,
  ADR-0176 admission and ADR-0183 independent ownership/force/reclaim safety.
- No slow sanitization inside DB scopes/locks; no original hash reused for
  changed bytes; no sensitive child diagnostics in telemetry; no test-only
  bypass that makes production ingress or consumer proofs vacuous.
- Existing positive limits remain authoritative. Proposed execution bounds are
  security constraints, not evidence of measured fixture suitability: task 1/2
  must supply that proof before production integration.
