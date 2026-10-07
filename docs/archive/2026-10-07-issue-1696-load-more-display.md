# Keep timeline “Load more” on one line (#1696)

## Outcome

The timeline continuation label “Load more” displays on one line at mobile and
desktop widths. This is a presentation-only correction: activating the control
continues to load the next page exactly as it does today.

## Load-bearing decisions

- Preserve the existing label, link-like appearance, alignment, and spacing.
- Give the control sufficient room for its label rather than stacking its words.
- Apply the correction to the shared timeline continuation presentation used by
  Local, author timelines, tag timelines, and Home.
- Preserve the semantic button, accessible name, keyboard activation, focus
  indicator, hover treatment, and disabled/loading presentation.
- Leave cursor handling, page size, ordering, fetching, visibility, and append
  behavior unchanged, consistent with ADR-0004.
- Keep the existing Style Contract continuation hook and theme ownership intact
  (ADR-0184). Arbitrary Theme Package overrides are not a sizing guarantee.

## Acceptance

- A browser regression assertion demonstrates that the rendered “Load more” text
  occupies one line and fits within its control at a narrow mobile viewport (390
  × 844) and a desktop viewport (1280 × 900).
- Exercise Local signed out and Home signed in with enough deterministic Posts
  to expose the continuation; both use the built-in Studio presentation.
- Capture comparable Before/After screenshots for those two routes at both
  viewports, with the continuation visible and the same data and authentication.
- The final screenshots retain the control’s existing placement and visual
  treatment, with no clipping or new overflow caused by the correction.
- Existing pagination tests remain valid; focused browser proof verifies the
  control still appends Posts and disappears when the timeline is exhausted.
- Inspection of the diff confirms no changes to pagination or server behavior.

## Boundaries

- No redesign of timelines, Posts, navigation, or other buttons.
- No changes to draft/history pagination controls unless the shared timeline
  styling directly applies to them.
- No general mobile overflow remediation or image-scaling changes.
- No asset-cache invalidation work; that is tracked separately in #1695.
- No new theme policy, API, storage state, or committed screenshot baselines.
