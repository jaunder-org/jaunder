# Issue #1784: Baseline upload contract without incidental patch coupling

Status: draft; pending cold soundness review and qualified human approval.
Issue: https://github.com/jaunder-org/jaunder/issues/1784

## Outcome

A compatible upload-artifact patch update does not fail the
host-resource-baseline workflow contract test solely because its dependency ref
changed. The test still rejects missing or incorrect baseline capture/upload
behavior before heavy work.

## Load-bearing decisions

- `.github/workflows/ci.yml` owns the upload dependency ref. Require the exact
  `actions/upload-artifact` action identity and an explicit nonempty ref, not a
  duplicated patch pin. This is not certification of arbitrary upstream ref
  compatibility; dependency review and CI retain that responsibility.
- Preserve setup < capture < upload < heavy host validation. Runner-level ENOSPC
  can prevent even `always()` post-validation uploads; early upload is
  essential.
- Preserve capture id `host-resource-baseline`, upload condition
  `always() && steps.host-resource-baseline.outcome != 'skipped'`, artifact name
  `validate-host-resource-baseline`, path `.xtask/host-resource-baseline.txt`,
  missing-files policy `error`, and retention of 14 days.
- Keep the existing real CI YAML consumer and real capture-script execution and
  evidence assertions in `xtask/src/gate.rs`. A small test-local contract seam
  shared by that consumer and minimally mutated YAML is the chosen testing seam;
  do not expand shared CI types or introduce a fixture framework.
- Scope is the test contract, not a workflow dependency bump. No new
  architecture or ubiquitous-language decision is required; ADR-0028 and
  ADR-0141 retain the host-only xtask and standalone-workspace testing
  boundaries.

## Acceptance

- Parameterized positive controls accept baseline upload refs v7.0.1 and v7.0.2
  through the same contract check used for the checked-in workflow, changing
  only that step's ref. The patch-bump case demonstrably fails before the
  correction and passes afterward; current main alone is not local red proof.
- Minimal negative controls reject a missing upload step or `uses`, wrong action
  identity, absent/empty ref, and each violated setup/capture/upload/validation
  ordering edge. A negative verdict must be attributable to its changed field.
- Controls reject changes to the capture id, upload condition, artifact name,
  path, missing-files policy, and retention; preserve existing script checks for
  successful/missing evidence and resource totals, inodes, and memory signals.
- Focused standalone xtask host tests pass, including the original real-workflow
  and script consumer. Ordinary hook and PR CI proof remains required; focused
  tests do not replace broad gates or authorize merge.
- Review can judge the delivered diff without committed logs: retain red/green
  commands, revision, outcomes, and sanitized excerpts in ignored run storage.
  Commit this approved working spec before implementation; maintain it through
  conformance review and delete it at final cleanup per CONTRIBUTING.

## Boundaries and evidence limits

No live Actions mutation, new fixture infrastructure, CI topology change,
application/storage/browser behavior change, upstream action compatibility
audit, or lint/coverage/test waiver. Preserve the assigned branch and checkout;
no outline trigger is present for this localized deterministic test correction.

Accepted intake records remote PR1782 host CI failing at `src/gate.rs:1072`
(v7.0.2 versus v7.0.1), not a locally executed reproduction. PR1782 head was
`c80fa8adc2a8abf988f1ef5fb5d6989a4f4c968a`; reported PR2914 remains unresolved
and is not asserted to be PR1782. No local test/build proof exists yet.
