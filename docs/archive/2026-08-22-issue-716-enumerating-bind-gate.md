# Issue #716: enumerating `sqlx-newtype-bind`

## Outcome

`sqlx-newtype-bind` stops searching for known newtype-strip spellings and
becomes an enumerating, deny-by-default static gate for sqlx binds under
`storage/src`. The live `FeedMinItems` laundering site in `posts.rs` is removed
by carrying the domain value to the bind seam.

## Load-bearing decisions

- The gate's population is every sqlx `.bind(...)` argument under `storage/src`
  whose Rust expression is structurally a bare primitive or primitive reference.
  Legitimacy is never decided during population selection.
- The gate denies population members by default. A primitive bind passes only
  through a line-adjacent in-source marker on the line immediately above the
  exempt `.bind(...)`, following ADR-0094. The marker uses the gate token and a
  categorized reason:
  - `// sqlx-newtype-bind:allow permanent-primitive — ...` for a scalar storage
    fact with no domain identity;
  - `// sqlx-newtype-bind:allow test-fixture-corruption — ...` for intentional
    invalid test rows;
  - `// sqlx-newtype-bind:allow deferred-newtype #NNNN — ...` for debt that
    should become typed.
- Markers are line-form only. A marker with no reason, no category, no following
  primitive bind, more than one following primitive bind, or a
  `deferred-newtype` category without an issue fails closed.
- The implementation parses source instead of line-scanning bind regions.
  Unreadable roots, unreadable files, and unparseable Rust files under the
  policed root are hard failures.
- The gate still does not promise cross-function attribution. It detects an
  untyped bind; it does not prove which caller stripped a domain value before
  reaching that bind.
- `posts.rs` keeps `FeedMinItems` typed through `list_published_in_window_rows`;
  any required generic bounds are added at that seam rather than reintroducing
  `i64::from(window.min_items.value())` in the caller.
- The #716-known laundering sites are not eligible for ordinary permanent
  allowlisting. `FeedMinItems` must be fixed in this issue; the feed-event
  `limit_i` trait seam and closed-enum `kind_name` bind must either be typed in
  this issue or recorded as `deferred-newtype` with tracking issues.
- The `raw_ids` / `PgHasArrayType` decision is already recorded by ADR-0071's
  #891 amendment: Postgres array binding is opt-in per bridge family,
  `IdNewtype` opts in, and the old helper is not part of this issue's remaining
  work.
- Existing ADR-0085 remains the governing decision for this work: static
  type-safety gates enumerate their population and fail closed; no new ADR is
  expected unless implementation reveals a new architectural rule.

## Acceptance

- `storage/src/posts.rs` no longer strips `window.min_items` to `i64` before
  calling `list_published_in_window_rows`, and the helper binds a `FeedMinItems`
  value directly.
- `xtask/src/steps/sqlx_newtype_bind_check.rs` no longer documents itself as a
  line-based search or as non-conforming to ADR-0085; its module doc records the
  rebuilt gate's remaining structural blind spots and boundaries.
- The bind gate's failure detail reports unmarked primitive binds with enough
  site information to type the seam or add a categorized in-source marker. When
  any primitive-bind problem exists, diagnostics also include a derived census
  of all marked primitive-bind exemptions as `file:line — category — reason`, so
  marker use stays reviewable without a central list. The recovery text treats
  typing as the default and `deferred-newtype` as debt.
- Unit tests prove a novel/unanticipated primitive bind spelling fails because
  it is in the enumerated population, not because it matches `.as_ref()`, `&*`,
  `i64::from(...)`, or a hoisted-local spelling.
- Unit tests prove malformed, orphaned, shared-line, uncategorized, and
  `deferred-newtype`-without-issue markers fail closed.
- A focused xtask check for `sqlx-newtype-bind` passes after implementation, and
  a broader `cargo xtask check --no-test` pass has no regression in static
  checks.

## Boundaries

- No call-graph analysis. Cross-function attribution remains out of scope.
- No SQL semantic inference. Counts, timestamps, booleans, strings, and
  driver-required primitives are justified by categorized in-source markers, not
  inferred from query text; suspicious cases use `deferred-newtype`, not
  `permanent-primitive`.
- No expansion beyond sqlx bind policing under `storage/src`.
- No rework of `sqlx-newtype-decode`; it is a comparison point, not a target.
- No change to feed-event array binding unless current code contradicts
  ADR-0071's recorded #891 decision.
