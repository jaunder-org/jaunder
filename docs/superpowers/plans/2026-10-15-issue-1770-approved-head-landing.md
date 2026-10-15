# Approved-head landing implementation outline (#1770)

## What will change

I will change `cargo xtask pr land` so it requests merging only the commit that
was checked and approved, instead of following whichever PR commit is current
when it reads again or retries.

The caller must ensure the initial commit is the approved one; this command
retains it for this invocation only and does not record human approval.

It will remember the initial commit ID, stop if the PR changes, and include
GitHub's supported commit-match condition in every merge request. Readback will
still verify that the request took effect on that same commit. This does not
lock the PR against future pushes or change how read-only `pr watch` works.

## Execution outline

- [x] Carry the initial commit ID through landing and its merge requests; apply
      the same requirement to the shared promoter caller using the expected
      commit ID it already holds. No unqualified merge-request path.
- [x] Reject changed commits before arming, accepting readback or retrying;
      preserve existing typed failures, check/ejection refusals, and the
      permitted single retry when the commit is unchanged.
- [x] Prove this with existing offline fake-source/arming/clock tests: A changes
      to B before arming or retrying; GitHub rejects a commit mismatch at the
      request boundary; unchanged A still lands through ordinary arm, direct
      enqueue or retry. Verify the production request includes the commit-match
      condition and retain observation-only/adverse-result regressions.

## Boundaries and gates

Execute with `jaunder-iterate` / `jaunder-dispatch` only after outline approval.
The trigger is the read/check/merge-request/retry race. Governing spec:
`../specs/2026-10-14-issue-1770-approved-head-landing.md`, approved as
JAU-M33-1770-SPEC-001. Revised request JAU-M33-1770-OUTLINE-001 remains pending;
the previous outline hash is superseded, not approved.

Use focused host xtask tests, not product `test-local` or live GitHub arming.
Offline tests prove our request/refusal behavior, not GitHub's internal queue
implementation. No new public outcomes, automation framework, CI/ruleset change,
approval revocation or weakened protections. Normal hook/prepush/CI gates stand.
Lead owns the later baseline/source-fit boundary at available pinned main
`08ea5b46173d83d6f9f06fde68e96362c73beb91`, preserving both contracts. This
revision makes no source changes, runs no tests and performs no integration.
