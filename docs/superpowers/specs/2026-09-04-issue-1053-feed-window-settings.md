# Issue #1053: Feed window settings

## Outcome

Syndication Feed selection is configurable through the site configuration values
`feeds.min_items` and `feeds.min_days`, without weakening the feed policy in
[ADR-0139](../../adr/0139-syndication-feed-hybrid-window.md).

Each accepted configuration snapshot is a coherent publisher generation: feeds
produced from earlier configuration cannot become newly visible after it.

## Load-bearing decisions

- The two settings form one publisher/feed configuration snapshot; either
  setting may be set or unset independently.
- An unset `feeds.min_items` means 20 items. An unset `feeds.min_days` means 30
  fixed, UTC 24-hour days.
- Defaults apply only to absent values, not malformed stored values.
- Every accepted set or unset, including one that is semantically unchanged,
  advances the publisher generation and invalidates every cached Syndication
  Feed before the mutation returns success.
- A feed regeneration is bound to the generation snapshot from which it began; a
  stale concurrent regeneration or commit cannot replace the feed cache for a
  newer generation.
- Each setting mutation is atomic across durable configuration, publisher
  generation, and feed-cache invalidation. A failed mutation leaves the prior
  coherent snapshot observable.
- If durable commit status is indeterminate, the operation is not reported as
  confirmed success; callers receive an outcome that preserves that uncertainty.
- Configuration loading validates both stored values before exposing a
  publisher/feed snapshot. A corrupt value fails the whole snapshot rather than
  silently receiving its default.
- Corruption diagnostics are typed and include the offending configuration key
  and validation reason, never its raw stored value. If both settings are
  corrupt, `feeds.min_items` is reported first for deterministic behavior.
- Minimum-items and minimum-days remain the union window defined by ADR-0139.
  Eligibility, ordering, inclusive cutoff behavior, regeneration snapshot
  semantics, and defaults retain their ADR-0139 meanings.
- Age-cutoff arithmetic is checked. A cutoff too old to represent selects all
  otherwise eligible history instead of panicking or narrowing the result.
- SQLite and PostgreSQL implement the same externally observable behavior.

## Acceptance

- Setting either key through the CLI site-configuration surface yields feeds
  selected according to its new value while retaining the other key's value or
  default.
- Unsetting either key restores its stated default and yields feed selection
  consistent with that default.
- A semantic no-op set or unset still fences prior feed work: the publisher
  generation advances and every cached Syndication Feed is invalidated before a
  successful response is observable.
- A regeneration begun before such a mutation cannot commit a stale feed after
  the newer generation is established, including when its work completes later.
- A failed atomic update leaves the prior configuration, generation, and cached
  feed state coherent; an indeterminate durable commit is never returned as a
  confirmed successful mutation.
- A configuration with one corrupt stored key fails with a typed diagnostic that
  names that key and validation reason, without exposing its value or
  defaulting.
- A configuration with both keys corrupt fails as one invalid snapshot and
  reports `feeds.min_items` first.
- With an unrepresentably old minimum-days cutoff, all otherwise eligible
  history is selected without overflow or panic.
- Equivalent acceptance coverage passes against SQLite and PostgreSQL, including
  generation fencing, mutation outcome handling, defaults, corruption
  diagnostics, and the overflow case.

## Boundaries

- Remote WebSub delivery is out of scope.
- The existing CLI site-configuration mutation surface is the only mutation
  surface in scope; no web UI is introduced.
- This issue does not revise ADR-0139 policy or create a new ADR.
