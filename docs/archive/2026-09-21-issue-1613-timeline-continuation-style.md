# Timeline continuation style

Issue: [#1613](https://github.com/jaunder-org/jaunder/issues/1613)

## Outcome

Every web Post timeline presents **Load more** as a deliberate, low-emphasis
text action aligned with the Post's outer padding edge, rather than as unstyled
browser button chrome. The control remains easy to identify and operate across
pointer, keyboard, loading, disabled, desktop, and compact-width use.

## Load-bearing decisions

- The treatment applies uniformly to the shared continuation control on Local,
  Home, User, site-tag, and User-tag Post timelines.
- The control remains a semantic `button`; this is a presentation correction,
  not a link, navigation, or pagination-model change.
- Studio presents the control as accent-colored text without a filled surface or
  conventional bordered-button box.
- The control's left edge aligns with the outer Post padding edge, before the
  avatar column—not with the body-text column after the avatar and grid gap. It
  has intentional vertical breathing room separating it from the final visible
  Post and the end of the timeline.
- Hover and keyboard focus remain clearly perceivable. Focus treatment must not
  rely on color alone.
- The existing disabled/loading behavior and labels remain **Load more** and
  **Loading…**.
- The projector placeholder and the mounted reactive control use the same
  geometry and presentation, preserving the no-reflow handoff.
- The existing `continuation` Style Contract concept remains the stable theming
  hook; this work does not add incidental wrapper structure to that contract.

## Acceptance

- Local, Home, User, site-tag, and User-tag timelines with another cursor page
  show the same low-emphasis continuation treatment.
- The control's left edge aligns with the outer Post padding edge at both
  desktop and compact widths, with visible space above, below, and to its left.
- The default browser button border, fill, and platform-specific bevel are not
  visible in Studio.
- Pointer hover, keyboard focus, loading, and disabled states are legible and do
  not shift the control's layout.
- Activating the control still appends the next cursor page; completing the
  final page still removes the control.
- Projected HTML and the mounted client retain a semantic `button` element and
  the existing `data-jaunder-part="continuation"` hook. The projected
  placeholder remains non-functional; pagination activation begins only after
  client mount.
- Comparable before/after visual proof shows Studio's public Local timeline with
  enough seeded Posts to expose the resting control at 1440×900 and 390×844.
- Focused browser proof exercises resting, pointer-hover, keyboard-focus,
  loading, and disabled presentation and verifies that loading/disabled state
  changes do not alter the control's outer dimensions.
- Focused automated coverage proves continuation markup and pagination behavior,
  and the repository's required checks pass.

## Boundaries

- No infinite scrolling, automatic loading, page-number navigation, cursor, or
  page-size change.
- No redesign of draft pagination, Post revision history, or other management
  list continuation controls.
- No wording, Post-card, timeline-order, custom-theme package, or Style Contract
  version change.
- No visual snapshot-baseline expansion; the transient before/after pair is the
  review evidence for this focused presentation fix.
