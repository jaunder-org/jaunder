# Discoverable permalinks for Posts without visible titles (#1725)

## Outcome

A Post without a visible title exposes a simple clickable `#` in the place where
a titled Post displays its permalink-linked heading. Users can reach its
canonical permalink without opening management controls.

## Load-bearing decisions

- Use the fallback whenever the Rendered Title is absent or an empty canonical
  fragment and the Post has a permalink. This includes an authored title whose
  sanitized projection has no visible text.
- Display exactly `#`; give the link the accessible name `Permalink`. Its
  destination is the Post's existing canonical permalink, not a fragment.
- Keep visible Rendered Titles and their existing linked/unlinked heading
  behavior unchanged. A Post without a visible title and without a permalink
  still omits the heading; do not manufacture a draft URL or empty heading.
- Apply the same presentation across every surface using shared Post article
  rendering, including Local, Home, author/tag timelines, and permalink pages.
  Anonymous projector and CSR markup must continue to coincide by construction.
- The fallback is a navigation affordance, not an authored or Rendered Title. Do
  not persist `#`, alter source titles or slugs, or change AtomPub, Syndication
  Feed, metadata, revision, or editing representations.
- Preserve existing Style Contract hooks and theme behavior; no CSS redesign,
  additional theme variants, or public protocol/schema changes.

## Acceptance

- Through shared Post article rendering, absent and empty Rendered Titles with a
  permalink produce one `#` link named `Permalink` to that exact URL.
- Visible titles retain their rendered inline markup and existing destinations;
  titleless/empty-title Posts without permalinks still omit the heading.
- Browser proof locates an untitled Post's fallback by its accessible name,
  follows it within the app, and verifies that the intended Post is displayed.
  Cover both public presentation and authenticated Home without relying on
  management controls; retain existing titled-Post behavior.
- Confirm projector/CSR coincidence and run focused host and browser regression
  proof before normal hook-backed gates and CI.
- Capture comparable Before/After pairs for Local and authenticated Home at
  1280×900 with Studio styling, deterministic titled and untitled Posts, and
  identical authentication/data state. Keep screenshots and execution evidence
  in transient run/session storage or the PR, never committed source.

## Boundaries

No new storage, endpoint, permalink-generation policy, management action, or
fallback source title. Tests and current presentation documentation are enduring
deliverables; this approved contract is retired after conformance review.
