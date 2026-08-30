# Issue #1267 `validator` assessment

## Outcome

Jaunder has a tracked, evidence-backed assessment of whether the Rust
`validator` crate should replace any of its handwritten parsing or validation.
The report gives auditable gross and net handwritten-LOC estimates and
recommends repository-wide adoption, selective use for named validation
families, or rejection.

## Load-bearing decisions

- The deliverable is `docs/issue-1267-validator-assessment.md`, not a
  production-code adoption pilot. It does not add `validator`, change runtime
  behavior, or migrate a validation path.
- The report records the assessed commit. Its population is every tracked Rust
  item in workspace packages and build scripts, including tooling and
  target/feature-gated code. Test-only items, test/bench/example/fixture
  targets, generated output, vendored code, and parsing with no data-validity
  decision are excluded.
- A documented, repeatable census combines mechanical searches with a manual
  review of every included module. Every census hit reconciles to one candidate,
  a duplicate candidate, or a named exclusion.
- Each candidate has one subtotal-bearing primary family, assigned in this
  precedence: stateful/storage check, protocol/grammar parser, configuration
  registry, cross-field check, domain newtype, then field predicate.
  Non-additive tags record secondary behaviors.
- Each candidate is evaluated against a validator-native greenfield design.
  Existing constructor, `FromStr`, serde, sqlx, error, client-prevalidation, and
  wasm contracts may be redesigned rather than treated as mandatory interfaces.
  Every projection cites the exact `validator` API and includes a complete,
  countable replacement sketch. Its behavior map classifies each current
  responsibility as validator-owned, retained custom code, or deliberately
  redesigned.
- Consequently, projected savings are a greenfield ceiling, not an estimate of
  the effort or benefit of adopting `validator` in the current architecture. ADR
  reversal and compatibility-migration costs are outside the calculation.
- The report still names accepted ADRs that the greenfield design contradicts,
  so the comparison cannot be mistaken for an architecture-compatible cutover.
- Security-sensitive validation remains in the inventory. A replacement must
  still account for timing equalization, cheap validation before expensive work,
  secret-safe errors, and atomic validation/write behavior; these semantics are
  not discarded by the greenfield comparison.
- LOC means physical Rust source lines, excluding blank, comment-only, and
  test-only lines. Current counts cite exact source ranges. A mixed-purpose line
  counts only when the greenfield design removes it wholesale; retained behavior
  is not counted as savings.
- Gross savings are current removed LOC. Net savings subtract projected
  attributes, imports, custom validators, adapters, errors, and other required
  replacement LOC. Shared current or replacement support is a separate row
  allocated exactly once. Dependency, test, ADR-reversal, and migration costs
  are reported separately rather than converted into LOC.
- A family qualifies for adoption only when its aggregate net ceiling is
  positive and dependency/target fit, security semantics, maintenance cost, and
  accepted-ADR incompatibility contain no stated disqualifier. Broad adoption
  means every technically supported family qualifies; selective use means at
  least one qualifies and at least one does not; rejection means none qualify.

## Acceptance

- `docs/issue-1267-validator-assessment.md` identifies the assessed commit,
  evaluated `validator` version and capabilities, primary source documentation,
  target constraints, complete census recipes, manual-review procedure, and LOC
  methodology.
- The census reconciles every hit as a candidate, duplicate, or named exclusion
  and explicitly disposes of every included Rust module.
- An auditable table identifies every candidate by repository-relative path,
  symbol, and current source ranges; assigns its exclusive primary family and
  any secondary behavior tags; and records full, partial, or no replacement.
- “Full” means the behavior map leaves no validation responsibility in custom
  code; “partial” means `validator` owns at least one responsibility while
  another remains custom; “none” means it owns none.
- Every full or partial row references an exact-API, countable replacement
  sketch and records current LOC, replacement LOC, gross savings, net savings,
  assumptions, and the complete behavior map. Shared support appears once.
- Family subtotals and repository totals reconcile exactly with the exclusive
  candidate rows and shared-support rows. Exclusions and ambiguous cases are
  explicit rather than silently omitted or rounded into totals.
- The assessment distinguishes declarative predicates from parsing,
  normalization, grammar recognition, cross-field policy, I/O, storage state,
  and transaction semantics.
- Security-sensitive rows account for the constraints above and are not counted
  as replacements when the proposed design omits one.
- The report identifies conflicts with applicable accepted ADRs and clearly
  labels the numerical result as a greenfield ceiling that excludes reversal and
  migration costs.
- The conclusion applies the stated decision rule and recommends broad adoption,
  selective use naming both qualifying and non-qualifying families, or
  rejection. It explains every qualitative disqualifier separately from the LOC
  ceiling.

## Boundaries

- No production dependency, public API, schema, protocol, storage behavior,
  security behavior, or validation error contract changes in this issue.
- No permanent inventory generator, lint, xtask command, or CI gate is added;
  the table and documented method are the reproducibility surface.
- The assessment does not authorize a later migration. Any adoption requires a
  separate issue and, where accepted decisions would change, the repository's
  ADR process.
- Non-Rust validation is outside scope.
