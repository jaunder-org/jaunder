# ADR-DRAFT: User-wide current Content Rights

- Status: proposed
- Date: 2026-09-21
- Issue: [#1611](https://github.com/jaunder-org/jaunder/issues/1611)

## Context

Jaunder has no rights model for locally authored Posts. A declaration could be
stored as a per-Post snapshot or resolved from the author's current setting; it
could also be mixed into authored bodies or projected separately into public web
and Syndication Feed representations. These choices have lasting legal, storage,
cache-invalidation, and protocol consequences.

## Decision

A User owns one current, publication-wide Content License. It defaults to All
Rights Reserved and may instead be CC0 or any of the six Creative Commons 4.0
licenses, identified by the official SPDX identifier and canonical Creative
Commons URL. Changing it intentionally applies retroactively to every Post by
that User; Posts carry neither a license snapshot nor a per-Post override.

Every public Post projection carries a Copyright Declaration composed from the
Post's immutable creation year, the author's current Display Name or canonical
Username fallback, and the author's current Content License. Web presentation
renders that declaration as Post metadata. Syndication Feed serializers expose
it as item-level rights and license metadata without changing authored or
rendered Post content: native Atom fields, namespaced RSS fields, and a
`_jaunder` JSON Feed item extension. AtomPub Collections remain source-oriented
editing representations and do not expose the declaration.

Because Display Name and Content License changes alter concrete public feed
representations, each mutation atomically enqueues every affected Site, Site
Tag, User, and User Tag feed event. The existing publisher generation gate
regenerates those representations before duplicate-safe, at-least-once WebSub
publication.

## Consequences

A Display Name or Content License change updates the public rights presentation
of every Post by that User; feed-event enqueue failure therefore fails the same
mutation rather than leaving a stale cache. Existing and new Users read as All
Rights Reserved until they choose another value. Adding a future license is an
explicit closed-domain change rather than acceptance of arbitrary text, URLs, or
SPDX expressions.
