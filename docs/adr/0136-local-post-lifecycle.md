# ADR-0136: Retain local Posts through soft deletion and full revisions

- Status: accepted
- Date: 2026-08-14
- Issue: [#937](https://github.com/jaunder-org/jaunder/issues/937)

## Context

Local Post updates currently copy five core fields into `post_revisions`, and
soft deletion stamps `deleted_at` while active reads omit the row. The revisions
are write-only, omit summary, tags, audiences, media, publication, and deletion
state, and are created only by full updates. Deleted Posts stop protecting media
and release their permalink for reuse. None of this has a local lifecycle ADR.

[ADR-0009](0009-edit-delete-policy.md) is a near-miss: it governs updates and
deletions received for consumed content. It does not govern locally authored
Posts or `post_revisions`. Native source retention remains required by
[ADR-0015](0015-atompub-serialization-surfaces.md), and active Post visibility
remains governed by
[ADR-0020](0020-content-visibility-and-subscription-model.md).

## Decision

A local Post row is durable canonical identity and latest state. Soft deletion
stamps a tombstone and removes the Post from active web reads, public
Syndication Feeds, and AtomPub Collections. Active lookup behaves as absent (404
or omission), not as a public tombstone or 410.

Public permalink identity is active-only. A new Post may reuse a deleted Post's
permalink while the retained tombstone remains identifiable by its internal Post
ID. Atom/RSS/JSON Syndication Feed item identity derived from that permalink is
therefore also reused; feed readers may conflate the replacement with the
deleted Post. This accepted cost preserves active-only identity while no restore
is promised, and any future restore design must resolve collisions explicitly.

Before every meaningful state change, storage appends a full-state Post Revision
of the prior state: authored source and format, rendered representation, title,
slug, summary, tags, audiences, media references, immutable creation time, prior
modification time, publication timestamp/state, and deletion timestamp/state.
Content edits, tag/audience/media changes, publish, unpublish, and delete are
versioned. Byte-identical or otherwise no-op writes create no revision.
Revisions describe state transitions, not every accepted request.

Post Revisions are immutable through the storage API. Controlled schema
migrations and whole-store backup/restore may rewrite them. An owner may list
and inspect revision history, including for a deleted Post. Anonymous users and
other owners may not. No product revert operation is established.

Deleted Posts, their revisions, idempotency records, and child relationships are
retained indefinitely under the current policy. Media referenced by either the
retained current Post or any retained Post Revision participates in the ordinary
reference guard for active and deleted Posts. Explicit forced media deletion
remains an administrative override, so reconstructibility is not absolute after
force. There is no product purge in this decision. Whether a future privileged
or retention-based purge exists is explicitly undecided and requires a separate
ADR resolving Post, revision, media, idempotency, and child-reference erasure as
one policy.

## Consequences

Revision rows become larger and more numerous than today's core snapshots. No-op
suppression avoids weightless history. Owner read access requires list/detail
storage and product surfaces with authorization that does not reuse public
visibility as a substitute.

Soft deletion is not erasure. Operators and users must not be told that Delete
physically removes content. Whole-store backup includes tombstones and revision
history. A future purge or legal-erasure design remains possible but must be a
new decision rather than an accidental `DELETE` statement.

Active permalink and syndicated-item identity reuse is preserved, so restoration
is not promised and feed-reader conflation is accepted. Retaining ordinary media
protection trades storage reclamation for reconstructible archival history
unless an administrator explicitly forces deletion.

Current production behavior deviates from this decision:
[revision fidelity, no-op suppression, owner history, and retained media references](https://github.com/jaunder-org/jaunder/issues/1055)
remain incomplete.
[AtomPub idempotency replay can expose a Deleted Post](https://github.com/jaunder-org/jaunder/issues/1056).
Those are implementation debt, not lifecycle policy.
