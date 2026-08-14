# ADR-0131: Test-only cheap KDF fails closed

- Status: accepted
- Date: 2026-08-13
- Issue: [#936](https://github.com/jaunder-org/jaunder/issues/936)

## Context

Password hashing uses Argon2id at the crate defaults in production. The
test-only `cheap-kdf` feature substitutes minimum memory cost and one iteration
so routine tests do not spend most of their time hashing passwords. Linking that
feature into a deployed binary would make newly stored passwords materially
cheaper to attack.

Cargo resolver 2 and test-only dependency edges keep the feature out of normal
production builds, but dependency topology alone is not a sufficient safety
boundary: a future manifest change or nonstandard build could enable it.

Password verification does not need a feature branch. Argon2 parameters are
encoded in each PHC string, and verification derives its cost from the stored
hash. The deployment risk is therefore creation of weak hashes, not inability to
verify hashes created under another parameter set.

## Decision

`cheap-kdf` remains a test-only performance feature guarded by three
complementary layers:

1. Production dependency edges do not enable the feature.
2. `common` rejects `cheap-kdf` whenever `debug_assertions` are disabled with a
   compile-time error. An optimized artifact cannot be produced with cheap
   hashing enabled.
3. The `jaunder` binary checks `common::CHEAP_KDF_ENABLED` before parsing the
   CLI or starting application work and exits with a fatal error when it is
   true. This catches a debug artifact mistakenly deployed as production.

The compile-time and startup guards are intentionally redundant because they
cover different artifact classes. Neither replaces the other. Verification
continues to derive parameters from the stored PHC string, so production and
test hashes remain readable without a verification-mode branch.

## Consequences

- An optimized build carrying `cheap-kdf` fails to compile; a debug binary
  carrying it refuses to start before any command runs.
- Tests retain fast password hashing without changing production defaults.
- Dependency isolation, the compile guard, and the startup guard are all
  load-bearing. Removing one requires reopening this decision rather than
  treating the remaining layers as equivalent.
- The startup check deliberately terminates the process and is covered by an
  integration test that executes the built binary.
