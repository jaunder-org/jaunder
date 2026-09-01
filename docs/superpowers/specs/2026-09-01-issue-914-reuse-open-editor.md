# Issue #914: Reuse post navigation helpers

## Outcome

Post editor E2E flows use the shared post-navigation helpers instead of
repeating selector, permalink, post-ID extraction, and SPA navigation details in
`posts.spec.ts`.

## Load-bearing decisions

- Migrate every remaining inline permalink-to-editor flow on current `main`; all
  six sites named by the issue still remain.
- Use `followPermalink` for each compatible save-summary-to-permalink hop, not
  only `openEditor` for the following permalink-to-editor hop.
- Keep `postId` only in tests that use it after reaching the editor; helper
  return values are otherwise intentionally ignored.
- Preserve each test's assertions, readiness semantics, and SPA navigation
  behavior.
- Update comments that describe the removed inline navigation mechanics.
- The existing helpers remain the single owners of selectors, href validation,
  post-ID extraction, and navigation readiness.

## Acceptance

- `posts.spec.ts` contains no inline copy of the PostCard Edit-link selector,
  post-ID regex extraction, or its following editor navigation.
- Compatible save-summary permalink hops in the migrated flows call
  `followPermalink` rather than reconstructing the helper's behavior.
- The affected browser flows still reach the editor and retain their existing
  behavioral assertions.
- Focused Playwright execution covering every migrated flow passes.
- Repository static checks pass without new lint, formatting, or type errors.

## Boundaries

- Do not change production application behavior, selectors, or navigation
  helpers.
- Do not refactor unrelated post tests or navigation paths that do not form the
  migrated save-summary/permalink/editor flow.
- Do not add a new abstraction or broaden this work into general E2E navigation
  cleanup.
