# #697 — finish the newtype conversion, not a drift gate

Issue: [#697](https://github.com/jaunder-org/jaunder/issues/697). Milestone:
Domain-value type safety (newtypes).

## Summary

Issue #697 framed the problem as primitives "drifting back" after ADR-0063
newtypes existed. The evidence does not support that stronger claim. The issue's
examples are better understood as **unfinished comprehensive conversion**: an
audit found remaining primitive seams after the newtype convention and many
newtypes had landed.

This cycle should therefore optimize for finishing the conversion, not for
building a broad adoption gate. The work is an audit-and-cutover pass: enumerate
remaining candidate primitive seams, convert the ones where an existing domain
type unambiguously owns the value, record deliberate primitives at the owning
source comments, and file follow-ups only for genuinely separate design
decisions.

## Decisions

### D1 — Treat #697 as conversion closure

Do not implement `newtype_adoption_check` in this cycle. A general gate tries to
mechanize semantic ownership ("does this `String` mean the complete domain
value?") and quickly turns into a central registry, source-marker ecology, or
heuristic false-positive engine. That is not the shortest path to the milestone
goal.

The target is narrower and more useful: close the known remaining adoption gap
from the milestone audit. Code changes should remove primitive seams where the
domain type already exists and the local semantics are clear.

### D2 — Reconstruct the remaining candidate inventory

Start from issue #697's evidence and re-check the current tree rather than
assuming the issue body is still accurate. Build a candidate list for:

1. `#[macros::server]` parameters and named request/response structs under
   `web/src` that still carry primitives for complete domain values.
2. Production struct fields whose nearby module already declares or imports the
   domain type that owns the value.
3. Same-typed adjacent primitive parameters in production signatures where a
   transposition would compile and existing domain types would remove the risk.
4. SQL boundary primitives only where existing `sqlx-newtype-bind` /
   `sqlx-newtype-decode` gates do not already own the population.

This inventory is investigative, not a new gate. Use structural search where it
helps, but every candidate still needs local semantic review before conversion.

### D3 — Convert only unambiguous existing-type seams

For each candidate, convert to the existing domain type when all are true:

- the value is a complete domain value, not a fragment, flag, raw accumulator,
  open-ended key/value slot, unit constant, or external-library shape;
- the domain type already exists and owns the validation/normalization/trust or
  transposition invariant;
- the conversion can keep wire/storage/rendered behavior unchanged.

Examples that are already fixed in the current tree and should stay fixed:

- `LoginRequest.label: Option<SessionLabel>`;
- `web::tags::api::list(..., limit: Option<PageSize>)`;
- Syndication Feed metadata fields using `FeedTitle` / `FeedDescription`.

Do not introduce new domain types in this cycle. If a candidate needs a new
type, a schema/API decision, or product judgment, file a follow-up issue rather
than stretching #697.

### D4 — Keep deliberate primitives local

When a primitive is deliberate and easy to misread as a missed conversion, make
the owning source comment say why. Do not create a central allowlist/sin bin.

Known deliberate primitive classes include:

- `web/src/tags/api.rs` `prefix: Option<String>` — partial SQL `LIKE` fragment,
  not a complete tag value.
- `storage/src/helpers.rs` `PostRow.tags: String` — JSON aggregate parsed in
  `build_post_record`.
- AtomPub pre-validation accumulators for raw XML text.
- `common/src/tag.rs` `MAX_TAGS_PER_POST` — cardinality bound owned by the tag
  collection constructor.
- `web/src/media/format.rs` unit constants.
- `web/src/media/api.rs` `force: Option<bool>` — flag.
- `client/src/storage.rs` localStorage key/value helpers — browser storage is
  open-ended infrastructure.

If an existing comment already states the reason, do not duplicate it.

### D5 — Regression protection is ordinary typed seam coverage

For seams converted or verified in this cycle, prefer ordinary compile-checked
coverage that ties directly to the real seam, not metadata markers. Examples:

- destructure `LoginRequest { label, .. }` and pass `label` to a helper
  requiring `Option<SessionLabel>`;
- assert the actual tag server function/request seam still exposes
  `Option<PageSize>`, using the real function/request type available in this
  codebase;
- construct/destructure the actual feed metadata structs so fields must be
  `FeedTitle` / `FeedDescription`.

Do not add tests that only prove local variables can parse; they must type-tie
to the seam being protected.

### D6 — No product behavior change

This issue is a type-threading cleanup. It must not change domain invariants,
serde/sqlx bridges, storage schemas, API payload shapes, rendered output, route
behavior, or user-visible validation.

## Acceptance criteria

- **AC1 — inventory is current.** The implementation records the current
  candidate list used for the conversion pass, with each candidate classified as
  converted, already typed, deliberate primitive, or follow-up issue.
- **AC2 — unambiguous seams are typed.** Any remaining primitive seam that
  unambiguously carries an existing complete domain value is converted to that
  domain type, with callers migrated cleanly and obsolete primitive helpers or
  comments removed.
- **AC3 — deliberate primitives are locally justified.** Primitive sites that
  are easy to confuse with missed conversions have source-local comments
  explaining the reason; no central allowlist or adoption registry is added.
- **AC4 — regression coverage ties to real seams.** Added or updated
  tests/assertions compile-check the actual converted seams, not unrelated local
  variables.
- **AC5 — follow-ups are explicit.** Any candidate requiring a new domain type,
  schema/API decision, or broader gate is filed as a separate issue and linked
  from the implementation notes.
- **AC6 — behavior is unchanged.** Existing focused tests for touched web,
  storage, feed, and serialization paths pass.

## Out of scope

- Building `newtype_adoption_check` or any broad adoption gate.
- Creating a central `name -> type` registry, rejects registry, or marker
  contract system.
- Introducing new domain types or changing ADR-0063 trailer behavior.
- Replacing `sqlx-newtype-bind` or `sqlx-newtype-decode`.
- Changing storage schemas, wire formats, rendered markup, route behavior, or
  validation semantics.

## Risks

- **Audit incompleteness.** A manual conversion pass can miss candidates.
  Mitigate with structural searches and an explicit classified inventory, but do
  not call it a permanent gate.
- **Over-conversion.** Some primitives are correct because they are fragments,
  flags, accumulators, units, or open-ended infrastructure. Keep those primitive
  and document locally only when ambiguity remains.
- **Weak regression tests.** Type assertions that do not touch the real seam are
  false confidence. Tie assertions to actual structs/functions.
