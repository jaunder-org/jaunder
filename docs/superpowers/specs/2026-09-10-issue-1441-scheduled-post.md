# Make scheduled publication explicit in the new-Post form

Issue: #1441

## Outcome

An author creating a Post can deliberately set an optional local publication
date and time without a partially completed browser control being mistaken for
“publish now.” A future publication time creates a Scheduled Post, and the
author-visible result identifies that state clearly.

## Load-bearing decisions

- This issue changes the full `/posts/new` composer. Editing, unpublishing, and
  rescheduling existing Posts remain in #1440.
- The optional publication time uses an inline disclosure, not a modal dialog
  and not one native `datetime-local` field.
- With no publication time set, the form shows **Set publication time…**.
  Activating it reveals separate native Date and Time fields with Apply and
  Cancel actions.
- The disclosed fields are provisional. Apply commits a complete valid local
  date and time; Cancel restores the previously committed value without changing
  it.
- While the disclosure contains unapplied changes, both Post-creation actions
  are disabled. The author must Apply a valid value or Cancel before creating
  the Post, so the primary action cannot bypass an incomplete selection.
- Selecting a date while Time is empty visibly fills Time with `00:00`. This is
  a displayed, editable default, not a hidden interpretation performed during
  submission.
- Apply cannot commit an absent, malformed, or impossible local time. The form
  reports an inline error and does not create a Post. Local times in a daylight
  saving gap are impossible; an ambiguous fall-back time retains the existing
  earlier-instant resolution.
- The disclosure states that the value uses the browser’s local timezone.
  Jaunder continues to convert the committed local wall time to a UTC instant at
  the browser boundary.
- Once committed, the form shows the formatted publication time plus **Change
  publication time…** and **Clear schedule**. Clear returns to the unset state.
- While a publication time is committed, **Save draft** is unavailable. An
  author who wants a Draft clears the schedule first; saving a Draft never
  silently discards a committed publication time.
- A committed instant is classified against the browser clock when Apply commits
  it. A future instant changes the primary creation action from **Publish** to
  **Schedule**; an already-due instant retains **Publish** for ADR-0027
  backdating. That label remains stable until the value is changed; if the
  instant passes while the form remains open, the server request clock is
  authoritative and publishes it immediately.
- Backdating remains supported as required by ADR-0027. This issue does not
  reinterpret a past publication time as a schedule.
- After confirmed scheduling, the existing creation summary says **Post
  scheduled.** and links to the Post’s author-visible permalink; creation does
  not add an automatic redirect. The permalink identifies the Post as
  **Scheduled for …**, formatted in the browser’s local wall time with a **local
  time** indicator, rather than presenting it as already live.
- Scheduled visibility remains the ADR-0027 invariant: the author can inspect
  the Post, but public pages and Syndication Feeds omit it until its publication
  instant.
- No new domain term or architectural decision is introduced. The existing
  Draft, Scheduled Post, and live Post states remain derived from
  `published_at`.

## Acceptance

- On `/posts/new`, an author can open the publication-time disclosure, select a
  future date, and observe Time become `00:00` before applying the choice.
- Cancel leaves the prior committed publication time unchanged; Clear removes a
  committed publication time.
- While the disclosure has unapplied changes, both Post-creation actions are
  disabled. Apply refuses incomplete or impossible local date/time input with an
  inline error and commits nothing.
- With a future value committed, **Save draft** is unavailable, the primary
  action reads **Schedule**, and creation stores the selected local wall time
  converted to UTC.
- The creation summary says **Post scheduled.** and retains the existing **View
  post** link. Following it shows **Scheduled for …** in browser-local wall time
  with a **local time** indicator. An unauthenticated viewer cannot read the
  Post there or through public listing and Syndication Feed surfaces before it
  is due.
- With no committed value, Publish retains the existing immediate-publication
  behavior. A committed past value retains ADR-0027 backdating behavior.
- The regression is developed red-first at the agreed Playwright seam: the real
  `/posts/new` form, publication-time interaction, creation action, resulting
  permalink, and pre-due public visibility.

## Boundaries

- No change to storage schema, publication-state derivation, scheduled worker,
  public time gates, or Syndication Feed generation.
- No edit-page lifecycle controls, rescheduling, or unpublishing work from
  #1440.
- No timezone selector, persisted author timezone, or timezone inference beyond
  the existing browser-local conversion boundary.
- No modal/dialog infrastructure and no redesign of unrelated forms.
- The compact inline composer remains publish-now/draft only.
