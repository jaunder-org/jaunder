# Durable local media copies for pulled Posts — implementation outline

> Execute with `jaunder-iterate`; delegate bounded tasks with
> `jaunder-dispatch`. This outline exists because issue #80 changes the pull
> protocol trust chain, anonymous-vs-authenticated transport, and multi-file
> installation ordering across three native source formats.

## Scope

In:

- Pure pulled-Member data and native-format link planning.
- Anonymous same-origin binary acquisition with exact instance/hash validation.
- Durable Local Media Copy installation, retry/reuse, and pull orchestration.
- Pure and live ERT plus the decision/glossary/architecture/user-doc updates.

Out:

- External media, redirects, matched-Post repair, cache eviction, orphan
  collection, reconcile report changes, background retry, server changes, and
  Markdown/HTML republish work.

## Task outline

- [x] Task 1: Produce deterministic localization plans for Org, Markdown, and
      HTML
  - Contract: expose one pure pulled-Member data seam so the existing exact
    Entry-to-Org wrapper and the pull orchestrator share one XML parse. A
    localization plan identifies each distinct eligible canonical media URL, its
    expected hash, decoded safe leaf, native encoded relative target, and every
    source replacement; planning and application perform no I/O.
  - Contract: candidates are absolute configured-origin public-media URLs with
    no userinfo/query; link destinations only. Plans preserve labels, alt text,
    order, fragments, and all unrelated bytes.
  - Verification: pure ERT pins byte parity for the existing mapper; all three
    formats; `srcset`; encoded spaces/percent/non-ASCII; duplicates; and every
    unchanged URL class.

- [x] Task 2: Materialize verified Local Media Copies without credentials or
      overwrite
  - Contract: add a pull-media binary adapter that never calls the authenticated
    AtomPub transport, sends no App Password, follows no redirect, accepts only
    direct `200`, and preserves response bytes exactly.
  - Contract: exactly one canonical `X-Jaunder-Instance` UUID must match the
    Member identity; computed SHA-256 must match the canonical URL hash and
    strong `"sha256-<hash>"` response ETag. Install at
    `local-media/<sha256>/<decoded-filename>` without symlink traversal or
    overwrite; create staging files exclusively and verify existing bytes before
    reuse. The root is trusted author-owned state: replacement after Emacs's
    final check is out of scope without dirfd-anchored mutation.
  - Contract: stage all distinct downloads, retain verified copies after any
    late failure, and clean temporaries. Reuse remains internal; tests count
    calls at the binary-adapter seam without adding production-only evidence.
  - Verification: pure adapter/filesystem ERT covers status, redirect,
    transport, header cardinality/syntax, hash, path, race, corruption,
    temporary cleanup, retained-copy, and reuse behavior.

- [x] Task 3: Integrate fail-closed localization into server-only pull
  - Contract: occupied `<slug>.org` remains the first preflight. After the
    authenticated Member GET and identity validation, build the localization
    plan, materialize every Local Media Copy, render the rewritten native body,
    then install the Post through the existing atomic no-overwrite seam.
  - Contract: preserve deterministic sequential fail-fast reconciliation. A
    failed Post remains server-only, so rerunning `jaunder-reconcile` retries it
    and reuses verified copies.
  - Verification: pure pull-orchestration ERT injects every planning,
    acquisition, materialization, and final-install failure and proves the
    specific error, Post-last ordering, retained existing/verified entries,
    temporary cleanup, and retry GET count.
  - Verification: extend real-server live ERT to prove anonymous download, exact
    bytes and relative links, offline preview, late-failure retry/reuse, two
    separate Posts sharing one copy without a second binary GET, and
    occupied-destination no-I/O. Org republish must deduplicate while leaving
    the relative source link and Local Media Copy bytes/path unchanged. Run the
    hermetic `.#checks.x86_64-linux.e2e-elisp-integration` check at the
    deliverable boundary.
  - Docs: keep the proposed ADR, `CONTEXT.md`, and `docs/ARCHITECTURE.md`
    projection with the feature; document `local-media/` durability, backup, and
    retry behavior in the Emacs user guide. Do not edit `docs/README.md` or
    promote the draft.

## Risk checks

- The App Password appears only on the AtomPub Member request; no accepted media
  URL or redirect path can induce an Authorization header.
- URL origin comparison includes scheme, normalized host, and effective port;
  URL userinfo and query are rejected before network I/O.
- Member and media instance identity use the accepted exactly-one canonical UUID
  contract; ambiguous/malformed headers fail closed.
- Native target encoding resolves the decoded on-disk leaf for spaces, literal
  percent signs, and non-ASCII names in every format.
- No normal failure exposes a pulled Post before all media verifies. Verified
  copies may remain; pre-existing entries and the Post destination are never
  overwritten.
- Existing pull byte fixtures, pagination/inventory/reconcile classifications,
  strong ETag behavior, and Org publish media localization remain unchanged.
- Every changed pure mapping/transform has ERT; the live suite uses the real
  AtomPub server and public media route per ADR-0035.
