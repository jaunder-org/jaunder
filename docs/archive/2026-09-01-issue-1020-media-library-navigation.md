# Issue #1020: Centralize media library navigation

## Problem

`end2end/tests/media.spec.ts` repeats the same in-app transition from the
mounted root page to the media library: click the `/media` sidebar link, wait
for the `/media` URL, and wait for the page-specific `Attach media` button. The
issue names three setup sites, one of which is the shared `attemptDelete` helper
used by three tests. The current tree also contains the identical transition in
the first media delete-guard test, so four current setup sites require the seam
for a clean cutover.

ADR-0111 requires one document load per Playwright `Page`, followed by in-app
movement with an honest content barrier.

## Decision

Add a spec-local helper in `end2end/tests/media.spec.ts`:

```ts
async function openMediaLibrary(page: Page): Promise<void> {
  await navigateInApp(page, () => click(page, "a[href='/media']"), {
    url: "/media",
    ready: "button:has-text('Attach media')",
  });
}
```

Import `navigateInApp` from `./navigate`. Replace the repeated click/readiness
sequences with `openMediaLibrary(page)` in:

- `ordinary media delete confirms and removes unreferenced item`;
- `media manage page is reachable via nav link`;
- `attemptDelete`, covering its three current delete-guard callers;
- `ordinary media delete confirms and refuses referenced item`, the current-tree
  duplicate not present in the original audit count.

Each caller retains its existing initial `goto(page, "/")`. The helper is only
called after that root document is mounted, never from an existing media-library
state.

## Preserved boundaries

Keep authentication, uploads, post creation, dialog handling, delete request
stalling, and all assertions at their current owning sites. Keep the nav-link
appearance test on `/`; it verifies only that the authenticated sidebar link
exists and must not enter the media library.

`navigateInApp` supplies both required barriers: the expected `/media` URL and a
visible media-library control that is absent from the root page before the
action. No raw `page.goto`, `waitForLoadState`, or mount wait is added.

## Verification

Run `devtool run -- cargo xtask e2e-local media.spec.ts`, then
`cargo xtask check`. Review the final diff against repository standards, the
issue, ADR-0111, and this specification.

## Non-goals

- Moving the helper into shared e2e infrastructure.
- Changing media-library behavior, selectors, delete flows, navigation
  components, or public/test interfaces.
- Migrating tests that do not transition into `/media` from the mounted root
  page.
