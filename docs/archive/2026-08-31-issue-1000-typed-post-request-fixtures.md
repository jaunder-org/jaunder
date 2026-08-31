# #1000 — use typed post request fixtures

Issue: [#1000](https://github.com/jaunder-org/jaunder/issues/1000). Milestone:
Code quality: test fixture and lifecycle consolidation.

## Outcome

Valid post create/update requests in the four audited server-test files use one
typed `PostInputs` fixture seam instead of manually assembling JSON envelopes.
Raw JSON remains deliberate and visible wherever malformed input or exact wire
shape is the contract.

## Load-bearing decisions

- `PostInputs::new(body, format)` requires the two valid non-optional fields and
  initializes every optional field to `None`.
- Callers set optional fields directly. In particular, `publish: None` remains
  distinct from `Some(false)` for Org-header lifecycle behavior.
- Shared typed request fixtures live in `server/tests/helpers/posts.rs`, with
  explicit assembly/re-exports through `helpers/mod.rs` under ADR-0128.
- The create/update fixtures accept `PostInputs`, construct the generated
  `web::posts::Create` or `web::posts::Update` request aggregate, select its
  JSON server-function path, and preserve the existing status/body response
  contract.
- The generic raw `post_json` helper remains unchanged for malformed payloads
  and intentional wire-shape assertions.
- Every currently valid create/update JSON envelope in the four named files
  migrates, including valid cases added after the audit recorded fifteen sites.
  Leaving a second valid-request convention is not permitted.
- Raw JSON remains for malformed body, format, tag/count, cursor, missing-field,
  and other decode-rejection cases, plus assertions about nested versus flat
  wire shape.
- This applies ADR-0129's existing typed request-aggregate decision and
  introduces no new architectural decision.

## Acceptance

- Valid create/update requests in `server/tests/feed/feed_events_hook.rs`,
  `server/tests/web/posts/create.rs`, `listing.rs`, and `update.rs` use the
  shared typed fixtures.
- No valid manual create/update JSON envelope remains in those files; each
  remaining raw payload is justified by a malformed-input or wire-shape
  contract.
- The constructor defaults all optional fields to `None`, and tests prove the
  default plus meaningful `None`/`Some(false)` lifecycle behavior.
- Typed fixtures serialize the generated create/update request aggregates as
  JSON and preserve the existing `(StatusCode, String)` response seam.
- Existing endpoint paths, wire format, response behavior, backend coverage, and
  product-facing interfaces are unchanged.
- Affected focused server tests and `cargo xtask check` pass.

## Boundaries

- No AtomPub test splitting or other work owned by #976.
- No changes to malformed-input acceptance/rejection semantics, endpoint codecs,
  post lifecycle policy, or storage behavior.
- No builder family, fluent setters, compatibility alias, retry behavior, or
  generic HTTP-helper redesign.
