# Issue #913: Shared SQLx impl generics

## Outcome

The SQLx bridge derives `Type`, `Encode`, and `Decode` impl generics through one
private helper while emitting exactly the same Rust tokens and behavior.

## Load-bearing decisions

- One private `sqlx_impl_generics` helper owns the repeated merge policy.
- The helper starts from the user's original generics, optionally prepends an
  impl lifetime, appends `DB: ::sqlx::Database`, and adds the caller-provided
  where predicate.
- Optional leading lifetimes preserve Rust's required lifetime-before-type
  ordering.
- The emitted self type continues to use type generics from the user's original
  generics, never the merged impl generics; `DB`, `'q`, and `'r` cannot leak
  onto it.
- Existing user where clauses survive alongside the SQLx predicate.
- The `Type`, `Encode`, and `Decode` bodies and cfg/derive attributes are
  unchanged.
- `BridgeSpec.generics`, its call sites, and the compile-fail documentation
  triads remain unchanged; their repetition is outside this issue.
- This private refactor introduces no architectural decision or domain
  vocabulary.

## Acceptance

- All three impls obtain their merged generics through the same helper; the
  three near-identical merge blocks are gone.
- Emitted tokens remain unchanged for empty and generic user types, including
  lifetime ordering, user where clauses, SQLx predicates, and self-type
  arguments.
- `the_users_generics_thread_through_all_three_impls` and the existing SQLx
  bridge tests pass without weakening their assertions.
- The macro crate's focused test lane and commit gate pass.

## Boundaries

- No generated trait behavior, public macro syntax, or SQLx feature gating
  changes.
- No unrelated `BridgeSpec` or rustdoc-proof cleanup is included.
