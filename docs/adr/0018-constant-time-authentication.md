# ADR-0018: Timing-Equalized Authentication (Username-Enumeration Resistance)

- Status: accepted
- Deciders: mdorman, Claude Opus
- Date: 2026-06-13
- Amended: 2026-06-26 — added the scope boundary vs. ADR-0022 (see _Scope
  boundary_ below); the original decision and durable invariant are unchanged.

## Context and Problem Statement

`UserStorage::authenticate` (both the SQLite and Postgres implementations)
returned `InvalidCredentials` **immediately** when the username was not found,
but ran a full Argon2id verification (tens of milliseconds) when the user
existed and the password was wrong. The error _value_ was uniform, but the
**response time** was not — giving a remote attacker a reliable oracle to
enumerate valid usernames. (analysis §2.1; complements ADR-0007, which covers
the authentication mechanisms themselves.)

## Decision Drivers

- Do not leak account existence through response timing.
- Keep the fix minimal and local to the storage layer.
- Apply identically to both backends (backend parity).
- The protection must **survive the SQLite/Postgres `authenticate` dedup**
  (analysis §1.1) without being "optimized" away.

## Decision Outcome

On the absent-user path, perform an Argon2id verification against a fixed, valid
**dummy hash** before returning `InvalidCredentials`, so the
present-and-wrong-password path and the absent-user path take comparable time.

- The dummy hash is produced by `storage::helpers::dummy_password_hash()`,
  computed **once** (via `OnceLock`) by hashing a fixed throwaway password with
  `Password::hash()`. Computing it through the real hashing path guarantees it
  carries the **same default Argon2 parameters** as production hashes, so the
  dummy verification costs the same as a genuine one.
- Initialization is infallible (no `unwrap`/`expect` in production): a hardcoded
  valid Argon2id hash is used as a fallback if runtime hashing ever fails.
- The verification result on the absent path is intentionally discarded — the
  path always rejects with `InvalidCredentials`.
- Applied to both `SqliteUserStorage::authenticate` and
  `PostgresUserStorage::authenticate`.

### Durable invariant

**The absent-user authentication path MUST perform an equalizing Argon2
verification.** Do not remove it as a "fast path" optimization, and preserve it
when the two backends' `authenticate` bodies are merged under analysis §1.1
(recorded on `jaunder-kq8w.3`, pre-GitHub bead tracker).

### Scope boundary

This ADR governs validating an **enumerable identifier** (username/email), where
response timing must be equalized so it cannot reveal account existence.
Validating a **high-entropy secret** (invite code, password-reset token) is the
opposite case and is governed by **ADR-0022**: there, a cheap rejection of an
invalid secret _before_ the Argon2 work is both safe (no useful timing oracle
exists against a ~256-bit space) and preferred (it bounds DoS amplification and
preserves capability-issuance as a throttle). Do not apply this ADR's
equalizing-dummy-hash rule to high-entropy-secret paths.

### Alternatives considered

- **Rate-limiting / lockout** — orthogonal; reduces brute-force throughput but
  does not close the per-request timing oracle. May still be added separately.
- **Accept the risk** — rejected; username enumeration is a real disclosure with
  low fix cost.
- **A generic constant-time wrapper around `authenticate`** — overkill for a
  single hot path; the dummy-verify is the standard, well-understood mitigation.

## Consequences

- Good: closes the enumeration oracle cheaply; the change is localized to the
  storage layer and the shared `helpers` module.
- Cost: one extra Argon2id hash on failed/absent logins. This is the same cost a
  real failed login already pays, and failed logins are not a hot success path.
- Scope/limitation: this is **timing _equalization_, not a formal constant-time
  guarantee**. Residual differences (the DB lookup itself, allocator/cache
  effects) remain, but they are small relative to the dominant Argon2 cost. If a
  stronger guarantee is ever required, revisit alongside rate-limiting.
