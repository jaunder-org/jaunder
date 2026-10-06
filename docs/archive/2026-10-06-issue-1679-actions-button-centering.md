# Issue #1679: Center the Post Actions label

Issue: <https://github.com/jaunder-org/jaunder/issues/1679>

## Outcome

The visible `Actions` label is horizontally and vertically centered inside its
button outline. The compact Post control keeps its existing appearance and
behavior; surrounding controls are not redesigned.

## Load-bearing decisions

- Retain the exact `Actions` label, existing typography, colors, border, radius,
  and 72 × 32 CSS-pixel button footprint. Internal spacing may change only as
  needed to center the label without clipping it.
- Preserve the button's association with its own Post and protected in-flow
  slot. Owner enhancement must not shift Post content.
- Preserve ADR-0188's trusted sibling, Theme Package isolation, CSS-anchor
  association, and supported-browser floor. Do not introduce JavaScript
  positioning or a compatibility fallback.
- Preserve native popover opening/dismissal, menu contents, link destinations,
  keyboard interaction, focus restoration, and server authorization.
- Anonymous and non-owner viewers gain no mutation controls.

## Acceptance

- A browser regression assertion measures the actual visible label against the
  button outline in both axes, with at most 1 CSS pixel of center deviation. It
  fails on the reproduced pre-fix symptom and passes after the fix; checking a
  CSS declaration alone is insufficient evidence.
- Existing Post Actions coverage remains effective for Home (`/app`), the author
  timeline (`/~<username>`), site-tag timeline (`/tags/<tag>`), and a Post
  permalink. Authenticated root visits retain their Home redirect.
- Desktop (1280 × 720) and narrow (375 × 800) layouts retain the 72 × 32
  footprint, Post association, viewport containment, and readable label.
- Existing menu interaction, anonymous absence, hostile Theme Package isolation,
  and zero-shift owner-enhancement proofs remain intact. Any existing alignment
  assertion revised to reflect correct centering must retain proof of
  button/slot coincidence rather than weaken placement coverage.
- Comparable before/after screenshots show an owned published Post on its
  permalink at both viewports, with the menu closed, the same seeded content,
  authentication, and built-in theme. Present the pairs for human judgment.
- Run focused Post Actions and owner-enhancement no-shift browser proofs.
  Required Chromium/Firefox CI remains the cross-browser authority; record any
  unavailable Safari-specific evidence honestly, without claiming it.

## Boundaries

No menu redesign, general button restyling, theme contract expansion, browser
support change, storage/API change, or unrelated image-scaling work. Diagnose
and reproduce the reported centering defect before changing presentation code.
