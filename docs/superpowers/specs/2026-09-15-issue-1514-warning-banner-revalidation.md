# Immediate warning-banner revalidation

Issue: [#1514](https://github.com/jaunder-org/jaunder/issues/1514)

## Outcome

The authenticated shell's site-base-URL and backup warnings reflect newly saved
configuration immediately. Operators no longer need to navigate or reload to
know whether a confirmed settings change resolved or recreated a warning.

## Load-bearing decisions

- Each warning remains a projection of its persisted server-side condition:
  whether the site base URL or backup destination is absent.
- A confirmed site-identity save re-evaluates the site warning. A confirmed
  backup-settings save independently re-evaluates the backup warning.
- Re-evaluation reads the persisted condition; a save never blindly hides or
  shows a warning. Saving an unrelated field while the warning condition remains
  unresolved leaves the warning visible.
- Clearing a previously configured base URL or backup destination makes the
  corresponding warning reappear after the confirmed save.
- Confirmed and commit-indeterminate mutation outcomes both request
  re-evaluation, as required by the repository's mutation-outcome contract. A
  commit-indeterminate result remains visibly error-like and never asserts a
  banner state directly; the subsequent persisted read determines whether the
  warning is present. Rejected or rollback-confirmed failures preserve the prior
  warning state without re-evaluation.
- The existing soft authorization behavior remains unchanged: operators see
  applicable warnings, while non-operators and stale cookie-only sessions do not
  gain an authorization error surface.
- Site and backup warning refreshes are independent; changing one configuration
  does not perturb the other banner.

## Acceptance

- Starting with no site base URL, saving a valid base URL and receiving
  confirmed success hides the site warning without navigation, reload, or a
  second boot.
- Clearing that base URL and receiving confirmed success makes the site warning
  visible again without navigation or reload.
- Saving only the site title while the base URL remains absent leaves the site
  warning visible.
- Starting with no backup destination, saving a valid destination and receiving
  confirmed success hides the backup warning without navigation or reload.
- Clearing that destination and receiving confirmed success makes the backup
  warning visible again without navigation or reload.
- Saving backup schedule, retention, or mode while the destination remains
  absent leaves the backup warning visible.
- Automated browser proof exercises both hide-and-reappear lifecycles in the
  mounted authenticated shell and does not use navigation as the refresh
  mechanism.
- Focused logic coverage proves that confirmed and commit-indeterminate site
  outcomes invalidate only the site-warning resource, and the same backup
  outcomes invalidate only the backup-warning resource. Rejected or
  rollback-confirmed failures invalidate neither.

## Boundaries

- No changes to warning copy, links, visual styling, sticky placement, or the
  settings-form layouts.
- No changes to site or backup storage semantics, validation, authorization, or
  server API contracts.
- No general background polling or application-wide data-cache design.
- No new warning categories and no coupling between site and backup settings.
