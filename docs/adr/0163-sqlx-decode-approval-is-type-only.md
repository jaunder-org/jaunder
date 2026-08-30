# ADR-0163: SQLx decode approval is type-only

- Status: accepted
- Date: 2026-08-30
- Issue: [#1201](https://github.com/jaunder-org/jaunder/issues/1201)

## Context

[ADR-0085](0085-static-type-safety-gates-enumerate.md) requires static
type-safety gates to enumerate their population, deny unknown members, and fail
closed. Its `sqlx-newtype-decode` worked example approved declaration-backed
types but retained a second path: an exact, counted, reason-bearing allowlist
for legitimate primitive targets.

The allowlist eventually held 53 occurrences spanning database catalog metadata,
counts and existence probes, opaque payloads, persisted corruption states,
flags, counters, test identities, and custom row decoders. Its site and
multiplicity checks were honest, but a green gate still meant “valid by type, or
accepted at this site.” Primitive decode targets could therefore remain outside
the type model indefinitely.

Some values cannot decode directly into validated application-domain types.
Unknown site-config keys must remain visible to faithful export, invalid stored
session labels must reach repair-on-read policy, and a malformed feed URL must
remain identifiable long enough for the feed-event worker to divert that one
row. Eliminating exemptions therefore requires explicit persistence roles, not
pretending all stored text satisfies a domain grammar.

## Decision

`sqlx-newtype-decode` has no exemption path. Every structurally readable decode
leaf is approved only when it is:

- a declaration-backed type whose macro emits the SQLx bridge;
- an explicitly approved foreign type; or
- part of a composite whose leaves the gate independently polices.

Bare primitives, site markers, field attributes, marker traits, SQL-text
heuristics, receiver-name heuristics, and central or distributed exception
registries do not approve a decode.

Storage uses explicit private role types for intentional persistence
representations. A role with a real closed contract validates at decode; a role
whose contract is lossless preservation remains infallible until its existing
parse, repair, export, or deletion boundary. Shared private types are limited to
mechanics with genuinely identical contracts, currently row cardinality and
boolean existence. Catalog metadata, persisted payloads, counters, flags, config
values, and test identities remain concern-owned.

Custom conversion policy follows a typed intermediate-row pattern. SQLx first
decodes a derived row whose every leaf is approved; conversion then applies the
policy. Feed-event claims use this boundary so only an unparseable stored feed
URL enters the purge path, while every other decode failure rolls the claim
transaction back.

The gate continues to enumerate the same AST population, inspect no SQL, fail on
unreadable inputs, and audit its declaration-macro model. It still does not
prove SQL column-to-field correspondence or resolve types written only by later
use.

This decision narrows ADR-0085 for `sqlx-newtype-decode`; it does not supersede
ADR-0085's general enumeration rule or decide exemption policy for other gates.

## Consequences

A green decode gate now has one meaning: every enumerated leaf is valid by
construction under the declaration model. Adding a legitimate stored primitive
requires naming its semantic or persistence role, not editing the gate.

The storage layer gains small private types and explicit conversions. This is
intentional friction, but the types stay local and distinguish lossless stored
representations from validated domain values.

Count and attempt corruption fails at decode instead of normalizing silently.
PostgreSQL and SQLite existence probes expose one Rust boolean meaning. Claim
updates remain provisional until every non-URL field decodes, preventing a
failed read from leaving a row claimed.

The declaration model remains an incomplete static model by design. Its safe
failure direction is loud rejection of a legitimate new declaration form; the
macro audit names that omission instead of silently approving a primitive.
