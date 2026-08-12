# ADR-0114: Absent-user timing equalization via a dummy Argon2 hash

- Status: accepted
- Date: 2026-08-11

## Context

Authentication must not reveal whether a username exists. If the absent-user
path returns without hashing, its response time is measurably faster than a real
verification — a timing oracle (§2.1).

## Decision

The absent-user path verifies the supplied password against a dummy Argon2 hash
so both paths do the same work:

- The dummy hash is produced at runtime with the active Argon2 parameters, so
  its verify cost matches real hashes (parity is asserted by
  `dummy_password_hash_matches_real_hash_parameters`).
- If runtime hashing fails, a hard-coded fallback constant is used. It must also
  be a well-formed hash — a fast `Err` would reintroduce the oracle.
- **Accepted limitation:** the fallback carries _production_ Argon2 parameters,
  so under a `cheap-kdf` build (the coverage derivation) its timing parity is
  not exact, and no parameter-parity test exists for it — asserting parity there
  would assert something false. This is inherent to hard-coding and is why the
  fallback is a last resort, not the primary path.

## Consequences

- Both dummy hashes are covered by verifiability tests; parity is asserted only
  for the runtime one.
- Changing Argon2 parameters in production requires regenerating the fallback
  constant.
