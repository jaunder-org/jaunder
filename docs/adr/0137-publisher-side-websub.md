# ADR-0137: Publish Syndication Feed changes through WebSub

- Status: accepted
- Date: 2026-08-14
- Issue: [#937](https://github.com/jaunder-org/jaunder/issues/937)

## Context

Jaunder already advertises one optional site-wide WebSub Hub in its outbound
RSS, Atom, and JSON Syndication Feeds and sends `hub.mode=publish` requests from
the feed worker. [ADR-0010](0010-protocol-integration.md) discusses the opposite
leg—future inbound subscription delivery—and does not govern this publisher
behavior. [ADR-0015](0015-atompub-serialization-surfaces.md) separates AtomPub
and Syndication Feed serialization but both project the same local Posts.
[ADR-0016](0016-dependency-injection-and-appstate.md) places and injects the
client; it does not decide notification semantics.

The shipped path has policy gaps. Web mutations enqueue coarse feed events,
AtomPub mutations do not, and mutation and enqueue commit separately. Hub
configuration changes do not invalidate cached discovery, and the worker and
regenerator can read different hub/site snapshots. Feed regeneration and remote
publishing share a retry budget; every HTTP error retries alike, `Retry-After`
is ignored, configuration access errors can become `NoHub`, and terminal work
has no supported redrive. Treating those accidents as architecture would make
public freshness depend on the authoring protocol.

## Decision

Jaunder is a WebSub publisher for every concrete public Syndication Feed URL:
Site, User, Site Tag, and User Tag surfaces in RSS, Atom, and JSON Feed. One
optional site-wide WebSub Hub is advertised in each representation. A WebSub
Publish Ping is an outbound form request naming that exact feed URL as its
topic; it announces a changed representation and carries no content.

A ping is earned by a protocol-independent change to at least one concrete
public Syndication Feed representation. Web and AtomPub mutations obey the same
rule. Draft, private, and no-op changes do not notify. Publication,
unpublication, deletion, due-time go-live, edits, and tag changes notify only
when their old/new public projection changes; tag changes affect the union of
old and new tag surfaces.

The Post mutation and affected-feed events commit atomically as a transactional
outbox. The asynchronous worker regenerates and commits the cache before
publishing. Delivery is duplicate-safe and at-least-once; exactly-once delivery
across HTTP and durable acknowledgement is rejected.

Setting, changing, or unsetting the hub invalidates all cached Syndication
Feeds. Queued work late-binds to one coherent snapshot of current hub and site
identity. With no configured hub, regeneration completes without sending and is
not replayed after later enablement. Consistent with
[ADR-0102](0102-config-key-closed-registry.md), a malformed stored hub URL is
purged and treated as unset; storage, configuration-access, and site-identity
read errors are retryable failure, not `NoHub`.

Feed regeneration and remote publish have separate bounded attempt budgets.
Transport failures, timeouts, 408, 429, and 5xx responses are retryable; other
4xx responses are terminal. A valid, bounded `Retry-After` controls the next
attempt. Exhausted regeneration work and exhausted or terminal publish work
remain separate inspectable dead-letter states with explicit redrive. Exact
attempt counts and delays are operational policy.

The WebSub client remains server-owned and constructor-injected under
[ADR-0016](0016-dependency-injection-and-appstate.md). Remote I/O never occurs
inside the storage transaction, consistent with
[ADR-0021](0021-sqlite-transaction-discipline.md) and
[ADR-0092](0092-sqlite-bounded-write-lock-occupancy.md). Scheduled go-live
continues to use the restart-durable transition established by
[ADR-0027](0027-scheduled-publishing-time-gated-visibility.md).

## Consequences

Authoring protocol no longer determines outbound feed freshness. The outbox
requires a deeper storage operation spanning the Post mutation and bounded feed
fan-out. Duplicate delivery remains possible and consumers must tolerate it.

Hub enablement starts from current regenerated state; it does not replay every
historical change. Hub removal stops remote publishing immediately while still
refreshing documents so discovery is removed.

Current production behavior still deviates from the decision in
[hub configuration, configuration snapshots, failure classification, `Retry-After`, retry budgets, and terminal recovery](https://github.com/jaunder-org/jaunder/issues/1052).
That deviation is implementation debt, not policy.
