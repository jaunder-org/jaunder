# ADR-0138: Validate cached Syndication Feed representations conditionally

- Status: accepted
- Date: 2026-08-14
- Issue: [#937](https://github.com/jaunder-org/jaunder/issues/937)

## Context

Jaunder serves materialized RSS, Atom, and JSON Syndication Feed bodies with an
ETag, Last-Modified, and `Cache-Control: public, max-age=300`. The shipped ETag
is derived from a semantic tuple and conditional handling accepts only a narrow
happy path. The tuple omits representation inputs, while Last-Modified is based
on item timestamps. Either can therefore validate a changed serialized
representation falsely.

Conditional requests are representation protocol, not feed membership policy.
They warrant a separate decision from the hybrid item window.

## Decision

Each cached Syndication Feed representation has a strong ETag that is a
deterministic function only of a complete, ordered semantic identity tuple plus
serializer revision. The tuple includes every input capable of changing
serialized bytes, including feed format. Any byte change must change the tag;
identical semantic inputs and bytes keep the tag across regeneration. Serializer
revision covers behavior changes in the
[ADR-0015](0015-atompub-serialization-surfaces.md)- and
[ADR-0089](0089-upstream-atom-document-io.md)-governed paths, including upstream
wire-layout changes. Hashing the completed body is simpler to keep complete, but
tuple derivation is retained deliberately; completeness is therefore a
maintained invariant, not an assumption.

For GET and HEAD, `If-None-Match` follows RFC 9110 weak-comparison semantics,
accepts comma-separated tags and `*`, and takes precedence whenever it is
present. Malformed conditions cannot produce a false 304. For GET, a non-match
returns 200 with the body; for HEAD, a non-match returns 200 with GET-equivalent
headers and no body. A match returns 304 with no body and with the current
validators and applicable cache metadata for either method.

Last-Modified remains a weak, whole-second date validator. It is backed by a
persisted representation-modification time changed only when representation
identity changes, including metadata-only changes, removals, and empty-feed
changes. Identity/byte-identical regeneration leaves it unchanged. It is not
derived as `max(item.updated_at)`. `If-Modified-Since` is evaluated only when
`If-None-Match` is absent. Same-second precision limits are accepted as part of
HTTP-date validation; the ETag is authoritative when both are available.

`Cache-Control: public, max-age=300` remains the downstream freshness policy. It
controls protocol-client revalidation and does not trigger or promise
server-side cache regeneration.

## Consequences

Every serializer input and serializer behavior revision must participate in the
semantic identity tuple. Omitting one is a correctness bug that can cause a
false 304. Body hashing remains the rejected lower-maintenance alternative.

Persisting representation-modification time requires cache schema and write
changes. It gives removals, metadata changes, and empty feeds a truthful date
validator instead of borrowing an item timestamp.

Current production behavior deviates from this decision:
[tuple completeness, representation time, 304 metadata, and conditional parsing](https://github.com/jaunder-org/jaunder/issues/1054)
remain implementation debt.
