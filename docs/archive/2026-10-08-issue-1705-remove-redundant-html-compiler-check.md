# Remove the redundant RenderedHtml compiler check (#1705)

## Outcome

Remove the bespoke standalone `rendered-html-compiler-boundary` gate rather than
maintaining its stale offline Cargo-home dependency. Preserve the existing
RenderedHtml design and real sanitization proofs; add no replacement gate.

## Load-bearing decisions

- Keep the opaque `RenderedHtml` type, host-side sanitizer, typed assembly,
  test-support gating, and trusted database/DTO reconstruction unchanged.
- Keep existing positive and compile-fail doctests. They already detect exposing
  the tuple constructor and cover raw conversion and blanket deserialization.
- Keep actual publishing-path sanitization tests and existing HTML sink checks.
- Delete the standalone checker implementation, its module declaration, and its
  gate registration, including its checker-specific unit tests.
- Do not replace it with dependency-feature graph enforcement, source-spelling
  enforcement, a reduced compiler fixture, or automatic Cargo-home refresh.
- Record the narrow supersession of ADR-0079's standalone-check mechanism in a
  numberless ADR draft and project it into current architecture documentation.
  The sanitization decision and trusted reconstruction policy remain in force.
- Accept that fixture confinement depends on existing feature declarations and
  review; the removed check supplied one distinct bare-feature helper proof.
- Treat the type as an accidental-misuse guard, not a guarantee that every value
  has passed the sanitizer. Trusted DTO/SQLx reconstruction remains intentional.

## Acceptance

- Active gate catalogs and compiled xtask modules do not contain the standalone
  checker. No replacement checker or Cargo-source selection machinery is added.
- Existing RenderedHtml doctests pass, including readable use and rejection of
  tuple construction, raw String conversion, and blanket deserialization.
- The existing publishing-path sanitization regression passes on SQLite and
  PostgreSQL, proving malicious author input is scrubbed before persistence.
- Applicable xtask gate/catalog tests pass after removing the step.
- Current architecture describes the retained proofs and accepted limits without
  claiming the standalone check or universal construction confinement remains.
- The PR records the executed necessity experiments and explains this narrowed
  resolution; it does not claim stale Cargo-source selection is fixed generally
  or attribute retained-environment propagation to an unproven owning layer.

## Boundaries

No product behavior, fixture API, schema, rendering output, sanitizer policy,
trusted reconstruction semantics, offline policy, pinned tools, or test budgets
change. Other offline Cargo-home consumers and general source-selection repair
are outside this approved solution. No separate implementation outline is needed
for this bounded deletion and documentation update.
