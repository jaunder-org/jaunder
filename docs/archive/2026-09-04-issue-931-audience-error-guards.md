# Audience Write Error Classification Tests

## Outcome

Audience creation and rename storage tests prove that database failures
unrelated to uniqueness remain storage failures rather than being misreported as
duplicate audience names.

## Load-bearing decisions

- Exercise `create_audience` and `rename_audience` through the real generic
  audience store and write transaction path.
- Run both contracts against SQLite and PostgreSQL; this is backend-common
  behavior under ADR-0053's dual-backend presumption.
- Inject an operation-specific database failure at the database boundary so the
  returned `sqlx::Error::Database` passes through the same uniqueness guard as a
  real write failure.
- The injected failure must not be classified as a unique violation by either
  backend.
- Keep production behavior, the audience schema, and public APIs unchanged.

## Acceptance

- A dual-backend creation test forces a non-unique database error from the
  audience insert and observes `AudienceError::Storage`, not
  `AudienceError::DuplicateName`.
- A dual-backend rename test forces a non-unique database error from the
  audience update and observes `AudienceError::Storage`, not
  `AudienceError::DuplicateName`.
- Existing duplicate-name behavior remains covered and unchanged.
- `devtool run -- .mutants-loop/verify.sh storage storage/src/audiences.rs`
  reports `missed=0`.

## Boundaries

- No new production fault-injection hook or extracted error-classification API.
- No schema migration, user-visible behavior change, glossary change, or ADR.
- No broader mutation-test cleanup outside the two guards named by issue #931.
