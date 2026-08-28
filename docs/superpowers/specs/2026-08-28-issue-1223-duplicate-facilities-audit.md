# Duplicate Facilities and Implementation Drift Audit

## Outcome

Jaunder completes a read-only repository audit that distinguishes actionable
duplicate facilities or implementation drift from intentional repetition.
Accepted findings become separate, evidence-backed remediation issues in
milestone 17; high-signal rejected candidates receive durable rationale on issue
#1223.

## Load-bearing decisions

- Discovery begins with a fresh `cargo xtask census --json` run over the
  branch's unchanged source tree. Generated census output remains ephemeral and
  is neither committed nor attached as a backlog.
- The audit triages every displayed structural-clone candidate across Rust,
  TypeScript, and Elisp.
- Triage is a funnel: complete-module review is reserved for candidates
  strengthened by churn, fan-in, adapter correspondence, or semantic risk.
  Low-signal candidates may be rejected after enough inspection to state why
  they are not one facility or drift risk.
- The audit separately searches for independently named facilities with
  overlapping behavior because structural clones cannot reveal differently
  shaped implementations of the same policy.
- Candidate review follows complete modules and callers: owned behavior,
  interface, invariants, representations, errors, tests, history, and every
  affected adapter.
- Similar syntax is not a finding. A finding must identify a concrete
  maintenance or correctness risk and pass the deletion test from
  `docs/codebase-audits.md`.
- Domain distinctions in `CONTEXT.md` and accepted ADR constraints are
  authoritative. In particular, outbound `feed_*` and inbound `ajr_*`, Post and
  AtomPub Entry, and intentionally different protocol/storage representations
  are not duplicates.
- SQLite/PostgreSQL correspondence is a review aid, never proof of equivalence.
  ADR-0019's generic-store/narrow-dialect model, ADR-0021's transaction
  discipline, ADR-0053's test-homing rule, and the backend-specific backup
  carve-out govern consolidation decisions.
- Shared facilities must preserve ADR-0016 dependency injection and ADR-0058
  crate layering; the audit must not recommend a service locator or omnibus
  shared module.
- One accepted remediation issue owns one seam and one maintenance or
  correctness risk. Split a cluster when it requires different owning modules,
  migrations, deletion sets, or verification surfaces.
- Every remediation issue uses the finding format from
  `docs/codebase-audits.md`: Evidence, Problem, Current seam, Depth assessment,
  Proposed module, Migration, Deletion, Verification, and Confidence. It also
  states scope, exclusions, coordination with sibling audit issues, and
  provenance from #1223.
- Before creating a remediation issue, search open issues and sibling audits
  #1224–#1227 to avoid duplicates or competing ownership.
- Issue #1223 receives one final completion comment. It links accepted
  remediation issues and records concise rationale for rejected high-signal
  candidates when that rationale prevents repeat investigation.
- Persisted census evidence is limited to section totals, candidate identities,
  exact paths/symbols, and conclusions. The full generated JSON is not
  persisted.

## Acceptance

- The final audit record includes the census section totals and identifies every
  displayed Rust, TypeScript, and Elisp structural-clone candidate as rejected
  or promoted to deeper review. Each disposition names the exact paths, evidence
  inspected, and concise rationale; candidates may share one disposition only
  when their inspected evidence and rationale are genuinely identical.
- Every promoted candidate is reviewed through complete modules and callers,
  with leverage, churn, fan-in, semantic risk, and relevant history stated from
  evidence rather than inferred from similarity alone.
- Accepted findings are ranked in the final audit record by evidence-backed
  leverage, churn, fan-in, and semantic risk, or the record explicitly states
  that no findings were accepted.
- The independently named facility search records its behavior lenses and
  queries, covering duplicated policies, validation rules, projections, state
  transitions, and error mappings. It enumerates resulting candidate pairs and
  gives each pair a disposition with exact definition/reference evidence.
- Corresponding SQLite/PostgreSQL implementations are compared whenever a
  promoted candidate touches storage; intentional dialect or transaction
  differences are cited rather than treated as drift.
- Every accepted finding is filed as a separate, one-concern milestone-17 issue
  in the prescribed finding format, after duplicate search and metadata
  readback.
- Every rejected high-signal candidate recorded on #1223 names the candidate,
  evidence inspected, rejection reason, and governing ADR or domain distinction
  when applicable.
- The final #1223 comment links every remediation issue, summarizes persisted
  census evidence, and explicitly states that the audit changed no production
  code.
- Verification consists of reproducible census generation, exact
  source/reference/history evidence for conclusions, and tracker readback
  confirming issue bodies, milestone, type, labels, priority, dependencies, and
  final audit comment.

## Boundaries

- This issue performs discovery only. It does not edit production code, tests,
  schemas, migrations, or runtime documentation unrelated to the audit record.
- It does not consolidate facilities, repair drift, or prove backend behavioral
  parity.
- It does not create an issue for a metric, clone, large file, or adapter
  difference without a concrete risk and proposed owning seam.
- It does not create a committed census snapshot, rejection ledger, speculative
  cleanup list, or repository-wide score.
- It does not absorb the interface-friction, repetitive-test,
  dead-code/shallow-module, or bounded storage-slice audits owned by
  #1224–#1227.
