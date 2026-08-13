# ADR-0134: Lowercase-canonical usernames

- Status: accepted
- Date: 2026-08-13
- Issue: [#936](https://github.com/jaunder-org/jaunder/issues/936)

## Context

A username is both a local account identifier and part of public URLs and
protocol credentials. Preserving multiple spellings for the same account would
force case-folding rules into persistence, lookup, routing, serialization, and
HTTP Basic verification. Unicode case folding would add normalization and
confusable-identifier policy that the product does not otherwise need.

The domain-value convention in
[ADR-0063](0063-domain-value-newtype-convention.md) requires one validating and
normalizing `FromStr` boundary, but it does not decide this value's grammar or
canonical representation.

## Decision

A `Username` is a case-insensitive local account identifier with canonical form
restricted to non-empty ASCII `[a-z0-9_-]+`.

`Username::from_str` is the single normalization and validation boundary. It
accepts mixed-case ASCII input by lowercasing it, then rejects every character
outside the canonical grammar. Serde deserialization, owned conversion, request
arguments, and storage decoding route through the same validated newtype rather
than pre-normalizing strings at their entry points.

The lowercase canonical form is the only username identity. It is stored,
compared, serialized, displayed, and embedded in URLs. Equality is direct value
equality; callers do not perform a second case-insensitive comparison. This also
makes HTTP Basic username matching case-insensitive in effect: both the supplied
name and the stored name are canonical `Username` values before comparison.

We reject case-preserving display names as a second spelling of `Username` and
reject Unicode identifiers/case folding. A future display-name feature would be
a separate domain value and would not change account identity.

## Consequences

- `Alice`, `ALICE`, and `alice` all resolve to the same canonical username
  `alice`; they cannot identify separate accounts.
- Database keys, wire values, rendered usernames, and URL path segments share
  one stable representation with no per-layer folding rules.
- Usernames cannot contain non-ASCII letters, spaces, dots, or other
  punctuation.
- UI lowercasing may remain a presentation convenience, but correctness cannot
  depend on it; the `Username` boundary owns normalization.
- Expanding the grammar or introducing case-preserving identity is a migration
  and architecture change, not a parser-local adjustment.
