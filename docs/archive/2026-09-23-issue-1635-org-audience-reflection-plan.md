# Issue #1635 — Org audience reflection implementation outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only if a task is
> explicitly delegated. This outline exists because adding server-confirmed
> metadata to the Emacs Protocol Client's durable create checkpoint must
> preserve replay/local-ahead safety and legacy-server compatibility. The
> approved spec is
> `docs/archive/2026-09-23-issue-1635-org-audience-reflection-spec.md`.

## Scope

In: per-operation audience capability evidence, publish/update write-back,
server-only and selected server-ahead pull, focused Emacs tests, and user-facing
synchronization guidance.

Out: new wire/server behavior, database changes, bulk local backfill, automatic
conflict resolution, and Named audience discovery. Owner-approved exception:
declare the existing Atom prefix on the server's Service Document so its
title/category elements form valid namespace-qualified XML.

## Task outline

- [x] **Capability evidence before synchronization.** Make a valid,
      version-qualified AtomPub Service Document an operation-bound prerequisite
      to publish and pull, independent of whether the authored Post has audience
      properties or an optional warning cached the document. Distinguish valid
      legacy omission from an advertising server's invalid missing audience;
      unavailable/malformed evidence stops before Post mutation or local pull
      replacement. Preserve the explicit-audience write gate.
  - Verification: pure ERT cases for advertised, valid legacy, unavailable, and
    malformed Service Documents in publish and pull paths. On unavailable or
    malformed evidence, assert unchanged audience-header bytes and no Post
    mutation or local file replacement; retain the explicit-audience gate.
- [x] **Publish/checkpoint audience reflection.** Project a valid returned
      audience into canonical Org properties on ordinary create, draft save, and
      conditional update. Keep an unchanged replay in sync; when the recorded
      create-request digest differs, preserve the current authored properties
      (including omission after a body-only edit) and mark local-ahead until an
      explicit conditional update. Do not compromise the ID/ETag-first durable
      checkpoint or mutate audience on rejection/412.
  - Verification: extend live publish and draft-save tests with absent local
    audience under a non-Public Default Audience, asserting both the remote set
    and saved Org header match. Cover an ID-bearing headerless update and
    canonical multi-target update. Focused write-back/recovery ERT covers legacy
    omission, incomplete advertised responses, matched replay, and
    changed-audience and body-only/omitted-audience replays: retain local-ahead
    state, then perform a conditional update and assert the authored audience
    intent reaches the server. Rejection and 412 must preserve header bytes.
- [x] **Pull/reconciliation audience reflection.** Reuse the same capability
      verdict for server-only pull and selected server-ahead refresh; keep the
      existing staged no-clobber and stale-ETag checks. Assert canonical remote
      targets reach the new or replaced Org file; a valid legacy response
      without audience preserves local headers, while invalid/unknown evidence
      does not replace the file. Update `elisp/README.md` with the new
      post-publish reflection and compatibility rule.
  - Verification: extend live server-only pull and selected server-ahead refresh
    tests; prove a blocked pull leaves audience-header bytes and the entire
    local file unchanged. Run the full pure suite with
    `devtool run -- emacs --batch -Q -l elisp/scripts/run-tests.el` and the live
    suite after `devtool run -- cargo build -p jaunder` with
    `devtool run -- env JAUNDER_TEST_BINARY=<absolute-checkout>/target/debug/jaunder emacs --batch -Q -l elisp/scripts/run-integration-tests.el`.
    Commit via the precommit hook; the prepush hook and PR CI gate the complete
    final tree.

## Risk checks

- ADR-0199's changed-request digest cannot be flattened into "successful
  response means local equals remote"; retain unsent edits and local-ahead
  state.
- ADR-0047's ID/strong-ETag checkpoint remains recoverable after interruption;
  reject invalid audience evidence before recording a synchronized audience
  baseline.
- ADR-0207's omission and canonical multi-target union survive older servers and
  both new and existing Post flows; ADR-0155 and ADR-0024 still keep Atom
  content body-only.
- Keep proof and commits per `jaunder-iterate`/`jaunder-commit`; the hook and CI
  own broader verification.
