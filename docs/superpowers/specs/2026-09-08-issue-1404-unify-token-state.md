# Unify token-state classification

Issue: [#1404](https://github.com/jaunder-org/jaunder/issues/1404)

## Outcome

Storage uses one shared representation and classifier for invitation,
email-verification, and password-reset token state. The refactor removes
duplicate invitation-only machinery without changing any observable claim or
error behavior.

## Load-bearing decisions

- Retain `TokenStateRow`, `TokenState`, and `classify_token_state` as the shared
  storage-owned classification path.
- Remove the invitation-specific row representation and classifier completely;
  provide no alias or compatibility path.
- Classify any row with `used_at = Some(_)` as `AlreadyUsed`, regardless of the
  timestamp value.
- For an unused row, classify `expires_at <= now` as `Expired`; only
  `expires_at > now` is `Claimable`.
- Preserve the write-first conditional-claim flow required by ADR-0021: a failed
  claim reads current token state and applies caller-owned fallback semantics.
- Preserve invitation fallback mapping of a still-`Claimable` row to
  `UseInviteError::AlreadyUsed`, representing a concurrent successful claimant.
- Preserve email-verification and password-reset fallback mapping of a
  still-`Claimable` row to each operation's `Expired` error.
- Keep those differing error policies at their existing caller-owned boundaries
  rather than introducing a shared error policy.

## Acceptance

- `storage/src/helpers.rs` contains exactly one token-state row representation
  and one classifier covering `Missing`, `AlreadyUsed`, `Expired`, and
  `Claimable`.
- Invitation, email-verification, and password-reset storage paths all consume
  the retained representation and classifier.
- One exhaustive pure test matrix covers an absent row, any used row, expiry
  exactly at `now`, expiry before `now`, and expiry after `now`.
- Invitation behavior remains unchanged for not-found, already-used, expired,
  and failed conditional-claim cases, including the `Claimable` race fallback.
- Email-verification and password-reset claim errors remain unchanged, including
  their `Claimable` fallback behavior.
- Existing dual-backend storage tests pass without changing SQL schema, claim
  predicates, or backend-specific behavior.

## Boundaries

- No database migration, public API change, or domain-language change.
- No unification of caller-specific error types or fallback policy.
- No changes to token hashing, issuance, pruning, expiry duration, or
  transaction strategy.
- No new abstraction beyond deleting the duplicate representation and classifier
  and migrating their callers.
