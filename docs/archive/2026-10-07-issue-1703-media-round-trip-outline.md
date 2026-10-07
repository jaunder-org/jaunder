# Verified original Media reuse implementation outline

> Execute with jaunder-iterate; use jaunder-dispatch for bounded delegation if
> useful. This outline exists because original-file reuse crosses remote Media
> verification and concurrent local-file revalidation boundaries.

## Scope

In: the approved
[#1703 spec](../specs/2026-10-07-issue-1703-media-round-trip.md), its draft
ADR/projection, and Emacs consumer documentation.

Out: server/storage changes, upload provenance, directory searches, new UI,
server-only or non-Org behavior changes.

## Task outline

- [x] Task 1: Prove eligible original Org destinations without guessing.
  - Contract: derive transient candidates only from the matched Post's actual
    body links. Keep exact destination spelling excluding fragment, resolved
    path, and byte digest separate. Uniqueness is by spelling, not filesystem
    identity; reuse requires safe containment and valid final-location
    resolution.
  - Verification: pure ERT covers repeated versus alias/distinct-file ambiguity,
    encoded and implicit/explicit links, fragments, excluded forms, renamed
    equal bytes, missing/changed/unreadable and non-regular files, symlinks and
    cross-root targets.
- [x] Task 2: Stage verified Media with optional original destinations.
  - Contract: extend the existing native-source plan/staging flow with optional
    matched-local evidence; ordinary callers retain current behavior. Retain
    remote verification even for reuse. Carry transient reuse evidence and
    sufficient fallback information alongside staged rendered Post bytes. Never
    install redundant Local Media Copies for successfully reused references.
    Preserve remote labels/fragments and non-destination source bytes.
  - Verification: ERT proves reuse, mixed reused/fallback references, remote
    link additions/removals, invalid remote evidence rejection, no redundant
    copy, and unchanged server-only, Markdown/HTML and durable-copy no-overwrite
    behavior. Assert original file bytes remain unchanged on reuse and fallback.
- [x] Task 3: Revalidate reuse in matched consumers and demonstrate round-trip.
  - Contract: server-ahead and keep-remote revalidate originals and finalize any
    verified fallback before replacing the Post. Re-run existing local Post
    preflight after any fallback work that can invalidate its safety snapshot.
    Merge uses reuse only while staging its remote snapshot; completion remains
    ordinary authored conditional publication, with no automatic result rewrite.
  - Verification: pure and live ERT exercise server regeneration followed by
    matched pull, keep-remote, merge staging/completion, changes during staging,
    canonical-slug rename, stale local/remote evidence, modified buffers and
    destination collisions. Update client docs and retain existing failure
    tests.

## Risk checks

- Tasks 2 and 3 depend on task 1's exact-spelling proof; task 3 depends on task
  2's transient staged evidence. These are sequential slices, not parallel
  writers.
- Fetching/verification and fallback must leave the Post untouched until all
  existing final checks pass. Retain and clean temporary evidence on all exits;
  cleanup errors use the existing warning path, never obscure the primary error.
- Expected local ineligibility is fallback; unexpected I/O errors and invalid
  remote proof remain failures. Preserve the documented post-check filesystem
  race and recoverable replace-at-old-path/rename semantics.
- Verify focused pure/live Emacs behavior first, then the normal commit/push
  gates and CI's authoritative Elisp coverage. No exemptions or suppressed
  checks.
