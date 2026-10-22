# AtomPub Media Upload Size Limit Alignment

## Outcome

`POST /atompub/{username}/media` honors the existing `media.max_file_size_bytes`
input limit rather than imposing an unrelated lower application limit. Uploads
above 2 MiB but within the configured limit can succeed, subject to existing
filename sanitization and prepared-output size checks.

## Load-bearing decisions

- Reuse the existing site media-size setting and shared media upload preparation
  behavior; do not introduce a separate AtomPub-specific size setting or
  arbitrary lower ceiling.
- Preserve the authentication and username/path checks, filename sanitization,
  media upload capability behavior, bounded resource use, and truthful size
  errors.
- Continue enforcing the configured limit on prepared output as well as input.
  The input limit is an acceptance bound, not permission to persist oversized
  prepared output.
- Do not disable request-body limits globally. Request extraction must preserve
  a finite bound derived from the existing setting (including when configured
  below 2 MiB) and reject oversized requests before unbounded buffering or
  persistence; do not introduce a minimum extractor ceiling that defeats a
  smaller configured limit.
- The reported production 413 is not attributed to this application mismatch:
  proxy limits, effective production setting, upload length, and response body
  are unavailable.

## Acceptance

- Integration coverage for the real AtomPub HTTP endpoint demonstrates an upload
  larger than 2 MiB and no larger than a configured limit succeeds and yields
  the expected Media response and persisted Media.
- Integration coverage demonstrates requests above the configured limit are
  rejected with the established size-error behavior and create no persisted
  Media. With a configured limit below 2 MiB, prove both that an upload within
  the configured limit succeeds and that one above it is rejected by request
  extraction without persistence; cover exact-boundary behavior as appropriate
  to the shared limit semantics.
- Existing authentication behavior remains enforced for this route, including
  unauthorized upload rejection without persistence.
- Existing AtomPub filename sanitization and prepared-output-size enforcement
  remain covered and unchanged in behavior.
- Required backend parity is maintained per CONTRIBUTING.md. Add Playwright E2E
  coverage through the existing AtomPub Protocol Client flow in
  `end2end/tests/atompub.spec.ts`, including a >2 MiB upload within the
  configured limit; do not add a test-only interface.
- No claim about the layer responsible for the earlier production 413 is
  required or made.

## Boundaries

In scope: only AtomPub media collection POST size handling and directly
necessary tests. Out of scope: changing the global server body limit, changing
the web upload limit, production configuration or proxy changes, upload retries,
unrelated AtomPub operations, and redesigning media preparation/storage.
