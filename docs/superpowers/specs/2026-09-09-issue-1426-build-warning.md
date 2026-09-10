# Issue 1426: Remove the conditional metrics import warning

## Outcome

The default-feature storage library build emits no unused-import warning for the
idempotency metrics types used only by test-support post creation.

Runtime behavior, telemetry behavior, and feature boundaries remain unchanged.

## Load-bearing decisions

- Limit this issue to aligning the import's conditional compilation with its
  only caller; do not move or add telemetry calls.
- Preserve the existing `test` and `test-utils` caller behavior exactly.
- This patch neither resolves nor expands the existing production expiry-signal
  contract in ADR-0167.
- Add no lint suppression and no compatibility path.

## Acceptance

- `cargo check -p storage --lib` completes without an unused-import diagnostic
  naming `IdempotencyEvent` or `self`.
- `cargo check -p storage --lib --features test-utils` compiles the existing
  idempotency-expiry metric call.
- Outside this specification, the implementation diff changes only the metrics
  import's conditional-compilation attribute in `storage/src/post_service.rs`.

## Boundaries

- No telemetry policy, event semantics, post-creation flow, or storage behavior
  changes; production expiry-signal conformance with ADR-0167 is outside this
  warning-only issue.
- No new tests are required for an import-visibility correction; the two named
  compile checks are the proof.
- No ADR or domain-glossary update: this change introduces no architectural or
  domain decision.
