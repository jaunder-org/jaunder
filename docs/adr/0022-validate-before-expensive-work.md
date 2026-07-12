# ADR-0022: Validate cheaply before expensive work when the gate is a high-entropy secret

- Status: accepted
- Date: 2026-06-26
- Deciders: Michael Alan Dorman

## Context

Several storage operations gate an expensive computation behind a credential
check. The expensive step is almost always an Argon2id password hash (tens of
milliseconds, deliberately). Two different gates call for opposite orderings,
and conflating them creates either a username-enumeration oracle or a
denial-of-service amplifier.

- `UserStorage::authenticate` gates on a **username** — a low-entropy,
  user-chosen, enumerable identifier. Returning early when the username is
  absent leaks account existence through response timing. ADR-0018 resolves this
  by performing an equalizing dummy Argon2 verification on the absent path.

- `create_user_with_invite` and `confirm_password_reset` gate on a
  **high-entropy secret** — an invite code or password-reset token, each 32
  cryptographically random bytes (`auth::generate_token`, ~256 bits). The
  enumeration concern does not apply: a timing oracle gives no usable advantage
  against a 2^256 space. But hashing _before_ validating the secret turns every
  request bearing a bogus secret into wasted Argon2 work — a CPU-exhaustion
  amplifier — and, for invite-gated registration, destroys the one
  capability-based throttle available (stop issuing invites ⇒ the hashing
  surface drops to zero).

## Decision

When the value being validated is a **high-entropy secret** (not an enumerable
identifier), validate it with a cheap lookup **before** performing expensive
work (Argon2 hashing, large allocations, etc.), and reject invalid secrets
without paying that cost.

This is the deliberate complement to ADR-0018, not a contradiction of it. The
dividing line is the **entropy of the thing being validated**:

- **Enumerable identifier** (username, email): equalize timing — do the work
  anyway (ADR-0018).
- **High-entropy secret** (invite code, session/reset token): cheap-reject first
  — skip the work on invalid input (this ADR).

## Consequences

- `create_user_with_invite` validates the invite (a `SELECT`) before hashing;
  the SQLite implementation additionally takes its write lock up front per
  ADR-0021, so the hash runs inside the immediate transaction on the success
  path only (issue #51).
- `confirm_password_reset` currently hashes the new password _before_ validating
  the reset token — it violates this ADR and is tracked as a follow-up issue
  (since fixed: the atomic token claim now precedes hashing).
- ADR-0018 carries a scope-boundary cross-reference to this ADR so the boundary
  is discoverable from both sides; its decision and durable invariant are
  unchanged.
- This is about _cost ordering_, not correctness: both orderings produce the
  same result for valid input, and no timing guarantee for enumerable
  identifiers is weakened — those remain governed by ADR-0018.
- Relates to ADR-0007 (auth mechanisms) and ADR-0018 (timing-equalized auth).
