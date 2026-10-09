# Reconciliation merge targets the current row (#1718)

## Outcome

In the Emacs Protocol Client's `jaunder-reconcile` report, pressing `e` opens a
two-way Ediff merge for the conflict row at point. The User does not need to
mark a row or select a region before merging.

## Load-bearing decisions

- Merge always targets the reconciliation row at point, independently of marked
  rows or an active region. It never falls back to selected rows.
- Point outside a reconciliation row produces an actionable user error without
  staging a Member or opening Ediff.
- Merge remains one Post at a time. Only an eligible, uniquely matched, reviewed
  `conflict` row may open a merge session.
- ADR-0211's local and remote revalidation, independent editable result,
  explicit completion, cancellation, and partial/unknown-outcome guarantees
  remain unchanged. Existing scratch work must not be overwritten.
- Bulk push, fetch, keep-local, keep-remote, and delete retain their existing
  mark/region selection and confirmation behavior.
- Preserve the existing callable merge command for compatibility; its documented
  targeting semantics change to the current row.

## Acceptance

- A conflict row at point opens merge without any marks or active region.
- Marks on another row, multiple marked rows, and an active region covering
  other rows do not redirect merge away from the row at point.
- With point outside row text, merge reports a user error and performs no Member
  staging or Ediff setup, even when another row is selected.
- An ineligible row at point cannot open Ediff or mutate either Post, even when
  a different eligible conflict row is marked.
- Regression tests exercise these cases through the interactive command;
  existing merge safety and bulk-selection tests remain green.
- Reconciliation help and the Emacs user guide accurately distinguish
  current-row merge from selected bulk actions.
- Execute focused Emacs regression proof and normal repository gates. Keep logs
  and review evidence in ignored run/session storage or the PR.

## Boundaries

No new conflict-resolution actions, batch merge, automatic publish, protocol or
storage change, or unrelated reconciliation redesign. No new ADR or domain term
is needed: this is a targeting refinement consistent with ADR-0211. The spec is
an in-flight contract, deleted after conformance review; tests and current user
documentation are the maintained deliverables.
