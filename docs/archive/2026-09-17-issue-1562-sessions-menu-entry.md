# Sessions menu entry

Issue: [#1562](https://github.com/jaunder-org/jaunder/issues/1562)

## Outcome

Authenticated Users can reach the existing Sessions page from Jaunder's primary
sidebar navigation instead of manually entering `/sessions`.

## Load-bearing decisions

- The navigation label is **Sessions**.
- Sessions is an authenticated destination; it is absent from anonymous
  navigation.
- The entry appears immediately before **Passkeys**, grouping the two
  account-security destinations without introducing a new menu section.
- The entry uses the existing cog icon, matching Passkeys and the sidebar's
  established account-settings treatment rather than introducing a new glyph.
- Activating the entry navigates within the existing SPA to `/sessions` and
  marks Sessions as the active sidebar destination.
- The existing Sessions page and its behavior remain unchanged.

## Acceptance

- An authenticated sidebar includes a **Sessions** link with the cog icon
  immediately before **Passkeys**.
- Selecting **Sessions** reaches `/sessions` without a full document load and
  renders the existing Sessions page.
- The Sessions entry has the sidebar's active treatment while `/sessions` is
  current.
- Anonymous navigation does not expose Sessions.
- Automated coverage pins the catalog destination, authenticated visibility,
  active-route matching, and browser navigation behavior.
- Before-and-after screenshots of the authenticated `/app` sidebar at a stable
  desktop viewport demonstrate the added entry without unrelated presentation
  changes.

## Boundaries

- Do not redesign the Sessions page or change Session/App Password behavior.
- Do not reorganize other sidebar destinations or introduce a new navigation
  grouping system.
- Do not expose Sessions to signed-out visitors or change authorization rules.
- Do not add a new architectural decision; this extends the existing sidebar
  catalog and SPA-navigation conventions.
