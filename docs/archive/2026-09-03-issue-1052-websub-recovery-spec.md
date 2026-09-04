# Issue #1052: Publisher-side WebSub configuration and recovery

## Outcome

Publisher-side WebSub configuration changes immediately remove stale discovery,
and each queued Syndication Feed change is processed from one coherent current
configuration snapshot. Retryable failures recover under phase-specific bounded
policies; terminal work is visible and explicitly redrivable by operators from
both the CLI and web administration surface.

## Load-bearing decisions

- ADR-0137 remains authoritative: every WebSub Publish Ping names one concrete
  public Syndication Feed URL, carries no content, and is duplicate-safe and
  at-least-once rather than exactly-once.
- Each feed event remains one durable lifecycle row. The row has explicit
  regeneration and publication phases, independent attempt counters and
  diagnostics, and distinct regeneration and publication dead-letter states.
  There is no second queue or fallible cross-table handoff.
- A successful regeneration commits the cache before the row enters publication.
  Publication retries do not repeat successful regeneration while that cached
  representation remains present.
- Any publication-phase work whose cached representation has since been removed
  re-enters regeneration with a fresh regeneration budget before another ping.
  This includes publication redrives after a hub configuration change.
- Each grouped feed-path attempt late-binds exactly one coherent snapshot of the
  current Feed Configuration and Site Identity immediately before processing.
  Regeneration and publication consume that snapshot without rereading either
  source during the attempt.
- A successful hub configuration mutation wins over every in-flight attempt
  holding an older snapshot. Before committing regenerated cache state or
  sending a ping, the attempt must prove that its hub snapshot remains current;
  stale work is immediately requeued from a fresh snapshot without consuming
  either phase's retry budget.
- A storage, configuration-access, or Site Identity read error prevents cache
  and network work and consumes the regeneration retry budget. It never becomes
  `NoHub`.
- A malformed stored hub URL is conditionally purged and treated as unset. The
  repair also invalidates cached Syndication Feeds and must not delete a valid
  replacement written concurrently.
- With no configured hub, regeneration still commits the current representation
  and then completes without a ping. Later hub enablement does not replay that
  completed history.
- Setting, changing, or unsetting the normalized hub URL atomically commits the
  new configuration and invalidates every cached Syndication Feed
  representation. Writing the already-configured normalized value is a no-op and
  does not invalidate caches.
- Regeneration receives seven total attempts: the initial attempt and retries
  after 1 minute, 5 minutes, 30 minutes, 2 hours, 2 hours, and 2 hours.
- Publication receives ten total attempts under its local fallback schedule: the
  initial attempt and retries after 1 minute, 5 minutes, 30 minutes, 2 hours, 4
  hours, 8 hours, 12 hours, 24 hours, and 24 hours. Without `Retry-After`, the
  bounded window is approximately three days.
- Every HTTP 2xx response succeeds. Transport failures, timeouts, HTTP 408, HTTP
  429, and HTTP 5xx responses are retryable publication failures.
- The client follows at most three HTTP(S) 307/308 redirects while preserving
  the POST method and form body. HTTP 301/302/303, a missing or invalid
  `Location`, a redirect loop, a fourth redirect, and any other final HTTP 3xx
  or 4xx response enter the publication dead letter immediately.
- A retryable HTTP response may replace the local publication delay with a valid
  `Retry-After` delta-seconds value or future HTTP-date. The selected delay is
  capped at 24 hours. Missing, malformed, or past values fall back to the local
  schedule.
- Operator recovery accepts one or more exact feed-event IDs. Every supplied ID
  must still exist and be dead-lettered; validation and redrive are atomic, so
  an absent, expired, or non-dead-lettered ID rejects the entire request.
- Redrive resets only the failed phase's attempts, diagnostic, terminal state,
  and scheduling state. A regeneration dead letter resumes regeneration. A
  publication dead letter resumes publication unless the missing-cache rule
  returns it to regeneration.
- Exhausted rows remain governed by ADR-0167: both dead-letter categories are
  inspectable and redrivable for seven days, after which bounded maintenance may
  remove them. Completed rows remain immediately cleanup-eligible.
- Operators receive both surfaces over the same storage contract: bounded CLI
  inspection/redrive commands and a dedicated operator-only Admin WebSub page.
  The existing `site-config set/unset` CLI remains an editable hub-configuration
  surface and uses the atomic cache-invalidating mutation. The Admin WebSub page
  edits the same normalized hub configuration and provides separate regeneration
  and publication dead-letter views with exact-ID redrive actions.
- Inspection is filterable by failed phase and uses newest-first keyset
  pagination ordered by terminal time then event ID, both descending. The shared
  page-size contract defaults to 50 and accepts at most 50 rows. Its cursor
  names both ordering values so equal timestamps neither skip nor duplicate
  rows.
- Each result exposes the event ID, Syndication Feed path, failed phase, phase
  attempt count, terminal time, and operator diagnostic; it exposes no Post
  content, secret, or client-visible internal error. A diagnostic is at most
  1,024 Unicode scalar values; longer text is truncated on a scalar boundary to
  1,023 values plus one ellipsis before storage.
- Existing dependency-injection, typed-error, structural write-scope, SQLite
  lock occupancy, PostgreSQL claim concurrency, observability, and
  backend-parity decisions remain in force.

## Acceptance

- Setting an absent hub, changing the hub, and unsetting the hub through either
  the existing CLI or the Admin WebSub page each leave both SQLite and
  PostgreSQL with the new configuration and no cached Syndication Feed
  representations in one confirmed mutation. Both surfaces preserve the cache
  when the submitted normalized value is unchanged.
- A malformed stored hub is removed without removing a concurrent valid
  replacement, all stale discovery is invalidated when repair wins, and the
  attempt continues under no-hub semantics.
- A worker attempt demonstrably uses one Feed Configuration/Site Identity
  snapshot even if either configuration changes while regeneration is running. A
  hub mutation that wins during the attempt prevents stale cache commit and
  publication to the replaced hub, then restarts the work from a fresh snapshot
  without charging either retry budget.
- Injected snapshot storage failures perform no regeneration or WebSub request,
  follow the regeneration schedule, and eventually produce an inspectable
  regeneration dead letter rather than `NoHub` completion.
- Regeneration and publication attempts advance independently on both backends.
  Successful regeneration is not repeated by an ordinary publication retry.
- Protocol tests prove all-2xx success handling; transport and timeout retries;
  408/429/5xx retries; the exact bounded, method-preserving 307/308 redirect
  policy; immediate terminal handling for every other 3xx/4xx response; both
  accepted `Retry-After` forms; the 24-hour cap; and local-delay fallback.
- Dual-backend integration tests prove pending, claimed, regeneration retry,
  regeneration dead letter, publication retry, publication dead letter,
  completion, stale-claim recovery, redrive, and terminal-retention transitions.
- CLI and Admin WebSub flows edit hub configuration, list each dead-letter
  category without skips or duplicates under stable bounded pagination and phase
  filtering, reject nonoperators on the web surface, and atomically redrive
  exact selected IDs.
- Publication redrive sends without redundant regeneration when the cache is
  present, but a cache invalidated after the original regeneration is rebuilt
  and committed before the next ping.
- Existing WebSub end-to-end coverage still observes the exact affected
  Syndication Feed URL wave after public Post changes.

## Boundaries

- Mutation-to-feed-event atomicity delivered by issue #1051 is unchanged.
- This issue does not implement multiple hubs, per-topic hubs, inbound WebSub
  subscriptions, or exactly-once HTTP delivery.
- Syndication Feed window selection and HTTP cache validators remain issues
  #1053 and #1054.
- Retry schedules, the 24-hour `Retry-After` cap, and seven-day terminal
  retention are fixed product policy here, not new runtime configuration.
- Inspection is operational state, not durable delivery history; pruning after
  the accepted retention window remains valid.
