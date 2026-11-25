# Issue #2113 — editor blanks after a confirmed save, so the "View post" link moves under the click

## Outcome

After a confirmed save on the Edit Post page, the editor stays painted. The save
summary's "View post" link stays where it is, and a click on it navigates to the
permalink. The `posts.spec.ts` draft-lifecycle test stops ejecting PRs.

## Root cause (from the retained trace, run 45678119946, `postgres-firefox-3-of-5`)

1. Every confirmed save sets `EditLifecycleState::confirmed_draft`. Signal `set`
   does not check equality, so this rebuilds `EditPostForm` inside
   `EditPostPage`'s `<Suspense>` even when the post was already a Draft.
2. The rebuilt form mounts a new `ComposerMediaDisclosure`. It creates a fresh
   `get_uploads_enabled` `Resource` and reads it with `.get()`. A resource read
   inside a `<Suspense>` with no value yet suspends the **whole** boundary. The
   editor is replaced by "Loading…" until the refetch returns. That took 165 ms
   in the trace.
3. Normally `.j-compose-grid` is `flex: 1` inside the fixed-height `.j-main`,
   which pins the save summary (rendered after the `<Suspense>`) to the bottom.
   While the "Loading…" fallback replaced the grid, the summary sat near the top
   of the page. Playwright found the link there and pressed at (284, 229). The
   press focused the link: the failure snapshot shows it `[active]`. Then the
   refetch resolved, the editor repainted, and the link moved to y≈676. The
   mouse-up therefore landed on another element, so no `click` event reached the
   anchor. The router did nothing and the browser did nothing. No request
   follows the click in the trace, not even the session check every in-app move
   makes. `waitForURL` then waited out the test budget.

The test's waits were correct. The fault is in the app: a confirmed save blanks
the whole editor and moves its controls under the user's pointer.

## Approach and tradeoffs

Move the uploads-enabled resource out of the component that gets rebuilt.
`ComposerMediaDisclosure` stops creating its own `get_uploads_enabled` resource.
It receives the resource as a prop, passed through `ComposerControlRail`.

- **`EditPostPage`** creates the resource once at page level, outside the
  rebuild closure, next to `post` and `current_audience`. Its first paint still
  waits for the flag, as it does today. A later rebuild reads the resolved
  value, so it neither refetches nor suspends. The editor, the rail and the save
  summary all stay where they are.
- **`PostCreateForm`** (`/posts/new` and Home's inline composer) creates the
  resource at the top of the form. Its rail is never rebuilt, so its behaviour
  does not change.

Alternatives considered:

- **A local `<Suspense>` around the media read.** It is a smaller diff, but
  Media is the first, full-width control in the rail. It would appear late on
  first paint and disappear and come back on every rebuild. Each time, Format,
  Slug, Publish and Share-with would move under the pointer: the same class of
  failure. Existing tests click those controls right after a rebuild
  (`posts.spec.ts:1964`, `:1986`).
- **Memoise `current_publication`** so a Draft→Draft save does not rebuild. The
  unpublish path (Live→Draft) would still rebuild and blank the editor.
- **Accepted cost:** one extra prop on `ComposerControlRail` and its two
  callers. The rebuild itself remains, which is out of scope.

## Acceptance

- New e2e regression test in `posts.spec.ts`: open an existing Draft's editor,
  then stall `media/get_uploads_enabled` (`stallServerFn`). Save the Draft. Once
  the save summary appears, assert that the body textarea and the Media control
  are still visible. Then release the stall, and check that clicking "View post"
  navigates (`navigateInApp`) to the permalink. Run it before the fix: it must
  fail with the editor replaced by "Loading…". Run it after the fix: it must
  pass. Proof lane: `dev e2e-local posts.spec.ts:<line>` in Chromium and
  Firefox.
- The existing "draft lifecycle: create, view, edit, and publish" test passes
  unchanged under `dev e2e-local`.
- No sleeps, no larger timeouts and no added retries (jaunder-e2e wait
  discipline).

## Boundaries

- No change to `navigateInApp`, the duration-pressure gate or test budgets.
- No change to how `EditPostPage` rebuilds on lifecycle changes. The loss of the
  composer's temporary media rows on a rebuild was not reported, and is not
  addressed here.
