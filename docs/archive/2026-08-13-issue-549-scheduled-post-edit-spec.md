# Issue #549: Scheduled Post edit controls

## Problem

A scheduled Post has a future `published_at`, but `EditPostPage` currently
treats any non-null `published_at` as already live. That collapses ADR-0027's
scheduled and live states into one UI branch:

- the editor hides the schedule control for a scheduled Post;
- the only action is `Save`, which submits `publish = true` and
  `publish_at = None`;
- `PublishUpdate::Publish { at: None }` preserves the existing timestamp, so the
  author can neither reschedule nor pull the Post back to draft.

The schedule value is also not seeded into `ComposeState`. Merely showing the
existing empty control would therefore make an untouched scheduled save
ambiguous and could not preserve the original instant.

## Decisions

1. The edit page distinguishes ADR-0027's three states from the fetched
   `published_at` and a `fetched_at: UtcInstant` captured by the server while
   serving that same preview response: draft (`published_at` is `None`),
   scheduled (`published_at > fetched_at`), and live
   (`published_at <= fetched_at`). That classification remains immutable for the
   loaded editor even if the scheduled instant later passes.
2. The live editor remains unchanged. Its slug and schedule controls stay
   hidden, and its existing `Save` action remains a publish-preserving update.
3. The scheduled editor keeps the slug hidden because a Post whose current
   `published_at` is non-null has a frozen slug. It shows a schedule control
   prefilled with the scheduled instant in the browser's local wall-clock time
   and a `Clear schedule` button.
4. A scheduled Post has one `Save` action. An edited non-empty schedule applies
   that instant: a future value reschedules, while a present or past value
   publishes/backdates under the existing ADR-0027 semantics. An empty schedule
   submits `Unpublish`, pulling the Post back to draft.
5. `Clear schedule` only empties the local field. It does not dispatch; the
   author must choose `Save`, so clearing remains reversible until the normal
   update succeeds.
6. The exact fetched UTC instant is retained separately from the display text.
   If the author does not edit the schedule field, `Save` sends that original
   instant rather than parsing the displayed local value. This prevents
   sub-minute precision loss, timezone-fold changes, and server/browser timezone
   drift.
7. Once the author edits or clears the schedule field, the field becomes the
   source of intent. A non-empty value must convert to a valid `UtcInstant`.
   Invalid input, including a nonexistent DST-gap wall-clock, shows an inline
   validation error and disables `Save`; it never silently becomes an
   unschedule. Empty is the only unschedule value.
8. The loaded editor's explicit action is honored even if the scheduled instant
   passes while the page is open. In particular, clear then `Save` still pulls
   the Post back to draft. This issue does not add optimistic concurrency or a
   stale-state rejection protocol.
9. Navigation remains the existing split. A successful update whose returned
   `published_at` is non-null redirects to the author-visible permalink. An
   unscheduled update remains in the editor and shows the existing save result.
10. Slug mutability follows current publication state, revising ADR-0027's
    stronger historical wording
    ([decision draft](../adr/drafts/current-publication-state-slug-freeze.md)).
    The update that clears a schedule preserves the frozen slug because the
    pre-update `published_at` is non-null. When the resulting draft is reopened,
    its slug control is visible and a later draft save may change it;
    rescheduling or publishing freezes the then-current slug again.
11. This issue repairs only the existing edit page. A dedicated scheduled-Post
    listing or broader management surface remains issue #15.

## Acceptance criteria

1. The preview response carries a server-captured `fetched_at`, and scheduled
   versus live controls are selected by comparing `published_at` with that
   snapshot rather than with the browser clock.
2. Editing a scheduled Post shows its schedule control and a `Clear schedule`
   button, while keeping the slug control hidden.
3. The schedule control is prefilled with the scheduled instant expressed as the
   browser's local `datetime-local` wall-clock value.
4. Saving without editing the schedule preserves the exact fetched UTC instant,
   including precision not representable by the control and an instant in a
   repeated DST wall-clock interval.
5. Changing the schedule to another valid future local time and saving updates
   `published_at` to the corresponding UTC instant; reopening the editor shows
   the replacement schedule.
6. Changing the schedule to a valid present or past local time uses the existing
   publish/backdate behavior rather than rejecting it as an edit-only rule.
7. Choosing `Clear schedule` empties the field without a server mutation. A
   subsequent `Save` sets `published_at` to null, leaves the author on the edit
   page, and reports the successful save.
8. An empty schedule is the only path from the scheduled editor to
   `PublishUpdate::Unpublish`; an untouched or valid non-empty schedule submits
   `PublishUpdate::Publish { at: Some(...) }`.
9. An invalid non-empty schedule cannot dispatch: the editor shows an inline
   error and disables `Save` instead of treating it as empty.
10. A scheduled Post that becomes due while its editor is open still honors an
    explicit clear-and-save as an unschedule.
11. After clear-and-save, reopening the resulting draft shows the slug control;
    the pullback itself preserved the previously frozen slug, a later draft save
    may change it, and scheduling or publishing freezes the current slug again.
12. Editing a live Post retains the current controls and behavior: no slug or
    schedule control, and one publish-preserving `Save` action.
13. End-to-end coverage drives schedule creation, edit prefill, reschedule,
    reopen, clear, save, and the resulting draft state on both storage backends
    through the repository's existing e2e matrix.
14. The applicable repository validation gate passes.

## Out of scope

- A dedicated scheduled-Post list or management dashboard (#15).
- A durable “ever scheduled or live” flag or historical slug freeze.
- A separate `Unschedule` server function or storage mutation.
- Optimistic concurrency or stale-edit rejection.
- Changing AtomPub scheduling behavior.
- Changing the existing create-form scheduling contract.
