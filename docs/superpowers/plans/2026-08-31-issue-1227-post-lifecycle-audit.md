# Issue #1227 Post Lifecycle Storage Audit Outline

> Execute with `jaunder-iterate`, delegating bounded lenses with
> `jaunder-dispatch`. This outline exists because storage-dialect invariants and
> parallel audit evidence require stable reconciliation contracts.

## Scope

In:

- Publish, unpublish, and soft-delete storage contracts at frozen commit
  `abddb9bce0afc1d3f69920e4013664746f316c86`.
- Shared storage implementation, SQLite and PostgreSQL dialects, web publish,
  web unpublish, web delete, AtomPub member delete, direct tests, domain terms,
  ADRs, and relevant census signals.
- Read-only discovery, terminal candidate dispositions, and focused remediation
  issues for accepted findings.

Out:

- Production or test changes, remediation implementation, and census baselines.
- Post creation, general update, scheduling, downstream feed processing, hard
  deletion/restoration, media behavior, and inbound `ajr_*` ingestion.

## Task outline

- [x] Task 1: Freeze and publish the reproducible audit manifest
  - Contract: record the source commit; exact contracts, implementations,
    callers, tests, glossary terms, ADRs, census query cells, exclusions,
    evidence schema, terminal dispositions, and amendment rule. Each occurrence
    has one stable ID and belongs to exactly one candidate group.
  - Verification: run `devtool run -- cargo xtask census --json`; hash the
    canonical manifest; publish it to issue #1227 and verify exact readback.

- [x] Task 2: Audit the complete lifecycle slice through independent lenses
  - Contract: parallel lenses cover (a) transition and adapter parity, (b)
    callers, DI, transactions, feed events, and error/result conversion, (c)
    direct storage and caller-level tests, glossary, and ADR constraints, and
    (d) census, structural, history, wrapper, and deletion signals. Every lens
    emits the common fields: occurrence IDs, exact path/symbol/range, evidence,
    invariant or risk, relevant decision, confidence, and proposed terminal
    disposition.
  - Verification: all manifest paths and symbols are accounted for; SQLite and
    PostgreSQL behavior is compared for each transition; every direct caller is
    covered; direct storage and caller-level tests reconcile by observable
    contract and backend coverage, with gaps promoted only for concrete risk;
    each declared query records success, unavailable, or failure rather than
    treating absence as clean.

- [ ] Task 3: Reconcile candidates and route accepted findings
  - Contract: deduplicate lens output by concern; apply every audit question and
    the deletion test; assign each group exactly one terminal disposition:
    accepted, rejected by the deletion test, prior-covered, or low-confidence.
    Every rejection retains its concrete deletion-test reasoning. Accepted
    groups use the finding schema from `docs/codebase-audits.md` and map every
    affected caller, adapter, conversion, helper, and test.
  - Verification: occurrence and group counts reconcile with zero unresolved
    groups; every accepted concern is searched against open and closed issues;
    each new one-concern issue has exact readback proving milestone 17 and
    required metadata.

- [ ] Task 4: Publish and archive the complete audit record
  - Contract: issue #1227 receives the frozen source, representativeness and
    boundedness rationale, path/caller/test inventory, adapter comparison,
    useful/noisy signal assessment, accepted dispositions, deletion-test
    rejections with their reasoning, remediation links, canonical hashes, and
    explicit no-production-change statement. Archive the approved spec and
    outline with the final docs commit.
  - Verification: read back every published comment and issue; reproduce the
    canonical hashes and reconciliation equations; confirm the branch-side diff
    contains only audit lifecycle documentation and no generated census output,
    production, test, schema, migration, dependency, or runtime-doc changes.

## Risk checks

- SQLite `BEGIN IMMEDIATE` and PostgreSQL row locking remain deliberate dialect
  differences under ADR-0019 and ADR-0021, not automatic findings.
- Publication-state, revision, timestamp, ownership, liveness, and idempotency
  semantics are compared before judging shared lifecycle depth.
- Feed-event enqueueing is audited only as caller-owned transactional
  coordination; downstream regeneration and WebSub delivery remain excluded.
- Web deletion and AtomPub deletion are both audited; neither may disappear from
  the caller census because they share `soft_delete_post`.
- `CONTEXT.md` distinctions among Post, Deleted Post, Post Revision, AtomPub
  Entry, Syndication Feed, and inbound `ajr_*` vocabulary remain intact.
- Analyzer unavailability and heuristic absence never establish a clean result
  or accepted finding.
- Findings do not become issues without exact evidence, deletion value,
  deduplication, migration scope, and verification contracts.
