# Implement terse successful e2e-goto-wrapper output

- Issue: [#1041](https://github.com/jaunder-org/jaunder/issues/1041)
- Spec:
  [`2026-08-14-issue-1041-terse-gate-output.md`](../specs/2026-08-14-issue-1041-terse-gate-output.md)

## Tasks

- [x] Separate step-result rendering from the source audit.
- [x] Return a detail-free `StepResult` for a clean audit.
- [x] Preserve the derived census in failure detail.
- [x] Add regression coverage for the clean result.
- [x] Amend ADR-0094's census-rendering consequence.
- [x] Run focused tests and the pre-commit gate.
