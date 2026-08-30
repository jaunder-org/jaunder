# Interface Friction and Representation Churn Audit

## Outcome

Jaunder completes a read-only, exhaustive audit of interface friction and
representation churn across crate, tier, protocol, storage, web, TypeScript, and
Elisp seams. Every match returned by the declared search suite receives semantic
review and a durable disposition; accepted findings become separate,
evidence-backed remediation issues in milestone 17.

## Load-bearing decisions

- The audit is exhaustive over a finite, recorded search suite, not an undefined
  notion of every possible conversion. The final record names every
  query/pattern, tool, source root, raw-match count, candidate grouping, and
  disposition.
- Before inspection, #1224 receives a frozen search-manifest comment pinned to
  the exact audited commit. It enumerates tracked roots, stable query IDs, exact
  commands/patterns/options, tool versions, grouping rules, and required
  fallback populations. An amendment is a versioned follow-up comment and
  requires the complete amended suite to be rerun.
- Every raw match is inspected. A raw-match reconciliation unit is one stable
  `(query_id, normalized location or symbol)` occurrence; per-query totals
  reconcile independently. Candidate groups may span queries only when matches
  cross the same seam with identical ownership, callers, evidence, and
  rationale. The final record retains a cross-query overlap table rather than
  deduplicating query coverage away.
- Search counts are indexes, never findings. Each candidate is traced through
  complete behavioral paths: definitions, callers, input/output ownership,
  invariants, representations, errors, tests, adapters, and relevant history.
  Discovery begins with a fresh `cargo xtask census --json`; generated
  census/search output remains ephemeral and is neither committed nor attached
  as a backlog.
- The declared suite covers Jaunder-owned tracked Rust, TypeScript, and Elisp
  source roots through these lenses:
  - **Conversions:** `From`/`TryFrom`/`Into`, parsing/deserialization followed
    by conversion, immediate pre/post-call conversions, DTO/domain/wire/storage
    translations, and equivalent-representation round trips.
  - **Serialization:** repeated JSON, XML, AtomPub, Syndication, storage-row,
    and wire projections, including source/rendered and trusted/untrusted
    transitions.
  - **Collections:** boundary-adjacent `collect`, map/set/vector reconstruction,
    iterator materialization, repeated filtering/projection, and
    collection-to-collection transformations.
  - **Cloning/copying:** boundary-adjacent `clone`, `to_owned`, `to_vec`,
    `to_string`, and equivalent Elisp/TypeScript copying solely for an
    interface.
  - **Argument bundles:** functions/calls with four or more non-ambient business
    arguments, repeated smaller bundles at multiple callers, and repeated
    struct/tuple reconstruction at a seam; ADR-0129 excludes ambient context.
  - **Error mapping:** typed `From`/`map_err`/projection matches across domain,
    host/operator, web/public, protocol, storage, TypeScript, or Elisp seams.
  - **Call order:** cross-owner operation pairs/sequences at two or more
    callers, alternating calls across one seam, and caller orchestration that
    may encode an unowned invariant.
- Syntax/AST searches enumerate structural matches; language-server
  definitions/references establish ownership and fan-in; Git history supplies
  churn/co-change; complete module review determines intent. Unavailable
  supporting evidence is recorded. Inability to enumerate a required lens blocks
  completion until a finite fallback population is added to a versioned manifest
  and the full amended suite executes.
- Real ownership changes are not friction by themselves. Compensating churn
  repeatedly translates equivalent knowledge because an interface lives at the
  wrong seam. The binding deletion test asks whether removing a proposed module
  makes complexity reappear across callers (locality) or disappear (weightless).
- Authoritative constraints: Post versus AtomPub Entry, Syndication Feed versus
  AtomPub Collection, and outbound `feed_*` versus inbound `ajr_*` are distinct;
  ADR-0015/0023 require separate Syndication/AtomPub serialization and
  compatibility behavior; ADR-0016 forbids service locators and heterogeneous
  dependency bundles beyond composition roots; ADR-0017/0059 require typed
  domain/operator/public error tiers and intentional lossy security projection;
  ADR-0058/0159 govern `common`/`host` target-reachability; ADR-0019/0021
  preserve deliberate storage dialect and transaction differences; ADR-0063/0068
  govern newtype trust, invariant, identity, and presentation translations;
  ADR-0153 makes `UtcInstant` own web/storage-boundary instants, superseding
  ADR-0072's raw-storage timestamp exception; and ADR-0129 requires cohesive
  typed request aggregates, so arity alone is not a finding.
- Existing ownership is verified then excluded: #1248 owns `PublicationState`
  persistence projections and #1178 mechanical storage-row projections. New
  findings require materially distinct risks outside those boundaries. #1225
  retains repetitive test structure and #1226 dead/shallow modules. #1227
  retains only its durably selected storage-backed behavioral slice: cite that
  selection before classifying overlap as covered; until then, hold overlap for
  explicit tracker coordination rather than disposing it as covered.
- One remediation issue owns one seam and one maintenance or correctness risk;
  split findings when owning modules, migrations, deletion sets, or verification
  surfaces differ. Before filing, search open/closed issues and sibling audits
  for duplicate or competing ownership.
- Each remediation uses `docs/codebase-audits.md`'s Evidence, Problem, Current
  seam, Depth assessment, Proposed module, Migration, Deletion, Verification,
  and Confidence format; explicit metadata is milestone 17, `Task` for
  design/refactor or `Bug` only for confirmed incorrect behavior, topic labels,
  rubric priority, native dependencies/coordination where applicable, and #1224
  provenance. Read back every named field after creation.
- #1224 receives one exhaustive final completion comment linking the manifest
  and amendments, recording census totals, every query/count,
  candidate/raw-match coverage, overlap table, exact definitions/references,
  dispositions, high-signal rejection rationale, rankings, remediation links,
  and that no production code changed. Full generated outputs remain ephemeral.

## Acceptance

- The pre-inspection manifest and every amendment meet the recorded-suite and
  rerun requirements above; every declared lens has an executed, auditable
  population and may not finish unavailable before its finite fallback executes.
- Every raw-match reconciliation unit is accounted for exactly once in a
  candidate group, with independently reconciled per-query totals and the
  cross-query overlap table.
- Every candidate group records exact paths/symbols, inspected
  callers/errors/tests/history, current seam, and rationale. Its initial
  disposition is rejected, covered by an existing issue, or promoted to complete
  behavioral-slice review. Promotion is intermediate: every promoted candidate
  must finish rejected, covered by an existing issue, or accepted with a linked
  remediation issue.
- A promoted candidate identifies the caller knowledge or orchestration burden,
  distinguishes ownership-changing translation from compensating churn, and
  applies the deletion test. High-signal rejection retains enough evidence,
  governing ADR/domain distinction, and deletion-test reasoning to prevent
  repeat investigation.
- #1248 and #1178 are checked against adjacent candidates; covered matches link
  them and no competing remediation is filed. #1227's durable selection is cited
  before any covered classification; absent it, overlap is explicitly
  coordinated. No #1225/#1226 concern is filed here.
- SQLite and PostgreSQL paths are compared whenever a candidate crosses storage;
  intentional dialect/transaction differences are cited rather than treated as
  drift.
- Accepted findings are ranked by evidence-backed leverage, churn, fan-in, and
  semantic risk, or the final record states none were accepted. Each is a
  separate one-concern milestone-17 issue after duplicate search, using the
  prescribed format, explicit metadata, and tracker readback.
- The final comment links every remediation issue, reconciles all populations,
  records the required audit evidence and final dispositions, and states the
  audit changed no production code. Verification is reproducible census/search
  execution, source/reference/history evidence, reconciliation arithmetic, and
  tracker readback of issue bodies, metadata, and the final comment.

## Boundaries

- This issue performs discovery only. It does not edit production code, tests,
  schemas, migrations, runtime documentation, or public behavior.
- It does not automatically condemn conversion, cloning, collection
  materialization, argument count, error projection, serialization, or call
  sequencing.
- It does not reopen #1248 or #1178 without materially distinct evidence outside
  their scopes; it excludes #1227 only after its durable selection is cited.
- It does not create issues for #1225 repetitive tests or #1226 dead/shallow
  wrappers, a committed search dump, conversion ledger, speculative cleanup
  list, repository-wide score, or a finding quota. Exhaustive search may
  legitimately conclude that no new remediation is warranted.
