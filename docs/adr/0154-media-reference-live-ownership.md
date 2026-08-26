# ADR-0154: Verify media-reference ownership through instance identity

- Status: accepted
- Date: 2026-08-25
- Issue: [#744](https://github.com/jaunder-org/jaunder/issues/744)

## Context

[ADR-0090](0090-media-references-extracted-at-render.md) makes sanitized
rendered HTML authoritative for a Post's media references and deliberately made
matching host-blind. The path-only rule is safe but wrong: a foreign URL whose
path resembles Jaunder's media layout can block deletion of an unrelated local
file.

Configured-origin comparison still answers an assumption about routing, not the
underlying question: whether requesting the referenced URL reaches this Jaunder
instance. A live request can identify the serving instance directly. That
network answer cannot run inside atomic SQL, and its exact evidence lets storage
validate only rows present at deletion time.

## Decision

This decision amends ADR-0090 decisions 5, 6, and 8 and replaces its
pre-existing-content consequence. Every other part continues unchanged.

Each database owns one persistent public random UUID created atomically on first
open and preserved through backup/restore. Identity-only bootstrap state is
pristine for restore; a restored identity replaces it. Every response carries
exactly one canonical `X-Jaunder-Instance`, replacing any inner value.

Pure extraction retains the stored-media identity plus complete query-bearing
reference form and kind: local, absolute HTTP(S), or scheme-relative. Fragments
are removed. Legacy rows are transactionally re-derived from stored rendered
HTML before requests are served.

Relative references are local. Absolute and scheme-relative references are
resolved immediately before deletion with HEAD; scheme-relative forms inherit
only the current canonical `site.base_url` scheme. Resolver work loads all
references for the media globally and is bounded by request and whole-operation
timeouts, concurrency, and target deduplication.

The live adapter owns one shared ordinary reqwest client. Reqwest owns DNS,
configured proxies, redirects, TLS, and connection pooling; Jaunder does not
classify addresses, resolve or pin sockets, or implement redirects. The author
trust model does not treat Post-authored references as hostile network inputs.
If a product later disallows an address class, it must reject that URL at the
authoring validation boundary rather than alter deletion-time ownership checks.

A final response with exactly one canonical matching UUID is owned. True absence
or one canonical different UUID is foreign. Ambiguous headers, malformed UUIDs,
or request failures are unknown and fail closed. The public UUID is
identification rather than authentication: copying it can cause only
conservative refusal.

Foreign results become resolver-constructed evidence containing the exact
persisted row key—Post ID, media triple, reference kind, complete form—and the
expected instance UUID. The conditional delete exempts only current rows exactly
named by that evidence; local, legacy, owned, unknown, near-match, and
concurrent rows refuse. Owner reporting is derived from the same global evidence
set.

PostgreSQL Post create/update and media delete/reclaim operations take
transaction-scoped advisory locks in one stable media-key namespace and global
order: create locks proposed identities; update locks the old/new union;
delete/reclaim locks the target before conditional SQL. SQLite uses its
immediate/single-writer discipline. Probes finish before locks. The decision
remains one `DELETE … WHERE … NOT EXISTS … RETURNING` statement.

## Consequences

- The system answers live serving identity rather than inferring ownership from
  host spelling.
- Foreign servers need no Jaunder support: a completed unambiguous response
  without this instance UUID is foreign.
- Request uncertainty is conservative and may require owner `force`.
- Author-provided media URLs use the ordinary configured HTTP client. Any future
  disallowed-address product rule belongs at authoring validation, not the
  deletion-time ownership probe.
- Exact forms/evidence and shared PostgreSQL locks enlarge internal interfaces
  but keep network policy outside storage and atomicity inside it.
- A restored clone retains logical identity; simultaneous clones can
  conservatively recognize each other as owned.
- Scheme-relative ownership still needs a serving scheme; the configured
  canonical site scheme is the explicit choice.
- No ETag, nonce/HMAC secret lifecycle, request-derived scheme, or cached probe
  result is introduced.
