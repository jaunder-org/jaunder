# Collapse the Home composer while scrolling

## Outcome

Home starts with its inline Post composer expanded, then yields vertical space
to the timeline by collapsing the composer after the User begins scrolling down.
The compact state remains an obvious, accessible entry point for creating a Post
and never discards in-progress work.

## Load-bearing decisions

- The behavior belongs only to Home (`/app`); the dedicated new-Post route keeps
  its existing full-page composer.
- Each fresh Home mount starts with the composer expanded. Collapsed state is
  local presentation state and does not survive navigation or reload.
- Downward scrolling automatically collapses an idle, pristine composer when
  Home's document scroll offset reaches 96 CSS pixels. Offset transitions count
  regardless of whether wheel, touch, keyboard, or script initiated them.
- Automatic collapse is suppressed while the User is actively editing or while
  current composer values differ from their initialized values, including
  changes outside the body field. Reverting every change or completing a
  successful submission/reset restores eligibility; a failed submission does
  not.
- Collapsing hides presentation only. The composer remains mounted and retains
  every field value and disclosure state.
- The Home masthead remains visible. The collapsed composer becomes a compact
  **New post** row beneath it with an explicit **Expand composer** control.
- The expanded composer provides an explicit **Collapse composer** control, so
  keyboard and non-scrolling Users can choose the compact state too.
- Manual expansion while down-page remains stable. Automatic collapse re-arms
  only after Home's document scroll offset returns to 24 CSS pixels or less; the
  next downward crossing of 96 CSS pixels may collapse it again.
- The behavior applies at desktop and narrow/mobile widths. It must not add a
  sticky composer or nested vertical scrolling to narrow layouts.
- Both controls expose their action through accessible names and ordinary
  keyboard activation. Collapsing moves focus to **Expand composer**; expanding
  moves focus to **Collapse composer**, so focus never remains in hidden
  content.
- Under `prefers-reduced-motion: reduce`, presentation changes immediately with
  no height, translation, or opacity animation; state and focus behavior remain
  identical.

## Acceptance

- On a fresh authenticated visit to `/app`, the complete inline composer is
  visible and usable.
- With an untouched, unfocused composer, crossing 96 CSS pixels while scrolling
  down collapses it into the compact **New post** row and exposes **Expand
  composer**.
- Focusing a composer field or changing any composer input prevents automatic
  collapse while scrolling; reverting all values or a successful reset restores
  eligibility, while a failed submission does not.
- **Collapse composer** compacts the composer without clearing its fields;
  expanding it again reveals the same values and open disclosure state.
- Expanding midway down Home does not immediately collapse again. Returning to
  24 CSS pixels or less and then crossing 96 CSS pixels downward re-enables
  automatic collapse.
- The flow works with keyboard activation, transfers focus to the newly visible
  toggle, and exposes accurate expanded/collapsed semantics to assistive
  technology.
- With reduced motion requested, collapsing and expanding use no height,
  translation, or opacity animation.
- The existing Home timeline ordering, pagination, Post creation, success/error
  feedback, and sticky desktop chrome continue to work.
- Comparable visual-proof pairs show `/app` at desktop and narrow/mobile widths:
  each pair uses the same authenticated seeded state and viewport, with the
  before image showing the current full composer after scrolling and the after
  image showing the compact row at the same scroll position.
- An end-to-end test proves the scroll-triggered transition, dirty-form guard,
  state preservation, manual expansion stability, and re-arm behavior.

## Boundaries

- Do not change Post creation, validation, publication, scheduling, audience,
  Media, or persistence semantics.
- Do not add a persisted User preference for composer presentation.
- Do not change the dedicated `/posts/new` composer.
- Do not redesign Home, its masthead, timeline rows, or timeline order control.
- Do not introduce a general application-wide scroll-state framework.
