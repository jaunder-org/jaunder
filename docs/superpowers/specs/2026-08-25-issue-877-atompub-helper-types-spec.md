# Issue #877: Typed AtomPub test helpers

## Outcome

The AtomPub integration-test request helpers accept distinct method,
request-target, and username domain types. Transposing a method and URI at a
callsite fails at compile time while existing AtomPub request behavior remains
unchanged.

## Load-bearing decisions

- `atompub_authed`, `atompub_xml`, and `atompub_at` accept `http::Method` by
  value and `RootRelativeUrl` by reference.
- Helpers that accept an explicit username use `Username` by reference;
  `RawToken` remains the credential type.
- Test literals are parsed once at the test or setup boundary and retained as
  typed values through request construction.
- Absolute AtomPub response `Location` values are reduced once to their
  validated root-relative path-and-query before follow-up request construction.
- The absolute-to-root-relative conversion strips scheme and authority while
  preserving the exact path and query.
- AtomPub suffix parameters remain strings because they are path fragments, not
  complete request targets.
- Existing `Method`, `RootRelativeUrl`, and `Username` types are adopted; no new
  domain type is introduced.
- No compile-fail test harness is introduced for integration-test helpers.

## Acceptance

- `atompub_authed` accepts `(Method, &RootRelativeUrl, &Username, &RawToken)`
  before its optional body concerns.
- `atompub_xml` accepts
  `(Method, &RootRelativeUrl, &Username, &RawToken, Option<&str>)`.
- `atompub_at` accepts `(&SeededSession, Method, &RootRelativeUrl)`.
- Swapping method and URI arguments at an AtomPub helper call produces Rust
  mismatched-type errors.
- All AtomPub integration callsites parse literal request targets and usernames
  once at their boundary, then retain and borrow typed values.
- Follow-up requests derived from absolute `Location` headers preserve the
  emitted path and query without re-parsing the absolute URL as root-relative.
- Existing Basic-auth identity mismatch, foreign-user path, malformed
  username-segment, media method-matrix, and response-header assertions retain
  their behavior.
- The complete AtomPub integration lane passes against both configured storage
  backends.

## Boundaries

- This change is limited to AtomPub integration-test helpers and their AtomPub
  integration callsites.
- `basic_header(username: &str, token: &RawToken)` remains unchanged; typed
  callers expose the username with `as_ref()`.
- Foreign or malformed username path segments that are valid root-relative URLs
  remain server input and are not pre-validated as `Username`.
- No compatibility overload, string-taking alias, method newtype, trybuild
  suite, or doctest compile-fail harness is added.
