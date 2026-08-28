# Issue #1087 — couple cached Syndication Feed bodies to format

## Outcome

A cached Syndication Feed representation carries its format with its rendered
body, so Jaunder cannot serve an RSS, Atom, or JSON Feed body with a conflicting
path format or `ContentType`. Inconsistent stored rows fail closed instead of
being served or silently repaired.

## Load-bearing decisions

- Apply ADR-0063's invariant-first rule with a closed representation type in
  `common::feed`, not a weightless `FeedBody(String)` newtype. The closed type
  owns the format/body pairing and exposes the body, `FeedFormat`, and derived
  `ContentType` without allowing callers to assemble conflicting values.
- Syndication Feed renderers establish the representation variant at the point
  where they serialize RSS, Atom, or JSON Feed output. That producer door proves
  serializer provenance in memory. Storage readback cannot recover provenance
  from text alone; it proves only that the persisted path and content-type
  metadata agree, and makes no claim that the body is syntactically valid XML or
  JSON.
- `FeedCacheRow` is an owning invariant boundary rather than a public bag of
  independently constructible fields. Construction and storage readback require
  the `FeedPath` format, representation format, and `ContentType` to agree.
- Storage keeps the existing cross-backend schema. The body and content-type
  columns remain persisted as text; the typed mapping seam reconstructs the
  format-tagged representation only after the existing primitive columns are
  decoded and their metadata agreement is established.
- A mismatched stored row is a typed storage failure. Cache reads remain
  side-effect free: they neither treat corruption as a miss nor rewrite it, and
  handlers never serve the body under a guessed media type.
- Body syntax remains serializer-owned and opaque at cache read time. Jaunder
  does not reparse cached RSS, Atom, or JSON Feed payloads on every read.
- ETag, cache-validator, regeneration, and worker behavior remain unchanged
  apart from carrying the typed representation through their existing flow.
- This change applies ADR-0063 and closes the deliberate primitive review from
  issue #697. It introduces neither a new architectural decision nor a new
  glossary concept.

## Acceptance

- RSS, Atom, and JSON Feed regeneration each produce a representation whose
  reported format and derived `ContentType` match the requested `FeedPath`.
- `FeedCacheRow`, regeneration, storage bind/decode, and HTTP serving carry the
  typed representation end to end; no production cache-body field or argument
  remains an unqualified `String`.
- SQLite and PostgreSQL storage coverage proves coherent rows round-trip and a
  row whose stored `ContentType` conflicts with its `FeedPath` is rejected.
- Coverage proves callers cannot construct a cache row with a representation
  format that conflicts with its `FeedPath`, and handlers serve the body's
  derived `ContentType` on a valid cache hit.
- An integration-level corrupt-cache-hit check proves a metadata mismatch
  propagates as a failure: no cached body is served, regeneration is not
  invoked, and the stored row is not rewritten.
- The resulting code or decision record references issue #697 and ADR-0063.
- Repository static checks and the focused common, storage, and server feed
  tests pass.

## Boundaries

- No feed-cache schema migration or database constraint.
- No XML or JSON parser on cache reads and no promise to detect arbitrary body
  corruption.
- No automatic cache repair, corruption quarantine, or new observability.
- No change to Syndication Feed routes, wire formats, cache validators, event
  processing, or public HTTP semantics for coherent rows.
