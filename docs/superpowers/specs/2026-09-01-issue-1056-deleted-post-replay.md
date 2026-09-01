# Deleted Post idempotency replay

Issue: [#1056](https://github.com/jaunder-org/jaunder/issues/1056)

## Outcome

Retrying an AtomPub create request with an idempotency key whose original Post
has since been deleted returns the ordinary deleted-resource response. A Deleted
Post is never serialized back to the Protocol Client as a successful replay.

## Load-bearing decisions

- ADR-0136 remains authoritative: a Deleted Post is absent from active AtomPub
  surfaces, and active lookup reports absence with `404 Not Found`, not a public
  tombstone or `410 Gone`.
- A retained idempotency record remains bound to its original Post. Deleting the
  Post does not release a still-current key for reuse by another create.
- Replay applies the same active-Post boundary as Member GET and Collection
  membership before constructing a Member Entry response.
- The `404 Not Found` response does not reveal whether the retained Post is
  missing, foreign, or deleted. Owner and non-owner authorization boundaries
  remain unchanged.
- ADR-0167 qualifies ADR-0136's retention rule: an idempotency mapping is live
  for one hour after creation and expires when `cutoff <= now`. Another user may
  independently use the same key text, and a request after that cutoff may
  create a replacement.
- This is implementation debt under the accepted local Post lifecycle. It does
  not introduce a new domain term or architectural decision.

## Acceptance

- On both SQLite and PostgreSQL, creating a Post with an idempotency key,
  deleting that Post, and replaying the key returns `404 Not Found` and never a
  `200 OK` Member Entry.
- The replay does not create a replacement Post while the retained idempotency
  mapping is live under ADR-0167's one-hour cutoff.
- Existing dual-backend integration behavior remains coherent: the first delete
  succeeds, repeated delete and Member GET report absence, and Collection
  listing omits the Deleted Post.
- Cross-user behavior remains observable and unchanged: an authenticated user
  targeting another user's Collection or Member receives `403 Forbidden`, while
  posting equal key text to their own Collection creates an independent
  `201 Created` Post. Neither response exposes the deleted owner's Member Entry.
- Active-Post replay still returns the original Member Entry, and a request
  after ADR-0167's expiry cutoff may create a replacement.

## Boundaries

- No change to Post Revision fidelity, media retention or reference protection,
  Syndication Feed invalidation, idempotency expiry, or physical purge.
- No new restore operation, public tombstone representation, or privileged
  Deleted Post read surface.
- No schema, migration, AtomPub representation, or HTTP status-policy change
  beyond enforcing the existing active-resource boundary on replay.
