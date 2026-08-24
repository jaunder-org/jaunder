# Issue #735: Compact composer summary validation coverage

## Outcome

Add end-to-end coverage for the `/app` inline composer’s compact summary field.
The browser spec proves that an over-long optional Post summary shows the shared
inline validation error and disables both compact compose actions.

## Load-bearing decisions

- The covered surface is `/app`, not `/posts/new`; `/app` is the route that
  renders `InlineComposer` with `compact=true`.
- The test must exercise the compact composer’s `textarea[name="summary"]`,
  selected through the existing shared `SEL.postSummary` selector.
- The body field is filled first so the submit gate being tested is summary
  validity, not the empty-body guard.
- The invalid value is 501 characters, one over `MAX_POST_SUMMARY_CHARS`; this
  preserves the existing PostSummary validation contract from ADR-0065.
- Both compact action buttons are part of the contract: Save draft
  (`button[name="publish"][value="false"]`) and Publish
  (`button[name="publish"][value="true"]`) must be disabled while the touched
  summary field is invalid.
- No production behavior change is intended; this is coverage for an existing
  browser validation path.

## Acceptance

- An e2e spec navigates to `/app` as an authenticated user and drives the
  compact composer summary field.
- The spec fills a valid body, enters a 501-character summary, blurs the summary
  field, and observes the inline error.
- The spec asserts both compact submit buttons are disabled.
- Removing the compact composer’s `summary_field.is_valid()` guard from either
  button makes the new spec fail.
- Focused proof runs with `devtool run -- cargo xtask e2e-local <target>` for
  the changed spec.

## Boundaries

- Do not redesign composer validation, selectors, or submit-gate plumbing.
- Do not add coverage for the non-compact `/posts/new` composer; existing tests
  already cover that surface.
- Do not add a new validation rule or change `MAX_POST_SUMMARY_CHARS`.
- Do not broaden the issue into publish-success, draft-save-success, or
  persistence coverage for `/app`.
