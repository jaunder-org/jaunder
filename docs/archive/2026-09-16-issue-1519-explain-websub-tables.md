# Explain the WebSub dead-letter tables

## Outcome

The WebSub administration page explains what each dead-letter table represents,
why an operator should care about its rows, and what a reasonable recovery
response is. An operator can distinguish regeneration failures from publication
failures and act without needing prior knowledge of Jaunder's WebSub internals.

## Load-bearing decisions

- Keep the established **dead letter** terminology used by the web surface and
  CLI, but define it in visible prose as feed work whose automatic processing
  stopped because retries were exhausted or publication encountered a terminal,
  non-retryable failure.
- Give each table its own concise, always-visible explanation because the two
  phases have materially different causes and responses.
- Explain regeneration rows as failures to rebuild a cached public Syndication
  Feed representation. Direct operators to review the diagnostic and correct
  relevant storage, site-identity, or configuration problems before redriving.
- Explain publication rows as failures to send a WebSub Publish Ping after the
  Syndication Feed representation was rebuilt. Direct operators to review the
  configured WebSub Hub and diagnostic, correct hub, network, HTTP, or redirect
  problems as applicable, and then redrive.
- Keep response guidance at the category level. Do not reproduce protocol
  response-code tables, redirect limits, retry schedules, or other operational
  implementation detail on this page.
- Explain that operators should address the reported cause before selecting and
  redriving rows. State that a redrive request fails as a whole when any
  selected row is stale or no longer dead-lettered.
- Identify the checkbox column visibly as **Select** while retaining an
  accessible label for each row's checkbox.
- Give each empty table an explicit healthy-state message saying that no work is
  currently dead-lettered for that phase.
- State once that terminal dead-letter rows are retained for seven days, so the
  recovery window is clear without repeating the note per row.
- Preserve the distinction between publisher-side WebSub and inbound
  subscriptions: this page concerns Jaunder's outbound Publish Pings for public
  Syndication Feeds.

## Acceptance

- The regeneration table visibly defines its rows, the affected Syndication Feed
  work, and the reasonable operator response.
- The publication table visibly defines its rows, the WebSub Hub notification
  work, and the reasonable operator response.
- Both explanations are visible without opening a tooltip, dialog, or
  disclosure.
- The page visibly defines dead-lettered work, explains safe redrive order and
  stale-selection behavior, and communicates the seven-day retention window.
- The selection column has a visible **Select** heading, and individual controls
  retain accessible labels identifying their event.
- A phase with no rows renders an explicit phase-specific empty state instead of
  an unexplained empty table.
- Browser coverage proves both explanations, the shared recovery guidance,
  retention guidance, selection heading, and empty states while preserving the
  existing pagination, exact-event redrive, success, and conflict behavior.

## Boundaries

- Do not change WebSub retry, dead-letter, retention, paging, redrive, storage,
  or authorization semantics.
- Do not change the CLI's inspection or redrive contract.
- Do not rename the route, tables, phases, API types, or existing dead-letter
  vocabulary.
- Do not turn the page into a general WebSub protocol reference or an inbound
  subscription administration surface.
- Do not expose diagnostics beyond the existing bounded operator-safe
  projection.
