# ADR-DRAFT: Make AtomPub create intent durable across client restarts

- Status: proposed
- Date: 2026-09-17
- Issue: [#1565](https://github.com/jaunder-org/jaunder/issues/1565)

## Context

The Emacs Protocol Client creates a Post with an AtomPub Collection `POST` and
writes the returned Post ID into the local Org file. A response can be lost
after Jaunder commits but before the client learns that ID. The current client
retries only within one invocation under one ephemeral `Idempotency-Key`; a
later invocation generates a new key. The server also gives each key only a
one-hour semantic replay window under
[ADR-0167](../0167-bounded-transient-data-retention.md). After either boundary,
the local draft cannot distinguish the committed Post from an unrelated remote
Member, so retrying can create a duplicate.

Explicit batch push makes this residual window routine rather than exceptional:
a long selection must remain safely resumable after cancellation, Emacs exit,
transport loss, or delayed operator recovery. Post ID remains the canonical
local/remote join key, and a heuristic match by title, body, slug, or timestamp
would silently attach the wrong Post. The ID-first write-back ordering from
[ADR-0047](../0047-emacs-publish-orchestration.md) protects failures after a
response, but it cannot recover an ID from a response that never arrived.

## Decision

A successful keyed AtomPub Post create permanently consumes that
`(User, Idempotency-Key)` pair and maps it to the created Post ID. The mapping
has no semantic expiry. Reusing the pair while the original Post is active
returns that original Post as the existing `200` replay, regardless of the new
request payload, and never performs another create. If the original Post has
been soft-deleted, reuse returns `409 Conflict`: the key remains consumed and
cannot create a replacement. Missing, blank, or unreadable keys retain the
existing unkeyed-create behavior.

This durable mapping is Post-create correlation, not transient retry telemetry.
It is written only when the keyed create commits, so its growth is bounded by
successful keyed Posts rather than request volume. It follows the retained Post
identity lifecycle from [ADR-0136](../0136-local-post-lifecycle.md). This
decision supersedes only ADR-0167's one-hour semantic expiry and cleanup
eligibility for Post-create idempotency mappings; ADR-0167's other retention
policies stand.

Before its first create request, the Emacs Protocol Client durably records a
stable create intent in the local Post: the key, the request-content digest it
represents, and the attempt time. Every recovery attempt reuses that key until
the server-confirmed Post ID has been durably written. The client removes the
local intent only after the ID write succeeds.

A local edit after an indeterminate create does not mint a new key and does not
make the recovered remote representation appear synchronized. Recovery first
binds the returned ID, slug, and ETag to the local Post. If the current request
digest differs from the recorded intent, the file remains local-ahead and needs
an explicit conditional update; it is never silently marked equal to the
replayed representation.

## Consequences

A response-lost create can be recovered after any delay without creating a
second Post. Batch cancellation and process exit no longer turn an unknown
commit into an unsafe retry choice, and the same safety applies to ordinary
single-Post publishing because both paths share create orchestration.

Post-create mappings are durable storage proportional to successful keyed Posts.
Cleanup must no longer remove or semantically expire them. Backup and restore
continue to preserve the mapping with its Post identity. A deleted Post keeps
its key unavailable, matching the retained tombstone policy rather than allowing
accidental identity replacement.

Persisting create intent is the one deliberate exception to the prior rule that
publish performs network mutation before local metadata mutation. The intent is
non-destructive recovery metadata written before the network request; authored
content and identity-bearing write-back remain untouched until the server
responds. A changed local file after an uncertain create requires a later
explicit update, trading one extra action for an honest synchronization state.
