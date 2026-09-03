# Theme Selector

Issue: #21

## Outcome

Authenticated users can reach `/profile` through Settings navigation and choose
Terminal, Studio, or Reader as their browser's active theme. The choice applies
immediately and persists in that browser across reloads.

## Load-bearing decisions

- Theme selection remains browser-local and continues to use the existing theme
  state and local-storage persistence path.
- The selector lives on `/profile`; the existing Settings navigation item links
  to that route so the control is discoverable without typing a URL.
- The three choices are exactly the shipped built-in themes: Terminal, Studio,
  and Reader.
- The choices use a segmented-button control with the programmatic group name
  `Theme`. Each button exposes its selected state with `aria-pressed`.
- Choosing a built-in theme applies it immediately. There is no staged value,
  Save button, server write, or account synchronization.
- Existing nonempty stored theme identifiers remain valid opaque values. If the
  stored identifier is not one of the three built-ins, the selector shows no
  built-in as selected and does not replace that identifier until the user
  explicitly chooses a built-in.
- The existing Studio default remains unchanged when no usable stored theme is
  present.
- The already-landed fix from #22 remains the authority for applying the
  selected value to `.j-root` through `data-theme`.

## Acceptance

- An authenticated user can follow Settings navigation to `/profile`.
- `/profile` presents a visible theme control exposed programmatically as a
  `Theme` group containing Terminal, Studio, and Reader buttons.
- Exactly the button matching the current built-in theme reports
  `aria-pressed="true"`; the other built-in buttons report `false`.
- Selecting each button updates `.j-root` to the corresponding lowercase
  `data-theme` identifier without a page reload.
- The selected built-in theme remains active after reloading the page in the
  same browser.
- Given an unknown nonempty stored theme identifier, opening `/profile` leaves
  that identifier unchanged and reports all three built-in buttons as unpressed.
- Choosing a built-in from that unknown state replaces the stored identifier and
  activates the chosen built-in theme.
- Existing storage read/write failures remain truthfully reported through the
  established client telemetry path rather than preventing the control from
  rendering.
- Browser-flow coverage exercises navigation, immediate application, the named
  group and toggle-button states, persistence, and unknown-identifier
  preservation.

## Boundaries

- Custom theme creation, editing, deletion, validation, and synchronization are
  tracked separately by #1341 and do not block this issue.
- This issue adds no server setting, database field, API, account-synced
  preference, custom CSS input, or operator-wide theme configuration.
- This issue does not change the shipped theme definitions, the Studio default,
  public projector output, or public-response cacheability.
- This issue does not redesign `/profile`, the sidebar, topbars, or the general
  segmented-control component family beyond what the theme control requires.
