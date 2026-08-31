# Issue #1227: Post Lifecycle Storage Audit

## Outcome

Audit the complete storage-backed Post lifecycle transition family at commit
`abddb9bce0afc1d3f69920e4013664746f316c86`: publish, unpublish, and soft delete.
The result documents whether the current cross-crate seam is deep and coherent,
which census signals were useful or noisy, and any evidence-backed remediation
work without changing production code.

## Load-bearing decisions

- The bounded behavioural slice is the Post lifecycle transition family, not the
  broader Post subsystem.
- The audited contracts are `PostStorage::publish_post`,
  `PostStorage::unpublish_post`, and `PostStorage::soft_delete_post`.
- The audit follows each contract through its shared storage implementation,
  SQLite and PostgreSQL dialect implementations, direct production callers,
  result and error mappings, transaction boundaries, feed-event coordination,
  and direct tests.
- Every direct production caller is in scope: the web publish, unpublish, and
  deletion operations and AtomPub member deletion.
- Shared lifecycle machinery is assessed as one ownership boundary. Differences
  between SQLite's write-transaction discipline and PostgreSQL row locking are
  compared as deliberate adapter variation, not presumed duplication.
- Post, Deleted Post, Post Revision, Syndication Feed, and WebSub Publish Ping
  retain the meanings in `CONTEXT.md`; an AtomPub Entry remains a wire
  representation rather than a synonym for a Post.
- The audit applies every behavioural-slice question and the deletion test from
  `docs/codebase-audits.md`.
- The repository census supplies candidates only. Each relevant signal is
  recorded as useful or noisy with the evidence needed to explain that
  classification.
- A remediation is accepted only when exact symbols, callers, tests, risk,
  current seam, proposed ownership, migration, deletion, verification, and
  confidence establish concrete maintenance or correctness value.
- Each accepted remediation becomes one separate, deduplicated GitHub issue in
  milestone 17 using the prescribed finding format. Rejected candidates and
  their deletion-test reasoning remain part of the audit record.
- Discovery is read-only. Audit artifacts may be generated locally but are not
  committed as a census baseline or speculative backlog.

## Acceptance

- The audit record identifies the frozen commit and explains why the lifecycle
  family is representative yet bounded.
- Both storage adapters are compared for all three transitions, including
  revision capture, ownership and liveness guards, timestamps, locking, returned
  records, and failure behavior.
- Every direct production caller is traced through dependency injection,
  transaction ownership, lifecycle invocation, feed-event enqueueing, response
  conversion, and public error mapping.
- Direct storage and caller-level tests are inventoried by observable contract
  and backend coverage; gaps are findings only when they create concrete risk.
- Relevant ADRs and glossary terms are checked before any apparent duplication
  or representation difference is classified as accidental.
- Every candidate receives an evidence-backed terminal disposition: accepted,
  rejected by the deletion test, prior-covered, or low-confidence.
- The audit record states which census signals produced useful candidates and
  which produced noise.
- Accepted findings, if any, have exact tracker readback proving one-concern
  scope, milestone 17 membership, metadata, and no duplicate issue.
- The tracked diff contains no production, test, schema, migration, dependency,
  or runtime-documentation changes.

## Boundaries

- Post creation, general editing, and scheduled publication through the update
  path are context only; their complete callers and tests are not audited.
- Feed-event processing after enqueue, Syndication Feed regeneration, and WebSub
  delivery are context only.
- Hard deletion, restoration, media upload/deletion, and inbound `ajr_*`
  ingestion are excluded.
- This issue records findings but does not implement them, introduce new
  abstractions, or change domain vocabulary or architecture decisions.
