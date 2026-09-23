# ADR-DRAFT: Publish Member ETags in AtomPub Collection Entries

- Status: proposed
- Date: 2026-09-23
- Issue: [#1636](https://github.com/jaunder-org/jaunder/issues/1636)

## Context

The Emacs Protocol Client inventories a paginated AtomPub Collection before
classifying local Posts. Collection Entries identify Posts and expose their
slugs and audience targets, but do not advertise the strong validator used by
the Member resource. Reconciliation therefore makes a separate synchronous
Member `GET` for each locally matched Post, primarily to read its `ETag` header.
With many Posts, the request fanout makes opening or refreshing a report feel
stalled.

The validator already exists as a deterministic function of mutable Post content
and the complete audience target set. Collection rendering loads these inputs,
as does Member `GET`. A separate endpoint or persisted validator would duplicate
authority. Preview classification must not confer permission to mutate later:
batch operations continue revalidating remote and local state.

## Decision

Add exactly one direct `<j:etag>` extension to each Entry in the authenticated,
paginated Posts AtomPub Collection. The expanded name is
`{https://jaunder.org/ns/atompub}etag`; its sole text is the exact strong,
quoted ETag returned in the corresponding Member `GET` HTTP header, with no
attributes or padding whitespace. Derive it from the existing Member ETag
function using the same content and audience inputs. It describes the Member's
mutable representation, not the Collection page or Entry serialization. Ignore
any incoming `j:etag` on writes; a Member response continues carrying its
validator in the HTTP header rather than duplicating the Collection extension.

Advertise the additive `member-etag` feature token in the existing version-1
Jaunder Service Document extension. A client may use a valid Collection Entry
value without separately fetching the Service Document. A client without the
extension, or receiving absent, duplicate, weak, malformed, attributed, nested,
whitespace-padded, or namespace-spoofed values, uses a Member `GET` instead.
This is a public Atom foreign-markup extension, not an Emacs-specific transport;
unaware clients may ignore it as with
[ADR-0023](../0023-atompub-jaunder-wire-extensions.md). Keep Atom document I/O
delegated to the namespace-aware upstream model as required by
[ADR-0089](../0089-upstream-atom-document-io.md) and
[the namespace-aware fork decision](../0172-temporary-atom-namespace-fork.md).

The Collection validator accelerates _report classification only_. Pull, push,
and delete retain fresh remote checks, conditional requests, explicit user
selection, and local preflights. This is not a snapshot protocol across
paginated Collection pages.

## Consequences

Collection rendering computes one validator per returned Post but needs no
storage migration or per-Member HTTP read; a client can classify matched Posts
with page-scale rather than matched-Post-scale request count. Older servers
remain usable through the existing Member-read fallback, and other AtomPub
consumers can adopt the same published contract. An interrupted or concurrently
modified Collection can still yield a stale preview; operation-time checks
remain authoritative. Progress feedback for synchronous report construction
remains necessary even with fewer requests. Generic Syndication Feeds and Member
write semantics do not change.
