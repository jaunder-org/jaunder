# ADR-DRAFT: Media Upload Capability Is Site-Wide

- Status: proposed
- Date: 2026-09-04
- Issue: [#552](https://github.com/jaunder-org/jaunder/issues/552)

## Context

Media byte and quota limits are positive quantities. Before their invariants
were typed, a stored zero accidentally rejected every upload and acted as an
undocumented feature switch. Restoring that behavior would again conflate a
limit with whether the operation exists.

Jaunder accepts media through both its web interface and the AtomPub Media
Collection. Both paths converge on the same media manager, while the AtomPub
Service Document and browser controls separately advertise the operation. A
web-only switch would therefore leave Protocol Clients with different authority,
and hiding controls alone would not secure direct requests.

## Decision

Jaunder has one site-wide Media Upload Capability, configured by the closed
boolean key `media.uploads_enabled`. Absence preserves current behavior and
means enabled. Invalid physical data fails closed and means disabled, while
storage failures propagate. The existing positive maximum-file-size and quota
settings remain limits only.

The shared media manager evaluates the capability once at upload entry, before
streaming or mutation. It returns a typed disabled error that both the web and
AtomPub public boundaries map to `403 Forbidden`. An upload already admitted may
finish after an operator disables the capability; new attempts are rejected.
Retrieval and deletion of existing media are unaffected.

Discovery projects, but does not enforce, the same policy. The browser media
page becomes read-only with an explicit notice and no upload controls. The
AtomPub Service Document omits the Media Collection. Direct web and AtomPub
upload handlers still delegate to the manager's authoritative check.

Operators control the capability through two views of the same site-config key:
the typed `site-config` CLI and a separate Media Uploads card on `/admin/site`.
The card owns an independent read/write action rather than extending the site
identity payload.

## Consequences

Every upload ingress has identical policy and failure semantics without
repeating authorization in transport adapters. Protocol Clients can discover
that media creation is unavailable, and users can still inspect or delete
existing media from the browser.

Failing closed on malformed stored configuration may disable uploads after
out-of-band database corruption, but it never silently expands capability.
Because the check is an entry snapshot rather than a live cancellation signal,
configuration changes are cheap and deterministic and do not introduce streaming
coordination or temporary-file cleanup races.
