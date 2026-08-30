# Interface Friction Audit Execution Outline

> Execute with `jaunder-iterate`, delegating bounded semantic-review slices with
> `jaunder-dispatch`. This outline exists because exhaustive multi-agent review
> requires stable occurrence, grouping, and disposition contracts.

## Scope

In:

- Freeze a finite search suite before inspection, execute it against one commit,
  reconcile every occurrence, semantically review candidates, and publish
  durable tracker dispositions.
- Keep raw search, census, ledger, and review-working files ephemeral under
  `.xtask/audits/issue-1224/`.

Out:

- Production code, tests, schemas, migrations, runtime documentation, and public
  behavior.
- Remediation itself; accepted findings become separate milestone-17 issues.
- Repetitive tests, dead/shallow modules, existing #1178/#1248 ownership, and a
  #1227 slice unless that issue first records and is cited for its durable
  choice.

## Task outline

- [x] Task 1: Freeze the audited commit and search manifest on issue #1224.
  - Contract: the comment records the audited SHA, tracked Rust/TypeScript/Elisp
    roots, stable query IDs, exact tool commands/patterns/options and versions,
    normalization/grouping rules, and finite fallbacks for every unavailable
    required lens. A coverage matrix maps every language/root to conversions,
    serialization, collections, copying, argument bundles, error mapping, and
    call order, plus structural, semantic-reference, and history evidence. Later
    changes create a versioned amendment and full rerun.
  - Verification: read back the GitHub comment; prove every matrix cell has an
    auditable population or executed finite fallback, then independently
    reconstruct every command before semantic inspection begins.

- [x] Task 2: Execute discovery and reconcile the raw occurrence population.
  - Contract: `.xtask/audits/issue-1224/occurrences.jsonl` has one record per
    `(query_id, normalized_location_or_symbol)` with raw evidence; `groups.json`
    assigns each occurrence to exactly one candidate group while preserving
    per-query totals and cross-query overlaps. Census output and query captures
    remain adjacent and ephemeral.
  - Verification: rerun the frozen suite; arithmetic checks prove captured raw
    counts equal occurrence counts, every occurrence has one group, per-query
    totals reconcile, and overlaps are explicit.

- [ ] Task 3: Complete behavioral-slice review and proposed dispositions.
  - Contract: candidate groups are independent review packets. Each packet names
    paths/symbols, definitions/references, callers, representations, errors,
    tests, relevant history and ADRs, backend comparison where applicable,
    current seam, rationale, ownership burden, and deletion test. It records an
    initial `rejected | covered | promoted` disposition. Initial rejection or
    coverage is terminal; promotion leaves the terminal field unset until later
    review resolves it. Parallel reviewers may not mutate shared ledgers; one
    integration owner merges packets. Candidates recommended for acceptance
    remain promoted until Task 4 creates and reads back their remediation issue.
  - Verification: every group has an initial disposition; terminal rejections
    and coverage cite evidence; promoted packets are complete for Task 4;
    #1178/#1248 and current #1227 records are cited where relevant; high-signal
    rejections retain enough evidence to prevent repeat investigation.

- [ ] Task 4: Publish accepted findings and the exhaustive completion record.
  - Contract: each accepted group becomes one non-overlapping milestone-17 issue
    using `docs/codebase-audits.md` fields, explicit type/labels/priority/native
    coordination, and #1224 provenance. After issue readback, the integration
    owner records terminal `accepted` plus its link. The final #1224 comment
    links the manifest and amendments; records census totals, every query and
    raw-match count, overlap table, exact definitions/references, all terminal
    dispositions, high-signal rejection rationale, rankings, and remediation
    links; and states that production code did not change.
  - Verification: duplicate search precedes issue creation. Within the
    initially-promoted cohort, transition arithmetic proves
    `promoted = terminal rejected + terminal covered + terminal accepted`, with
    no unresolved promotion. Tracker readback confirms every issue body/metadata
    field and every named final-comment section. A final tracked diff/status
    against the audited baseline permits only approved spec/outline archival and
    proves production/test/schema/migration/runtime/generated-audit paths stayed
    unchanged or ephemeral before publishing the no-production-change assertion.

## Risk checks

- No semantic candidate inspection occurs before the manifest comment is
  published and read back against the committed audited SHA.
- Query failure or unavailable analysis is never interpreted as a clean result;
  a finite manifested fallback and complete rerun are required.
- Parallel work partitions immutable candidate packets; only the integration
  owner writes reconciliation and final-disposition ledgers.
- SQLite/PostgreSQL similarity is treated as a candidate, not equivalence;
  ADR-0019/0021 differences are preserved.
- Domain ownership changes and intentional protocol, trust, error, target, and
  timestamp representations are not mislabeled as compensating churn.
- No generated audit output or speculative ledger enters Git; only the approved
  spec and outline are archived when the issue ships.
