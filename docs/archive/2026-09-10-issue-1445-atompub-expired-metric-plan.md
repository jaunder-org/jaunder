# Confirmed expired Idempotency Key telemetry implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for delegated work.
> This outline exists because commit-indeterminate storage outcomes make the
> post-commit telemetry invariant non-obvious.

## Scope

In:

- Route both Post-creation service paths through one post-commit outcome
  authority.
- Prove confirmed-expiry, non-expiry, and commit-indeterminate metric behavior
  through the shared SQLite/PostgreSQL storage harness.

Out:

- Idempotency Key lifecycle or protocol changes.
- New metrics, labels, schema, or general telemetry restructuring.

## Task outline

- [x] Centralize and prove Post-creation expiry telemetry
  - Contract: one private outcome authority accepts the storage creation
    `MutationOutcome`, emits `IdempotencyEvent::Expired` only for a confirmed
    result whose `idempotency_key_expired` flag is true, and returns the same
    outcome shape containing the created `PostRecord`.
  - Callers: both `create_rendered_post` and
    `create_rendered_post_with_media_ownership` use that authority; neither
    retains parallel expiry handling.
  - Verification: dual-backend tests assert an `expired` counter delta of
    exactly one for confirmed expiry and exactly zero for both confirmed
    non-expiry and commit-indeterminate outcomes, then
    `devtool run -- cargo xtask check` passes before commit.

## Risk checks

- Metric emission remains outside the transaction and after confirmed commit.
- Commit-indeterminate durable side effects never become confirmed telemetry.
- Each confirmed expired transition emits exactly once; other paths emit zero.
- Metric attributes remain bounded and contain no Idempotency Key or Post data.
- Existing Post creation, replay, media ownership, and feed-event behavior stays
  unchanged on SQLite and PostgreSQL.
