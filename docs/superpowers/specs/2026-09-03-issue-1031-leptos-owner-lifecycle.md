# Issue #1031: Use Leptos owner closure lifecycle

## Outcome

Synchronous web tests establish and dispose their reactive owner through
Leptos's existing `Owner::with` closure lifecycle instead of file-local wrappers
or repeated `Owner::new`, `set`, and `drop` prologues. Test behavior and public
interfaces remain unchanged.

## Load-bearing decisions

- Replace every current synchronous manual owner lifecycle with
  `Owner::new().with(|| { ... })`; do not add a repository helper.
- Migrate all 24 current sites: the issue's six file-local wrappers, thirteen
  `Field` test prologues, and two `Invalidator` test prologues, plus the current
  equivalent sites in `posts/edit_state.rs`,
  `posts/page_state.rs::create_settlement_classifies_published_and_draft_outcomes`,
  and `forms/submit_gate.rs::one_constructor_controls_gate_and_payload`.
- Remove the seven superseded file-local `with_owner` wrappers and replace every
  synchronous caller with the direct Leptos seam.
- Keep signal construction, reads, writes, and assertions that depend on the
  reactive owner inside the closure. Values used after the closure must be
  owner-independent copies.
- Keep async tests and owner-returning helpers explicit. Their strong owner must
  outlive future polling; closing the owner before an `.await` completes would
  violate ADR-0016's reactive lifetime model.
- Preserve all existing test assertions and behavior. This is lifecycle
  consolidation only, not a production reactive-state change.

## Acceptance

- A source census finds no synchronous `Owner::new`/`set`/`drop` boilerplate or
  file-local owner wrapper among the 24 in-scope sites.
- All migrated tests use the established direct `Owner::new().with(|| { ... })`
  form.
- Async tests and helpers returning an owner retain their explicit lifecycle.
- The affected focused web library test modules pass.
- `cargo xtask check` passes.

## Boundaries

- No production behavior, public interface, reactive primitive, or repository
  helper changes.
- Do not migrate an owner whose lifetime crosses `.await` or escapes through a
  helper return value.
- Do not change ADR-0016, ADR-0060, ADR-0083, or the domain glossary; this
  change follows their existing ownership and host-test conventions.
