# ADR-0130: Slug Freeze Follows Current Publication State

- Status: accepted
- Date: 2026-08-14
- Issue: [#549](https://github.com/jaunder-org/jaunder/issues/549)

## Context

[ADR-0027](0027-scheduled-publishing-time-gated-visibility.md) described a
Scheduled Post's slug as permanently frozen once `published_at` had ever been
set. Storage does not retain that history. Both backends decide slug mutability
from the row's current pre-update state:

```sql
slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END
```

Issue #549 adds scheduled-Post editing, including pulling a Scheduled Post back
to draft. The editor therefore needs one rule that agrees with storage before,
during, and after that transition. Enforcing the stronger historical rule would
require a new persisted "ever published" fact and would make existing storage
behavior and draft editing disagree.

## Decision

Slug mutability follows the Post's current stored publication state at the start
of an update:

- When `published_at` is non-null, the update preserves the existing slug. This
  includes the update that clears a schedule or unpublishes a live Post.
- After that update has produced a draft, a later update starts with
  `published_at` null and may change the slug.
- Scheduling or publishing that draft freezes the slug selected by that update
  until another update pulls the Post back to draft.

No historical "ever published" state is added. Scheduled and live editors keep
the slug control hidden; a reopened draft exposes it.

## Consequences

- Good: the web editor, SQLite, and PostgreSQL express the same rule without a
  migration or a second source of publication history.
- Good: pulling a Post back to draft cannot accidentally change its permalink in
  the same update; the author must reopen and deliberately edit the draft.
- Good: scheduling or publishing still stabilizes the permalink for every Post
  that is currently non-draft.
- Trade-off: changing the slug of a pulled-back draft can invalidate a permalink
  that was previously shared. That is explicit draft editing, not a side effect
  of the pullback.
- Supersedes only ADR-0027's stronger historical slug-freeze wording; its
  time-gated visibility and scheduled-publishing decisions remain accepted.
