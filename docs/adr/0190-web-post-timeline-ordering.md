# ADR-0190: Web Post timelines order by publication time

- Status: accepted
- Date: 2026-09-10
- Issue: [#1428](https://github.com/jaunder-org/jaunder/issues/1428)

## Context

Web Post cards display their publication timestamp, but timeline keyset queries
and cursors historically used the Post's creation timestamp. Creation and
publication can diverge when a draft is published later, a publication time is
edited, a Post is scheduled, or fixture data supplies them independently. The
result can look chronologically inconsistent: a card dated July 11 can precede a
card dated July 12 even though the query is correctly ordered by its hidden
creation key.

Jaunder has several web Post timelines backed by the same pagination model: the
site timeline, authenticated home timeline, User timeline, site-tag timeline,
and User-tag timeline. They need one coherent ordering contract. Public variants
are also rendered first by the anonymous projector and then adopted by the CSR
client, so URL state must produce the same ordered representation on both sides.

Cursor pagination remains required by ADR-0004. The choice is whether to retain
the efficient but invisible creation-time key, sort client-side within one page,
or make the keyset match the publication chronology viewers see. Client-side
sorting cannot order across page boundaries and would diverge from the
projector, so it cannot satisfy the contract.

## Decision

All web Post timelines order on the Post's publication timestamp and stable Post
ID. Newest uses both keys descending; Oldest uses both keys ascending. The
cursor carries the publication-time key and is bound to its ordering direction,
so a continuation from one direction cannot be consumed as the other. Because
publication time is editable, a Post may move relative to a cursor during an
in-progress pagination walk. Keyset stability is guaranteed for an unchanged
ordered data set; this decision does not introduce snapshot isolation across
publication-time mutations.

Viewer selection is URL-addressed. Newest is the canonical default and omits an
ordering query parameter; `order=oldest` selects Oldest. Unknown values safely
resolve to Newest. The state is not persisted as an account, site, cookie, or
browser preference.

Every affected web timeline exposes the same compact sort-direction icon button;
its accessible name and tooltip communicate the active order and toggle action.
Changing order starts that timeline from its first page. Public projector
operations parse the same order and embed it with the ordered seed consumed by
the CSR client. The projected representation remains anonymous and cacheable;
the complete URL distinguishes variants, and projector/CSR coincidence remains
required.

This decision applies only to web Post timelines. It does not alter Syndication
Feed chronology, AtomPub Collection ordering, drafts, or other management
listings.

## Consequences

- The visible timestamp and ordering key agree across every page and web Post
  timeline.
- Both storage backends need direction-aware keyset predicates and ordering,
  with deterministic dual-backend boundary coverage.
- Public timeline cache and ETag identity naturally vary with the complete URL,
  while remaining viewer-independent.
- Direct Oldest loads require projector-aware query parsing; deferring the
  choice until CSR mount is prohibited because it would visibly reorder Posts.
- Existing cursors are not a compatibility promise: continuation cursors are
  opaque and valid only for the order and deployed version that produced them.
- Adding more order modes later requires an explicit extension of the URL,
  cursor, storage, projector, and control contracts rather than client-only
  sorting.
