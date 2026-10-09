## Outcome

Keep the current source tree focused on maintained code, contracts,
documentation and executable protections rather than historical development
paperwork.

## Approved decisions

- Commit approved specs and necessary outlines before implementation; maintain
  them during work, then delete them before shipping rather than moving them to
  an archive. Non-squash history preserves their evolution.
- Keep point-in-time logs, traces, measurements, review packets and audit
  inventories in ignored run/session storage or explicitly retained external
  artifacts.
- Preserve ADRs, current guidance, live designs, regression tests and their
  maintained fixtures/baselines.
- Remove completed historical documents and evidence, repair consumers and
  references, retire the issue-58 historical ledger check without removing
  behavioral error-handling tests, and redirect production qualification reports
  away from docs.
- Update canonical development skills in agent-configuration; do not edit PR
  #1707's branch.

## Acceptance

- Completed artifacts no longer occupy the current tree; retained live designs
  and test fixtures remain intact.
- Documentation references resolve and current reusable procedures remain
  documented.
- Qualification output is ignored, with tested destination semantics and
  unchanged report validation/sanitization.
- Skills distinguish executed proof from committed deliverables, and enforce
  commit-then-delete planning documents.
- Focused tests and applicable commit gates pass; this cycle's committed spec is
  deleted in its final cleanup commit.

## Boundaries

No history rewriting, weakened product regression assertions, changes to other
active checkouts, automatic merge, or blanket deletion of live designs.
