# Keyword normalization and add-only repair implementation outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for a bounded
> delegated task. Trigger: reviewed, conditional remote mutations and recovery
> across HTTP and the filesystem. The approved issue-1724 spec is authoritative.

## Scope

In: shared ingress normalization, explicit invalid-category rejection, an opt-in
Emacs preview/apply command, disposable integration proof, and a remediation
runbook. Out: production application, local corpus/marker edits, draft repairs,
Unicode tag expansion, new endpoints, schema changes, and reconciliation policy.

## Task outline

- [x] Task 1: Normalize keyword/category ingress without silent loss.
  - Contract: add one explicit normalization door returning validated `TagLabel`
    values in `common/src/tag.rs`; keep existing `FromStr` validation and
    structured web inputs unchanged. Route raw Org KEYWORDS and AtomPub category
    terms through it. Preserve valid labels; reject unusable or non-ASCII terms
    atomically instead of filtering them out.
  - Verification: pure normalization cases including collisions, valid hyphen
    sequences, punctuation, non-ASCII, blank/unusable input, and cap behavior;
    dual-backend AtomPub create/update and raw Org metadata tests prove equal
    tags and no persisted changes on rejection. Focused host-native Rust proofs
    precede integration gates.

- [x] Task 2: Produce a read-only, identity-bound repair preview.
  - Contract: keep repair orchestration in a dedicated Emacs module, composed
    with existing configuration, inventory, Org parsing, Atom, and transport
    helpers. Reuse the existing Emacs tag grammar authority; implement the same
    explicit normalization door with matching Rust/Emacs regression cases. A
    preview row binds Post ID, local path/byte digest, current remote strong
    ETag, existing labels, proposed missing identities, and eligibility/reason.
    Preview uses no local mtime/sync-marker claim as remote write authority.
  - Verification: real Org parser boundary cases and unchanged local bytes;
    invalid terms, duplicate/missing identities, modified buffers, drafts,
    well-formed but invalid remote Members, and over-cap unions produce blocked
    rows. File/transport/XML-decoding failure aborts preview with its contextual
    cause retained, without granting write authority. Inspect the private corpus
    read-only with this parser; retain counts and full-policy collision findings
    in ignored/session evidence, never tracked fixtures.

- [ ] Task 3: Apply only reviewed additions with conditional remote writes.
  - Dependency: Tasks 1 and 2; deploy the normalization fix before production
    use, as documented in Task 5.
  - Contract: explicit confirmation selects an ordered reviewed batch.
    Revalidate local bytes/clean buffers and fresh unique remote identity plus
    ETag before each write. Construct the replacement from the fetched remote
    Atom representation, changing only categories; do not invoke local publish,
    Media upload, pull installation, or synchronization-marker write-back.
    Preserve existing labels first, append missing normalized identities, and
    validate the complete union. Published/scheduled slug freeze is the only
    admitted permalink-preservation path; drafts remain blocked.
  - Verification: command-level tests prove preview/declined confirmation
    performs no writes, stale local/remote proof blocks, `If-Match` is sent,
    HTTP 412 is blocked, definite failures remain distinct from possible
    dispatch with response loss, and no uncertain write automatically retries.
    Per-Post outcomes survive cancellation between Posts and failed later rows.

- [ ] Task 4: Prove repair preserves newer remote Posts end to end.
  - Dependency: Task 3.
  - Contract: use the existing self-booting Emacs live integration harness and
    disposable synthetic Posts, never production data or credentials. Exercise
    the public repair command rather than only isolated helpers.
  - Verification: server-ahead content, format, title, summary, audience,
    publication state/time, identity, permalink, and existing labels remain
    intact while missing tags are added. Cover scheduled and collision-suffixed
    published slugs, blocked drafts, over-cap union, conditional rejection, and
    lost-response recovery. Verify unchanged local source/markers and a fresh
    no-op second preview. Rust persistence assertions retain backend parity;
    live Emacs and the authoritative Elisp coverage lane cover client behavior.

- [ ] Task 5: Deliver the remediation runbook and final conformance proof.
  - Contract: maintained instructions in `elisp/README.md` explain backups,
    deployment order, preview interpretation, separately approved canary,
    bounded confirmed batches, verification, cancellation, unknown outcomes, and
    later reconciliation without destroying original local keywords. Document
    normalization, rejection, draft blocking, and local markers staying
    untouched. Project enduring behavior into current architecture docs where
    needed; do not commit a corpus inventory or execution report.
  - Verification: inspect runbook commands against delivered command behavior;
    reconcile every spec acceptance item with focused/persisted/live proofs and
    exact-parser census evidence. Use the normal hook-backed commit/push gates
    and `jaunder-ship`'s hermetic pre-PR barrier, including authoritative Elisp
    coverage. Independent final review precedes planning-document cleanup.

## Risk checks

- Normalization is explicit ingress conversion, not a widened tag validator;
  Rust/Emacs must agree on ASCII classification, trimming, identity, and dedup.
- Whole-entry category rejection must precede any Post mutation; retained server
  tags must never be discarded, relabelled, or normalized anew by repair.
- Remote Atom round-trip must preserve authored/lifecycle fields. If the actual
  transport cannot preserve an admitted representation, block it and surface the
  limitation rather than substituting local source or changing the spec.
- Check ADR-0130's freeze in both storage dialects and prove it with a suffix;
  post-update mutation metadata/revisions are expected, not authored corruption.
- An ETag check before a write does not replace conditional `If-Match`; local
  digest checks do not authorize reverting modified visiting buffers.
- No automatic retry after possible dispatch; fresh preview establishes recovery
  intent. No new batch transaction, rollback, or unchanged-local-sync claim.
- Work is serialized in this checkout. Production apply remains a separate
  human-approved operation after shipping and deployment.
