# Transactional public Syndication Feed invalidation

## Outcome

Web and AtomPub mutations of locally authored Posts produce the same precise,
transactional Syndication Feed work. Public feed membership is resolved before
HybridWindow ranking, so non-Public Posts cannot consume the count floor.

## Load-bearing decisions

- This change implements accepted ADR-0137 and ADR-0139. It does not establish
  new Syndication Feed or WebSub policy and requires no new ADR.
- A mutation earns feed work exactly when its old and new Post states differ in
  the anonymous/Public projection evaluated at one explicit operation instant
  supplied to the storage-owned boundary. That same instant is reused for
  mutation timestamps and visibility classification.
- A storage semantic no-op is the issue's byte-identical mutation: it leaves the
  Post and its feed-visible update time unchanged and earns no feed work. Feed
  documents are not serialized inside a storage transaction for comparison.
- Draft, future-scheduled, and published Posts without a Public audience are
  absent from the current public projection. Changes that remain wholly within
  those states earn no public feed work.
- A Public-to-Public semantic change earns feed work because the Post update
  time is part of Atom and JSON items, RSS feed metadata, and feed validators.
- A transition into the Public projection invalidates the new surfaces; a
  transition out invalidates the old surfaces. A Public Tag change invalidates
  Site and User surfaces plus the union of old and new Tag surfaces.
- Every affected surface is invalidated in RSS, Atom, and JSON Feed. Atom and
  JSON serialize Tag information directly; RSS still changes through its feed
  metadata and validator when a Public Post's update time changes.
- Concrete surfaces remain the ADR-0137 set: Site, User, Site Tag, and User Tag
  Syndication Feed URLs in all three formats.
- The storage-owned Post mutation boundary owns the complete operation: read old
  state under the `WriteScope`'s backend-appropriate write lock, mutate,
  classify the old/new public projection, compute paths, and enqueue feed events
  in that `WriteScope`. Web and AtomPub do not duplicate this policy.
- Mutation failure or feed-event insertion failure rolls back both the Post
  change and its feed work. Remote WebSub I/O remains outside the transaction.
  Existing commit-indeterminate behavior remains governed by the shared
  `WriteScope` contract.
- Future-scheduled Public Posts earn no authoring-time event while still in the
  future. The restart-durable due-time pass enqueues them after they cross its
  exclusive-lower/inclusive-upper interval boundary.
- The due-time pass excludes Deleted and non-Public Posts and uses their current
  author and Tag state. Delivery remains duplicate-safe and at-least-once; this
  change does not promise exactly-once processing across retries or crashes.
- The existing storage listing remains viewer-aware. Eligibility for the
  supplied viewer is applied before deterministic
  `published_at DESC, post_id DESC` ranking and before the count/age union.
- Current public Syndication Feed regeneration continues to supply the anonymous
  viewer. This issue neither introduces nor promises authenticated or
  account-associated Syndication Feed URLs.
- SQLite and PostgreSQL share the mutation policy and generic listing semantics;
  only existing dialect-specific SQL fragments may differ.

## Acceptance

Unless stated otherwise, every database-backed acceptance criterion below is
proven by backend-parametric integration evidence on both SQLite and PostgreSQL.

- Equivalent Web and AtomPub create, replacement update, delete, publish, and
  unpublish operations enqueue the same concrete feed paths.
- On both SQLite and PostgreSQL, a successful Public Post mutation and every
  affected feed-event row commit atomically.
- On both backends, an injected feed-event insertion failure rolls back the Post
  mutation for shared create/update and lifecycle mutation boundaries.
- Semantic no-op and repeated lifecycle writes add no public feed-event rows
  regardless of current visibility. Mutations whose old and new states both
  remain outside the current anonymous/Public projection—including drafts,
  future-scheduled Posts, and published Posts without a Public audience—also add
  no rows.
- Public creation, editing, publication, unpublication, and deletion enqueue
  exactly the affected Site, User, and applicable Tag feed paths in RSS, Atom,
  and JSON Feed.
- Changing a currently live Post from no Public audience—including Private,
  Subscribers-only, or Named-only targeting—into Public invalidates its new
  surfaces. Removing Public invalidates its old surfaces. Changes wholly among
  no-Public-audience targets enqueue nothing.
- Moving a Public Post from old Tags to new Tags invalidates the union of old
  and new Tag surfaces without duplicate paths.
- Rescheduling wholly within the future creates no immediate public feed work.
  Crossing into current Public visibility creates work immediately; otherwise
  the due-time pass creates it when the scheduled instant becomes eligible.
- Rescheduling a currently visible Public Post into the future immediately
  invalidates its old surfaces and creates no additional event when the new due
  time later arrives unless the Post is still Public then.
- Both the steady-state `(last_tick, now]` pass and the feed-relative startup
  catch-up produce no work attributable to Deleted Posts or Posts without a
  Public audience, and startup catch-up remains restart-durable.
- A dual-backend HybridWindow regression places a newer ineligible Post ahead of
  an older eligible Post outside the age floor and proves that the eligible Post
  still satisfies the count floor.
- Viewer-aware HybridWindow evidence proves the same eligibility-before-ranking
  rule for anonymous and authenticated viewer resolution without changing the
  public regeneration caller's anonymous behavior.
- Existing Site, User, Site Tag, and User Tag count/age ordering remains
  deterministic and backend-equivalent.
- Existing WebSub worker behavior remains cache-before-publish,
  duplicate-tolerant, and outside the authoring transaction.

## Boundaries

- No WebSub retry, `Retry-After`, dead-letter, redrive, or hub-configuration
  work from issue #1052.
- No feed-window setting activation, overflow, or corrupt-value work from issue
  #1053.
- No HTTP validator or feed-cache representation-time work from issue #1054.
- No private/account-associated feed URL, credential, or transport design.
  Preserving the viewer-aware storage listing is the only preparatory choice.
- No revision-history, Deleted Post replay, serializer, feed-format, Tag
  display, or audience-model changes.
- `CONTEXT.md` remains unchanged because the existing Post, Tag, Syndication
  Feed, and WebSub Publish Ping terms cover this behavior.
