# Profile Field Validation Case Table

Issue: #1275 Status: Approved

## Outcome

The shared typed client-validation behavior for over-length Profile fields is
covered by one independently named Playwright registration per field from a
local case table. Persistence and clearing contracts remain separate tests.

## Load-bearing decisions

- Each row carries the field phrase used in the test title, its selector, and
  the complete invalid value submitted by the browser.
- The rows cover display name with 256 characters and bio with 1001 characters,
  preserving the current first-invalid boundary inputs.
- Generated titles preserve the existing
  `over-long <field> shows an inline error and gates submit` diagnostics.
- Every generated registration uses the same Profile-page readiness, fill, blur,
  visible-inline-error, and disabled-Update-Profile flow.
- The error contract remains visibility of `SEL.error`; exact field-specific
  message text is not added.
- The observable boundary remains the typed direct-bind Profile UI governed by
  ADR-0065.

## Acceptance

- `profile.spec.ts` contains one explicit local validation-case table for the
  display-name and bio over-length cases.
- Exactly one Playwright test is registered per row, with the affected field
  named in each title.
- Every generated test opens the registered `/profile` page, fills and blurs the
  row's field, then proves an inline error is visible and Update Profile is
  disabled.
- The two copied over-length validation bodies are deleted.
- DisplayName and Bio persistence and clear flows, default-format behavior, and
  unrelated Profile tests remain separate and unchanged.
- Focused `profile.spec.ts` evidence passes with both generated cases
  independently reported.
- The normal CI E2E matrix continues to exercise the cases across
  SQLite/PostgreSQL and Chromium/Firefox.

## Boundaries

- Do not change Profile production code, domain validation, limits, selectors,
  server functions, routes, or public APIs.
- Do not add shared helpers, cross-file fixtures, dependencies, or an ADR.
- Do not consolidate persistence or clearing flows, revisit #1223's rejected
  flag-driven design, or alter the backend/browser matrix.
