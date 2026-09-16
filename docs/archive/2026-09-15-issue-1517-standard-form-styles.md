# Standard form styles

Issues: #1517, #1520, #1522

## Outcome

Jaunder's user-facing forms use one coherent field vocabulary: labels, help
text, controls, field spacing, and action regions look consistent even when a
page needs a specialized grid, inline row, or table layout. The visibly unstyled
SMTP Relay and Site Settings forms are repaired, and the other audited form
outliers join the same contract.

## Load-bearing decisions

- The shared form contract owns control presentation:
  - a field stacks its label, control, help, and validation feedback;
  - labels share one typography treatment;
  - text inputs, numeric inputs, selects, and textareas share control chrome;
  - ordinary card forms share padded bodies and separated action footers.
- Specialized classes may arrange fields into grids, inline rows, tables, or
  editor toolbars, but they do not create competing label typography or control
  chrome.
- Compact contextual actions do not need card chrome merely to be standard. They
  still use the shared field, label, control, help, validation, and button
  treatments where those elements are present.
- SMTP Relay and Site Settings use the ordinary settings-card treatment. Their
  current undefined styling vocabulary is removed rather than completed as a
  second form system.
- The Backup form keeps its responsive grid and wide-field placement while its
  labels and controls use the shared presentation contract.
- The WebSub hub form keeps its compact settings-page role while using shared
  field presentation rather than borrowing Backup-specific presentation.
- App Password creation uses the shared labelled-field and input presentation;
  its one-time credential behavior remains unchanged.
- Existing purpose-built layouts for composers, audience rows, tables, media
  controls, and Theme Studio remain purpose-built where the audit found their
  layout intentional and their field presentation already consistent.
- The web style guide describes this separation between shared presentation and
  purpose-specific layout so future forms do not invent undefined or duplicate
  styling vocabularies.

## Audited form population

- Standard card or compact forms: Auth, Registration, Password Reset, Email,
  Invitations, Passkeys, Profile, and Sessions/App Passwords.
- Purpose-specific layouts that still consume shared field presentation: Backup,
  Site Settings, SMTP Relay, WebSub, Audiences, Post composers and draft rows,
  Media, and Theme Studio.
- Subscription, tag, draft-row, and audience-action hidden inputs are wire
  fields, not visible controls; they are outside visual styling while their
  visible buttons remain in the audit.
- A complete audit means every class intended to present a field, label,
  control, help message, feedback message, body, or action region is defined in
  the built-in stylesheet. Selector identity uses `data-*` attributes instead of
  empty styling classes. Every visible `input`, `select`, or `textarea` and its
  label either consumes the shared presentation or belongs to one of the
  purpose-specific layouts listed above with defined styling.

## Acceptance

- At 1080px and 600px viewport widths, SMTP Relay, Site Settings, and Media
  Uploads have nonzero card-body padding, separated field stacks, and an action
  footer with nonzero padding and a top divider.
- Across those forms, standard labels have equal computed font family, size, and
  color; standard controls have equal minimum height, padding, border, radius,
  background, and text color.
- At both representative widths, target form controls and labels do not overlap,
  and the form cards do not make the document horizontally scrollable.
- Backup keeps two field columns at 1080px and one at 600px; Destination Path,
  Schedule, Retention Count, and Mode use the same computed label typography and
  control presentation.
- The WebSub hub URL and App Password label controls use the same computed
  label, input, help/error, and focus presentation as other standard fields.
- SMTP enablement, authentication, password replacement, validation, save
  feedback, and disabled-state behavior are unchanged.
- Site identity, media-upload capability, backup, WebSub, and App Password
  mutations retain their existing request and persistence behavior.
- The audited population contains no undefined form-presentation class, no empty
  styling class used only as selector identity, and no visible control outside
  either the shared presentation or an explicitly classified, stylesheet-backed
  purpose-specific layout.
- Focused browser coverage exercises the 1080px and 600px layout assertions
  while existing accessibility and behavior assertions stay green.
- The style guide explicitly identifies shared field presentation as mandatory
  and specialized layout classes as the supported extension point.

## Boundaries

- No server function, storage, validation, authentication, authorization, or
  secret-handling semantics change.
- This work does not force compact inline or row actions into cards and does not
  redesign specialized composer, table, audience, media, or Theme Studio
  layouts.
- Profile Username presentation and profile explanatory copy remain in #1523.
- This work does not introduce a new component abstraction unless implementation
  reveals repeated behavior rather than repeated styling.
- No new theme API or Style Contract guarantee is introduced; this aligns the
  built-in authenticated interface with its existing internal design system.
