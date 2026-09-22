# Issue #1619 — Post summary hierarchy

## Outcome

An authored Post summary reads as a distinct article deck between the optional
title and body instead of appearing to be the first body paragraph. The
presentation remains readable, responsive, and themeable on every public Post
surface.

## Load-bearing decisions

- The summary uses a conventional deck treatment: normal style, slightly smaller
  than Post body text, a softer text color, compact line height, and explicit
  separation from the body.
- Italics are not part of the baseline treatment because summaries may contain
  several sentences and must remain comfortably readable.
- The summary receives no label, border, background, icon, or other ornamental
  container; typography and spacing alone establish its hierarchy.
- An authored summary is displayed in full. Jaunder does not clamp, truncate, or
  replace its text for presentation.
- A titleless Post gives its summary the same deck treatment directly after the
  Post header. The summary's semantic role does not depend on title presence.
- The baseline applies anywhere the shared public Post presentation appears,
  including Local, author, tag, and permalink routes.
- `post-summary` remains the Style Contract ownership boundary. A custom Theme
  Package may override the baseline through that semantic hook without relying
  on incidental document structure.

## Acceptance

- A Post with title, summary, and body visibly separates all three layers in
  that order, with the summary subordinate to the title and distinct from body
  prose.
- A titleless Post still presents its full summary as a deck rather than body
  text.
- Long and multiline summaries wrap normally and remain complete at wide and
  narrow viewports without horizontal overflow.
- Local, author, tag, and permalink routes render equivalent summary hierarchy;
  absence of a summary leaves existing spacing unchanged.
- Light and system-dark presentation retain readable summary contrast and do not
  use italics or ornamental chrome.
- A focused regression proof covers present, absent, titleless, and long-summary
  states, and confirms a custom Theme Package can override `post-summary`.
- Transient Before/After visual pairs show deterministic titled and titleless
  summary states on Local at 1440×900 and 390×844 in light and system-dark
  modes.
- Existing public-route accessibility checks continue to report no detectable
  WCAG 2.2 Level A or AA violations.

## Boundaries

- This issue does not change summary authoring, validation, persistence,
  protocol serialization, metadata derivation, or renderer source order.
- It does not add a new Style Contract hook or change Theme Package schema.
- It does not redesign Post titles, body typography, cards, or unrelated public
  chrome.
