# Evaluate SeaQuery for dynamic SQL construction (#1491)

## Outcome

Determine, from Jaunder-specific evidence, whether SeaQuery would genuinely
improve any of the storage queries whose SQL structure is assembled at runtime.
Publish a durable, per-pattern recommendation rather than adopting or rejecting
the library repository-wide.

## Load-bearing decisions

- The evaluation covers dynamic SQL construction throughout `storage`, including
  handwritten `format!`/concatenated fragments, SQLx `QueryBuilder` use,
  variable-cardinality statements, and backend-specific catalog or backup SQL.
- Static SQL is out of scope. The evaluation will not introduce a broad storage
  abstraction or attempt to express every query through SeaQuery.
- Existing architecture remains authoritative: SQLite and PostgreSQL stay
  concrete at their backend leaves, shared stores retain their focused dialect
  seams, and backup implementations remain backend-specific.
- The deterministic candidate set is the newest stable `sea-query` and
  `sea-query-sqlx` releases published by the evaluation date. Their declared
  SQLx ranges are checked against Jaunder's pinned SQLx version; if
  incompatible, the evaluation additionally identifies and checks the newest
  stable adapter release that declares compatibility, if one exists.
  Pre-releases and Git revisions may explain upstream direction but cannot
  support adoption.
- Every candidate must also be checked against Jaunder's Rust toolchain, enabled
  database/runtime features, and dependency policy rather than assessed from
  generic latest-version documentation.
- Throwaway prototypes will exercise both a favorable variable-cardinality case
  and a high-friction composed query such as visibility or Syndication Feed
  window selection. Other dynamic patterns may receive prototypes when the
  inventory identifies a credible, materially different benefit.
- Prototypes must preserve Jaunder's typed bind admission, backend behavior,
  query and transaction shape, error propagation, and observability. Equivalent
  returned rows alone are insufficient evidence.
- “Genuinely better” means less manual placeholder, bind-order, duplication, or
  fragment-composition burden while keeping backend differences and SQL intent
  at least as auditable. Recreating the existing construction inside raw custom
  expressions is not an improvement.
- The conclusion may differ by pattern. `adopt` means a stable compatible
  candidate satisfies every preservation requirement and materially improves the
  pattern; it requires focused follow-up scope. `consider` means the candidate
  is viable but a named, bounded uncertainty prevents adoption; it must state
  the evidence needed and the scope of any resolving follow-up. `do not adopt`
  means compatibility or an invariant fails, or the prototype shows no material
  improvement over the current construction. Production adoption remains
  deferred to explicit follow-up work.
- The branch will not retain SeaQuery dependencies, prototype machinery, or
  production query changes. It will retain only the evidence and disposition.
- The durable record is a research note under `docs/superpowers/research/` plus
  the pull request and issue summary. No ADR is needed unless a later adoption
  changes an architectural decision.

## Acceptance

- The research note inventories meaningful dynamic SQL-construction patterns,
  cites representative owning paths, and explains why each pattern is dynamic.
- It records exact release metadata and establishes or disproves compatibility
  with Jaunder's SQLx/toolchain/features without leaving dependency changes in
  the delivered tree.
- Representative prototypes cover a variable-cardinality query and a complex
  composed query, recording generated SQL and bind behavior. Shared/generic
  patterns and patterns implemented by both backends require SQLite and
  PostgreSQL evidence; single-backend evidence is allowed only when the owning
  seam is intentionally backend-specific under ADR-0019.
- The evaluation checks relevant SQLite transaction/query-shape invariants and
  the existing typed decode/bind boundaries rather than treating compilation as
  sufficient proof.
- Before prototype removal, the research note records the comparison method and
  results for error propagation and observability, including representative
  error paths and the affected span names and bounded fields. Retained snippets
  or captured output must make those claims auditable from the delivered tree.
- A per-pattern decision matrix applies the defined `adopt`, `consider`, or
  `do not adopt` threshold with evidence for readability/reuse, safety, backend
  clarity, escape-hatch use, and dependency/maintenance cost.
- Every `adopt` recommendation identifies focused follow-up scope; every
  `consider` result identifies its bounded uncertainty, required evidence, and
  possible resolving scope; every rejection explains why the current
  construction is preferable. No production migration is performed under #1491.
- The final tree passes the applicable documentation and repository checks and
  contains no SeaQuery dependency or prototype artifact.

## Boundaries

- No static-query rewrite, ORM adoption, schema migration, storage-trait change,
  backend unification, or production behavior change.
- No claim that a generated AST makes SQLite and PostgreSQL semantically
  interchangeable; backend parity remains an explicit proof obligation.
- No benchmark program unless the evaluation surfaces a concrete performance or
  allocation concern capable of changing a per-pattern recommendation.
- No follow-up implementation is folded into this issue; recommendations become
  separately scoped issues only after review.
