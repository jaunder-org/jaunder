# Stylesheet Delivery Case Table

Issue: #1274 Status: Approved

## Outcome

The public built-in stylesheet delivery contract is expressed by one named
Playwright registration per stylesheet from a single explicit local inventory.
Each registration independently proves the stylesheet is available with the
expected CSS media type.

## Load-bearing decisions

- The inventory explicitly names `jaunder.css` and `jaunder-themes.css`; it is
  not derived from production internals or filesystem discovery.
- Each filename appears in its generated test title so failures remain
  independently attributable to the affected public asset.
- Every generated registration uses the same request and assertion body.
- Status remains exactly `200`.
- The `content-type` assertion continues to require that the header contains
  `text/css`, preserving tolerance for valid parameters.
- The observable boundary remains Playwright's request client against the public
  `/style/<filename>` route.

## Acceptance

- `static-assets.spec.ts` contains one explicit local inventory of the two
  built-in stylesheet filenames.
- Exactly one Playwright test is registered for each inventory entry, and each
  title identifies its filename.
- Each generated test requests its filename through `/style/`, asserts status
  `200`, and asserts that `content-type` contains `text/css`.
- The copied request and assertion bodies from the two former registrations are
  deleted.
- Focused `static-assets.spec.ts` evidence passes, with both assets reported
  independently.
- The normal CI E2E matrix continues to exercise the generated registrations
  across SQLite/PostgreSQL and Chromium/Firefox.

## Boundaries

- Do not change stylesheet contents, embedding, handlers, routes, cache
  behavior, MIME behavior, or production APIs.
- Do not add shared helpers, cross-file fixtures, dynamic asset discovery,
  dependencies, or an ADR.
- Do not refactor unrelated E2E tests or alter the backend/browser matrix.
