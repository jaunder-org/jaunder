# #1702 — Make uploaded pictures safe to publish

APPROVED — JAU-1702-APP-OUTLINE-001; follows the
[approved spec](../specs/2026-10-18-issue-1702-native-image-upload-privacy.md).
After approval, use `jaunder-iterate` / `jaunder-dispatch` to do the work.

## Why this needs a plan

Remove private details before publication without changing how the picture
looks. Calculate the stored hash, URL, size and quota charge from the cleaned
bytes, which may differ from the original. Repeating the same upload or
uploading its already-cleaned result with the same filename must give a stable
cleaned identity. A timed-out thread can keep running; a separate process lets
Jaunder actually stop it.

Each upload checks permission and the size limit, then takes a free work slot or
replies busy. It receives a private copy, cleans and checks it, and only then
hashes and stores the finished bytes.

## Work and checks

- [x] 1. Make one small picture-cleaning operation work before expanding tests.
  - Following the user's explicit release switch decision, use upstream
    `https://github.com/Le-Syl21/exiftool-rs` v0.13.0 at the release tag's full
    commit `863f9b8eb47f995eddef657e006e1a22b3a28334`, with its empty default
    feature set and no GUI features, through normal Cargo/Nix Git resolution
    (not a claim of crates.io publication). This release directly follows the
    previously approved unreleased PNG fix
    `b60a78446730bb7d25fdb367df4e1a7d9cb3c461` (package version 0.12.0), with
    only version/changelog/lock metadata changes. Verify the package, license
    and normal pinning; future automatic release switches remain unauthorized.
  - Remove planted ordinary EXIF/XMP/IPTC location, device, date, author,
    description, comment and preview data. Keep encoded picture/frame data,
    orientation, dimensions, intact ICC profiles, HDR information, transparency
    and animation. Do not erase all XMP and assume HDR survives.
  - Detect file contents using a private filename without an extension: the
    author's name must not steer detection. Reject malformed image-labelled
    input. Leave SVG/non-image bytes AND their existing MIME types unchanged.
  - Read the finished file again to check type and ordinary metadata removal.
    Zero edits are valid for an already-clean file; a positive count proves
    nothing. Respect legacy empty markers, without forensic recovery of ignored
    bytes.
  - Start with a non-personal JPEG; inspect output with TEST-only ExifTool and
    data comparisons.
- [x] 2. Run cleaning in a process Jaunder can stop and wait for.
  - Check upload permission/current positive size limits. Immediately take one
    of TWO shared work slots or return HTTP 503 Busy, without a waiting queue.
    This busy rule also applies to SVG/non-image uploads, without changing their
    bytes or MIME.
  - Receive a private working copy; count actual input bytes against the size
    limit. Use folders 0700/files 0600 under media/tmp. Separate controlled
    input/output names share one filesystem; never edit author originals or
    existing public files.
  - Directly launch another copy of the SAME installed Jaunder executable,
    running only the native job before ordinary logging/database startup. No
    shell, extra installed tool, runtime tool search, Perl or network
    acquisition.
  - ONE 30-second clock starts before launch and covers
    launch/edit/check/status/exit. Receiving keeps normal upload-transport
    handling; a thread timeout is not a substitute.
  - On disconnect/error/timeout/crash/shutdown, stop the child if still running
    and WAIT until it exits; THEN clean working files/release the slot. Keep
    this responsibility when the request stops waiting. Before launch, stop
    receiving/close handles/clean up.
  - Shutdown refuses new work, cancels active work and waits for all owned work
    before releasing the existing runtime lock. Success also waits for child
    exit; keep the finished file protected and owned until saving or cleanup
    completes.
  - Limit status data to 128 KiB before decoding; reject
    incomplete/unknown/oversized data or disagreement with process exit. Return
    only output name/type; discard raw stderr. Never log metadata, paths or
    environment.
  - If cleaning fails, send the specific native-library or I/O error kind and
    safe details, including an OS error code when available. Reconstruct the
    corresponding typed error and its cause in the main process; saying only
    which step failed is insufficient. Preserve original launch, pipe and wait
    errors. Keep descriptions bounded and free of private data. Never relabel
    unexpected read, write, conversion or checking failures as “unsupported
    image.”
  - Test two jobs/third busy, deadline, disconnect, crash, shutdown,
    wait-before-cleanup, response bounds, actual error causes, zero-count
    success and cleanup failures. No detached native work or fallback
    publication of the original.
- [x] 3. Give Web and AtomPub the same cleaning-and-saving path.
  - Require cleaning whenever an upload-ready MediaManager is constructed. Pass
    the cleaning step explicitly at server startup; neither streamed nor
    in-memory uploads may skip it.
  - Before saving, re-read the finished file's metadata and type, and enforce
    the output size limit. The tests below check rendering and encoded
    image/frame content. Hash/measure THAT file for deduplication, URL, ETag,
    quota and Media Record; keep the author's canonical filename. Only fresh
    validated public files become 0644; never change permissions on existing
    deduplicated files.
  - Keep content locks/short database transactions; cleaning/file I/O stays
    outside them. Remove only owned new files after confirmed failure. An
    uncertain database commit must not be treated as failure followed by
    deletion of a possibly saved file.
  - Prove real success/failure through both routes before expanding coverage.
    Partial uploads/write/check/cancellation failures leave no private input;
    rejection leaves no public original, new record or quota charge.
- [x] 4. Check pictures and real users' workflows.
  - Small owned JPEG, PNG/APNG, GIF, WebP, HEIC/HEIF fixtures prove actual
    removal in each, retained ICC/HDR/orientation/frames/transparency,
    misleading labels, invalid images and stable same-input/already-clean
    reuploads. Fresh tests; reuse only documented BEFORE data.
  - Test SQLite/PostgreSQL and both upload routes: authorization, limits, quota,
    cleanup, public bytes and detected MIME/hash/ETag/size. Existing immutable
    URLs stay unchanged.
  - Browser success looks/animates the same; failures are clear. Real Emacs
    upload, verified pull/reuse/republish leave local originals untouched and
    retain mandatory server-copy hash/ETag/instance checks and stable identity.
- [ ] 5. Finish the replacement procedure and normal delivery checks.
  - On a disposable instance, rehearse creating replacements and updating
    current Posts. Verify that retained Post history normally prevents removal,
    and that the supported owner-approved override removes only the owner's
    Media Record. Another user's record must still protect shared files. Do not
    delete older files where the existing rules cannot establish safety. Report
    whether the old public URLs remain accessible. Review a procedure for the
    two sensitive images, covering local copies/stale sources/caches/backups; do
    not run it in production. Actual historical-original retirement/script stays
    in #1714; remote copies cannot be recalled.
  - Record non-obvious decisions in ADR/current architecture docs. Check
    production `x86_64-linux` packaging, licenses, Cargo/Nix and current-main
    fixes. ARM Linux is not a delivery requirement; Apple Silicon macOS targets
    only the development environment, checked periodically from committed
    changes rather than as a per-WIP production/worker or merge prerequisite.
    Use normal hooks, independent review and exact-head CI before scoped human
    merge approval; tick completed tasks before their commit gate. No
    rejected-code reuse, fork restart, extra editor/vendor/path crate,
    framework, codec campaign, weaker guarantee or extra production authority.
