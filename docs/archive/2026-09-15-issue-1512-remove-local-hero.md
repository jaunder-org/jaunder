# Remove the Local hero

## Outcome

The public Local page no longer displays the large promotional hero headed “One
timeline. Every protocol.” The existing masthead, anonymous actions, and Local
timeline remain, allowing useful content to begin immediately below the
masthead.

Operator-controlled Local title and tagline work is separate and tracked by
#1524.

## Load-bearing decisions

- Remove the complete Local hero block, including its headline and explanatory
  paragraph.
- Do not replace the hero with alternate promotional copy, an empty wrapper, or
  reserved spacing.
- Keep the existing Local topbar and its Sign in and Register actions unchanged.
- Keep the Local timeline’s contents, ordering, pagination, public projection,
  and authenticated-root redirect behavior unchanged.
- Preserve projector/reactive coincidence by changing the shared Local masthead
  renderer rather than hiding the hero on only one rendering path.
- Update checks that use the hero as an observation anchor to observe an element
  that remains part of Local; do not weaken layout-shift or mount-stability
  coverage.

## Acceptance

- An anonymous visit to `/` renders no `.j-hero` element and none of the removed
  promotional copy.
- The Local topbar, Sign in action, Register action, and timeline remain visible
  and functional.
- The projected first paint and the mounted reactive Local view both omit the
  hero without a layout shift.
- Host rendering tests prove the shared Local masthead contains the retained
  topbar and actions but no hero.
- End-to-end coverage continues to prove the Local timeline and mount-stability
  behavior.

## Boundaries

- Do not change the current hard-coded Local title or description in this issue;
  #1524 owns operator-controlled `site.title` and `site.tagline` presentation.
- Do not change public-theme packaging or the Style Contract beyond removing the
  obsolete hero block.
- Do not change other route mastheads, Post presentation, authentication, or
  navigation policy.
