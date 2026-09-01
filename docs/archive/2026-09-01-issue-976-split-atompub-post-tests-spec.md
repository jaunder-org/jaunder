# Split AtomPub post tests by contract

## Outcome

The AtomPub Post integration tests are organized into cohesive contract modules
without changing protocol behavior, backend coverage, or test names. The stable
`atompub::atompub_posts::` prefix remains; fully qualified test filters gain one
concern-module segment.

## Load-bearing decisions

- Keep AtomPub Collection and Member tests distinct from public Syndication Feed
  tests and preserve the native-source serializer boundary from ADR-0015.
- Split the current suite into `collection_reads`, `member_reads`,
  `authorization`, `entry_mutations`, `etag_preconditions`, `visibility`,
  `scheduling`, `idempotency`, and `media_persistence` contract modules.
- Keep shared Entry XML construction in one private `fixtures` module;
  concern-local helpers stay with their owning contract.
- Assign overlapping tests by their primary observable assertion: stale Org
  synchronization to ETag preconditions; native Member body/ETag and
  delete-then-GET to Member reads; named/default audience behavior to
  visibility; explicit `app:draft=no` published-time behavior to scheduling.
- Preserve every existing test function name and dual-backend matrix. Exact
  paths may change only by insertion of the selected concern module beneath
  `atompub::atompub_posts`.
- Preserve ADR-0023 wire media-type, `j:slug`, and capability contracts;
  ADR-0024 Org canonicalization; ADR-0089 upstream Atom document I/O; and the
  single integration-test binary required by ADR-0067.
- Any introduced `mod.rs` is wiring-only under ADR-0128. No second integration
  target or duplicated path-based module is permitted.

## Acceptance

- Every existing test from `atompub_posts.rs` exists exactly once under the
  concern named above, with its original function name and backend
  parameterization.
- Each resulting test file has one independently nameable protocol
  responsibility; shared fixtures contain no tests.
- Existing broad filters under `atompub::atompub_posts::` still select the
  suite, and concern-specific filters select only their contract.
- AtomPub HTTP requests, assertions, fixtures, expected wire documents, status
  codes, ETags, visibility, scheduling, idempotency, and media persistence
  behavior are unchanged.
- Focused AtomPub tests and the repository gate pass.

## Boundaries

- No production AtomPub, storage, routing, serialization, or Syndication
  behavior changes.
- No test semantic rewrites, new coverage, renames, ignored tests, or
  backend-matrix changes.
- No glossary, ADR, architecture projection, or generated ADR table change; this
  is an internal test-organization refactor.
- No implementation outline is required because the work changes neither a
  durable architecture boundary nor a public protocol/API.
