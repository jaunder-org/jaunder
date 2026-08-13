# ADR-0133: Hash bearer-equivalent tokens before persistence

- Status: accepted
- Date: 2026-08-13
- Issue: [#936](https://github.com/jaunder-org/jaunder/issues/936)

## Context

Session tokens and app passwords are bearer-equivalent secrets: possession is
enough to authenticate. Persisting a raw value would turn a database disclosure
into immediately usable credentials.

The domain distinguishes the presented `RawToken` from the stored `TokenHash`.
`RawToken` has redacting `Debug` output under the observability rule in
[ADR-0011](0011-unified-observability.md), and its string-newtype declaration
opts out of the sqlx bridge defined by
[ADR-0071](0071-sqlx-string-newtype-bridge.md). Binding a `RawToken` directly to
SQL therefore does not compile. `TokenHash` remains independently constructible
because database decoding, revoke forms, and trusted digest construction all
legitimately produce a hash when no raw token is available.

Issue [#554](https://github.com/jaunder-org/jaunder/issues/554) considered an
associated-type or type-state relationship between the pair. It could not encode
an additional true invariant: provenance does not survive persistence, the hash
must stand alone, and a trait would add syntax without preventing any state the
current types permit.

## Decision

Bearer-equivalent tokens are persisted only as a SHA-256 `TokenHash`, encoded as
base64url. Fresh session credentials are minted with
`host::token::generate_hashed`, which returns the raw token and its digest
together. Presented credentials cross the explicit
`host::token::hash(&RawToken) -> Result<TokenHash, _>` boundary before a storage
lookup or write.

`RawToken` remains a distinct type with no sqlx `Encode`/`Type` bridge and with
redacting `Debug`. No implicit conversion exists between `RawToken` and
`TokenHash`. The explicit hash-before-store call remains visible at the
host/storage boundary.

We reject the #554 associated-type/type-state design. The enforceable invariant
is directional: raw tokens cannot be bound to SQL and the sole raw-to-hash
conversion hashes them. It is not “every `TokenHash` has a live `RawToken`.”

## Consequences

- A database disclosure does not directly reveal active bearer credentials.
- Accidentally binding a raw token is a compile error, and accidental debug
  formatting redacts its body.
- Hashing is a fast SHA-256 digest, not a password KDF. Tokens carry 256 bits of
  randomness, so offline guessing is not the relevant threat.
- The type system forces raw values away from SQL but cannot force every caller
  to invoke the correct conversion before all non-SQL uses. Reviews and tests
  must still keep `host::token::hash` as the single raw-to-hash door.
- `TokenHash` remains independently parseable and decodable; tightening its
  trusted digest constructor is a separate decision.
