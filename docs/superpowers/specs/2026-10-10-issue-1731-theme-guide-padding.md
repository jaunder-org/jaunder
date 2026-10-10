# Match theme-authoring guidance card padding (#1731)

## Outcome

The “Author themes from a repository” section on `/themes` has comfortable
internal spacing consistent with its neighboring private Studio cards. Its
heading, explanation, and guide link no longer sit against the card border.

## Load-bearing decisions

- Use the existing private Studio card spacing conventions rather than introduce
  a new spacing scale or redesign the Themes page.
- Keep the section heading, explanatory text, guide link text, destination, and
  new-tab behavior unchanged.
- Preserve the section's accessible name and heading structure.
- Apply the correction at desktop and narrow-screen widths; explanatory text
  wraps inside the padded card without causing page-level horizontal overflow.
- Keep public Theme Package CSS isolated from the private Studio surface, as
  required by ADR-0184. This presentation fix does not alter ADR-0197's
  repository-authoring contract.

## Acceptance

- At 1280 × 800 and 390 × 844 viewports, all guidance content has nonzero
  internal insets consistent with equivalent neighboring card content.
- The existing guide link remains visible and usable with its unchanged URL and
  new-tab behavior; the accessible section and heading remain intact.
- Narrow-screen text remains within the card and the page does not gain
  horizontal overflow from this section.
- Browser regression coverage checks the guidance spacing and retained guide
  link contract at both viewport sizes through the rendered `/themes` surface.
- Run the focused local browser proof covering the changed theme-management
  flow; retain the existing accessibility/private-Studio assertions.
- Capture comparable Before/After screenshots of `/themes` at both viewport
  sizes, with the same signed-in owner, empty author catalog, and Studio
  styling.
- Keep screenshots and execution evidence transient; present both pairs for
  review without adding canonical visual snapshot variants.

## Boundaries

- No unrelated page spacing, typography, wording, catalog, or import changes.
- No API, storage, authorization, or public-theme behavior changes.
- No new ADR or domain terminology is needed for this reversible correction.
- Commit this approved working spec before implementation and retire it after
  conformance review, retaining its history.
