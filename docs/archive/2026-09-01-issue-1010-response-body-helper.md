# Issue #1010: Reuse response body decoding helper

## Problem

The projector listing, permalink, and tag integration tests and the web router
integration tests manually collect Axum response bodies and decode them for text
assertions. The current tree contains sixteen such translations, four more than
the issue's original audit count. The shared `helpers::body_string` function
already owns this operation.

Raw bytes remain observable in separate tests: empty sanitized error bodies,
exact CSR shell bytes, and permalink repeat-response byte identity. Those
assertions must not be converted to strings.

## Decision

Import and use `crate::helpers::body_string` for every current manual
response-body translation used only for text inspection in:

- `server/tests/projector/listing.rs` (four callers)
- `server/tests/projector/permalink.rs` (six callers)
- `server/tests/projector/tags.rs` (five callers)
- `server/tests/web/router.rs` (one caller)

The migration follows the current tree rather than preserving the stale count of
twelve. `body_string` consumes the response, collects it with the existing
unbounded test limit, and requires valid UTF-8. No helper or production
interface changes.

## Preserved boundaries

Keep these byte-oriented paths explicit and unchanged:

- sanitized 500 response bodies asserted empty;
- exact `TEST_SHELL` byte comparisons;
- the permalink repeat-response byte-identity assertion, comparing the first
  response's decoded string bytes with the separately collected repeat response
  bytes.

Keep all status assertions and text assertions otherwise unchanged. The affected
HTML and router responses are text contracts; invalid UTF-8 is a failed test
rather than data to decode lossily.

## Verification

Run focused integration tests covering the projector listing, permalink, tag,
and web router modules for both configured storage backends. Then run
`cargo xtask check`. Review the final diff against repository standards and this
specification.

## Non-goals

- Changing `body_string` or adding another helper.
- Migrating raw-byte assertions.
- Changing production response generation, routing, status behavior, or
  public/test interfaces.
- Refactoring unrelated response-body handling outside the issue's four named
  files.
