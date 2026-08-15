# #937 — protocol and local content-lifecycle decisions

Issue: [#937](https://github.com/jaunder-org/jaunder/issues/937). Milestone:
Developer tooling & DX.

## Summary

Four pieces of shipped architecture have no governing decision record:
publisher-side WebSub, Syndication Feed membership, HTTP cache validation, and
the local Post lifecycle. This cycle records four ADRs, projects them into the
architecture view, and adds the domain language needed to keep outbound
publishing distinct from inbound syndication.

The investigation found behavior that should not become policy merely because it
shipped. The ADRs therefore establish coherent intended invariants. This cycle
does not change production behavior; every mismatch between those invariants and
the current implementation becomes a linked follow-up issue. The architecture
view must distinguish the accepted target from each known current deviation
until those issues land.

## Decisions

### D1 — ADR boundaries

Write four numberless drafts and promote them at ship:

1. publisher-side WebSub;
2. the `HybridWindow` for cached Syndication Feed membership;
3. HTTP conditional validation of cached Syndication Feed representations; and
4. local Post soft deletion and revision snapshots as one lifecycle policy.

Feed membership and HTTP validation change for different reasons and therefore
remain separate. Soft deletion and revisions share one durable-identity,
retention, media, and access model and therefore remain together. ADR-0010 and
ADR-0009 are explicitly opposite-leg near-misses: they govern inbound consumed
content, not locally authored Posts or outbound Syndication Feeds.

### D2 — publisher-side WebSub

Jaunder acts as a WebSub publisher for every concrete public Syndication Feed
URL: Site, User, Site Tag, and User Tag surfaces in RSS, Atom, and JSON Feed.
One optional site-wide WebSub Hub is advertised by each representation and
receives a publish request naming that exact feed URL as the topic. A WebSub
Publish Ping announces a changed representation; it carries no content.

A ping is earned by a protocol-independent change to at least one concrete
public Syndication Feed representation, not by an authoring endpoint invocation.
Web and AtomPub mutations follow the same rule. Draft, private, and
byte-identical changes do not ping. Publication, unpublication, deletion,
due-time go-live, edits, and tag changes notify only when their old/new
projection changes; tag changes invalidate the union of affected old and new tag
surfaces.

The Post mutation and affected-feed events commit atomically as a transactional
outbox. The worker asynchronously regenerates and commits the cache before
publishing. Remote delivery is duplicate-safe, at-least-once, and never claimed
as exactly once.

Hub set/change/unset invalidates every cached Syndication Feed. Work late-binds
to one coherent snapshot of the current hub and site identity. With no hub,
regeneration completes without a ping and is not replayed if a hub is enabled
later. ADR-0102's malformed stored hub URL remains purge-as-unset; storage,
configuration-access, and site-identity read errors retry rather than becoming
`NoHub`.

Feed regeneration and remote publish have independent bounded attempt budgets.
Transport failures, timeouts, 408, 429, and 5xx responses are retryable; other
4xx responses are terminal. `Retry-After`, when valid and within the bounded
policy, governs the next attempt. Exhausted regeneration work and exhausted or
terminal publish work remain separate dead letters with supported inspection and
explicit redrive. Exact attempt counts and backoff intervals are operational
policy, not architectural identity.

### D3 — cached Syndication Feed membership

`HybridWindow` selects the union of:

- the first `feeds.min_items` eligible Posts; and
- every eligible Post whose publication time is at least the inclusive
  `feeds.min_days` cutoff.

Anonymous/Public eligibility is applied before ranking. Ranking is deterministic
by `published_at DESC, post_id DESC`. Defaults remain 20 Posts and 30 fixed
24-hour UTC days: stable, simple behavior that preserves a useful floor for a
quiet publication and a useful recent interval for a busy one.

The window is a regeneration-time snapshot. Time passing alone need not schedule
an age-out regeneration; a cached feed may retain an older Post until the next
regeneration. This limitation is explicit rather than described as a
continuously exact 30-day window.

A successful valid setting mutation durably invalidates all cached Syndication
Feeds before returning. Unset settings use the 20/30 defaults. Cutoff arithmetic
is checked; an age too large for date arithmetic means all-history rather than
panic. Corrupt stored values surface a configuration error instead of silently
reverting to defaults. This narrowly supersedes ADR-0102's unchanged-read
fallback for `feeds.min_items` and `feeds.min_days`. No arbitrary maximum is
added without an operationally justified bound.

### D4 — HTTP conditional validation

Each cached representation has a strong ETag that is a deterministic function
only of a complete, ordered semantic identity tuple plus serializer revision.
The tuple covers every input capable of changing serialized bytes, including
feed format; any representation-byte change must change the tag, while identical
semantic inputs and bytes retain it across regeneration. Serializer revision
covers behavior changes in the ADR-0015/ADR-0089-governed serializer paths,
including upstream wire-layout changes. This deliberately retains tuple
derivation rather than hashing the completed body, accepting the maintenance
burden of keeping the tuple complete.

`If-None-Match` follows RFC 9110 GET/HEAD semantics: weak comparison, comma
lists, wildcard support, and precedence over `If-Modified-Since`. A malformed
condition does not create a false 304. For GET, a non-match returns 200 with the
body; for HEAD, a non-match returns 200 with GET-equivalent headers and no body.
A match returns 304 with no body and with the current validators and cache
metadata for either method.

`Last-Modified` is retained as a weak, whole-second date validator backed by a
persisted representation-modification time. That time changes only when the
cached representation identity changes, including metadata-only changes and
removals; it remains stable for identity/byte-identical regeneration and is not
`max(item.updated_at)`. When both validators are supplied, the ETag is
authoritative. `Cache-Control: public, max-age=300` remains the current
downstream freshness policy and does not imply server-side cache regeneration.

### D5 — local Post lifecycle

A Post row is durable canonical identity and latest state. Soft deletion stamps
a tombstone and removes the Post from active web reads, public Syndication
Feeds, and AtomPub Collections. Active lookups behave as absent (404/omission),
not as a public tombstone or 410. Public permalink identity is active-only: a
new Post may reuse a deleted Post's permalink while the tombstone retains its
internal Post ID.

Before every meaningful state change, storage appends a full-state Post Revision
containing the prior authored source and rendered representation, title, slug,
summary, tags, audiences, media references, immutable creation time, prior
modification time, publication timestamp/state, and deletion timestamp/state.
Content edits, tag/audience/media changes, publish, unpublish, and delete are
versioned. Byte-identical or otherwise no-op writes create no revision.
Revisions are immutable through the storage API; controlled migrations and
whole-store backup/restore may rewrite them.

Revisions are owner-readable through list/detail history, including for a
deleted Post. They are not public, do not weaken normal Post authorization, and
provide no product revert operation. Their purpose is archival insurance and
inspection, not an accepted-write audit log.

Deleted Posts and their revisions are retained indefinitely under the current
policy. Media referenced by the retained current Post or any retained Post
Revision participates in the ordinary reference guard for active and deleted
Posts. Explicit forced media deletion remains an administrative override, so
archival reconstruction is not guaranteed after force. Idempotency and child
records remain attached. There is no product purge in this decision. Whether a
future privileged or retention-based purge exists is explicitly undecided and
requires a separate ADR that resolves revision, media, idempotency, and
reference erasure together.

Active permalink reuse also reuses the stable item identity derived from that
URL in Atom/RSS/JSON Syndication Feeds. Feed readers may conflate the
replacement with the deleted Post; this is an accepted cost of active-only
public identity while no restore is promised.

### D6 — domain and architecture projection

Add the following domain terms without conflating them with inbound `ajr_*`
content:

- **WebSub Publish Ping** — outbound notification to a configured WebSub Hub,
  naming a Syndication Feed URL as topic and carrying no content;
- **Deleted Post** — a locally authored Post retained under a tombstone but
  absent from active publishing surfaces;
- **Post Revision** — an immutable prior full-state snapshot of a locally
  authored Post, distinct from an AtomPub Entry and inbound
  `ajr_entry_versions`.

Clarify that an AtomPub Collection contains active Posts and that a Post's
public permalink identity is active-state identity. Reserve **purge** for
physical removal; this ADR does not adopt a purge policy.

`docs/ARCHITECTURE.md` cites all four drafts by path, replaces the current
un-ADR'd statements, and names every implementation mismatch with its follow-up
issue. It must not present an unimplemented invariant as current behavior.

### D7 — implementation debt is separate

No Rust, SQL, route, or browser behavior changes under #937. Before the ADRs are
accepted, file focused follow-up issues covering every observed mismatch,
including at minimum:

- protocol-independent transactional feed invalidation, visibility-before-
  ranking, and hub-configuration invalidation/coherent configuration snapshots;
- independent WebSub regeneration/publish retries, HTTP failure classification,
  `Retry-After`, dead-letter inspection, and redrive;
- HybridWindow setting activation, checked extremes, and corrupt-value errors;
- complete ETag identity, RFC conditional parsing/precedence, complete 304/HEAD
  behavior, and persisted representation-modification time;
- full-state meaningful-change revisions, no-op suppression, owner read-only
  history, and prior timestamps;
- deleted-Post and revision media retention, forced-delete interaction, and any
  related lifecycle/storage corrections.

Each architecture deviation cites the issue that will remove it. Existing bugs
found during investigation—AtomPub feed staleness, tombstone idempotency replay
returning a Deleted Post as 200, incoherent worker/regenerator configuration
reads, and tests that claim revision creation without reading
`post_revisions`—must be captured rather than silently lost.

## Acceptance criteria

- **AC1 — four decisions are accepted.** Four promoted ADRs exist with the
  boundaries in D1. Each records context, decision, rejected alternatives or
  tradeoffs, consequences, governing ADR constraints, and issue #937.
- **AC2 — WebSub policy is complete.** Its ADR states topic coverage, the
  public-projection trigger, protocol parity, transactional outbox,
  cache-before-ping order, at-least-once delivery, configuration transitions,
  malformed-hub versus read-error behavior, retry classification, separate
  regeneration/publish budgets, both dead-letter classes, and redrive. It
  explicitly distinguishes publisher-side WebSub from ADR-0010 inbound delivery.
- **AC3 — HybridWindow policy is complete.** Its ADR states the union rule,
  visibility-before-ranking, deterministic ordering, 20/30 defaults, inclusive
  fixed-day cutoff, regeneration-snapshot semantics, durable invalidation before
  a successful setting mutation returns, checked all-history behavior, corrupt-
  value failure, and the narrow ADR-0102 supersession.
- **AC4 — HTTP validation policy is complete.** Its ADR states deterministic
  complete strong tuple identity and stability, serializer/version constraints,
  RFC 9110 `If-None-Match` comparison/list/wildcard/precedence behavior,
  distinct GET/HEAD 200 and 304 headers/body rules, representation-time change
  and no-op stability, and the 300-second cache policy.
- **AC5 — lifecycle policy is complete.** Its ADR states active-surface deletion
  behavior, active-only permalink and syndicated-item identity reuse, the exact
  full revision field boundary and meaningful-change rule, owner-only read
  access without revert, storage-API immutability, indefinite current retention,
  current-and-revision media protection with forced-delete override, and the
  explicitly undecided future purge policy. It distinguishes local revisions
  from ADR-0009 consumed-content history.
- **AC6 — implementation debt is actionable.** Every known mismatch between the
  accepted decisions and production behavior has a focused open issue. The ADR
  consequences and architecture view link those issues; none describes the
  mismatch as delivered behavior.
- **AC7 — the materialized view is truthful.** `docs/ARCHITECTURE.md` cites all
  four accepted ADRs, removes the four #937 bullets from `Un-ADR'd reality`,
  states current behavior separately from accepted target where they differ, and
  leaves #938 entries untouched.
- **AC8 — ubiquitous language is updated.** `CONTEXT.md` defines WebSub Publish
  Ping, Deleted Post, and Post Revision; scopes Collection and permalink
  identity correctly; and preserves the inbound/outbound naming boundary.
- **AC9 — documentation gates pass.** The repository's ADR format, generated
  index parity, architecture-view parity, links, formatting, and full
  documentation validation pass after draft promotion.

## Out of scope

- Implementing the follow-up behavior under issue #937.
- Inbound WebSub subscriptions or any `ajr_*` ingestion policy.
- Exactly-once remote delivery.
- Multiple hubs, per-user hubs, or a new hub discovery mechanism.
- Replacing the hybrid union with count-only, time-only, or a hard cap.
- A Post restore/revert operation.
- Choosing or implementing hard-purge policy.

## Risks

- **Accepted target precedes implementation.** The architecture view could lie
  if it states the target as current. AC6–AC7 require explicit, linked deviation
  notes until each follow-up lands.
- **Tuple-derived ETags are brittle.** A new serializer input can be omitted.
  The ADR makes serializer revision, complete ordered input coverage, and
  identical-input stability invariants; body hashing remains the simpler
  rejected alternative.
- **Full-state revisions are materially larger than today's rows.** This is the
  cost of reconstructible archival history; no-op suppression limits needless
  growth.
- **Active permalink reuse prevents uncomplicated restore.** Accepted because no
  restore is promised. Any later restore design must resolve collisions as a new
  decision.
- **Purge remains open.** Indefinite current retention is explicit, not an
  accidental permanent promise; any future purge must supersede this boundary
  with a complete erasure policy.
