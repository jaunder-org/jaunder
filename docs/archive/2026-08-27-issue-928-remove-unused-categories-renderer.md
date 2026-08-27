# Remove the unused AtomPub Categories Document renderer

## Outcome

Jaunder no longer carries an unreachable serializer for an out-of-line AtomPub
Categories Document. Its supported category-discovery behavior remains the
inline `app:categories` declaration in each applicable Collection within the
Service Document.

## Load-bearing decisions

- Jaunder does not serve an out-of-line Categories Document or advertise one
  through an `app:categories href` reference.
- Collection categories remain inline in the AtomPub Service Document with the
  existing open-set `fixed="no"` semantics.
- Remove the standalone renderer and its public module surface rather than
  retaining speculative protocol code without a route or caller.
- Keep the architecture view explicit about the supported inline form and the
  intentionally unsupported out-of-line form.
- Preserve archived implementation plans as historical records.
- Record a narrow proposed ADR superseding only ADR-0089 Decision 6's retention
  of the Categories Document writer; preserve the rest of ADR-0089.

## Acceptance

- No production symbol, module declaration, re-export, or renderer-local test
  remains for the standalone Categories Document.
- Current AtomPub module documentation no longer claims that Jaunder serializes
  a standalone Categories Document.
- The Service Document test proves that inline Collection categories retain
  their terms and `fixed="no"` open-set declaration.
- The architecture view states that Jaunder uses inline category discovery and
  does not serve an out-of-line Categories Document.
- The tracked ADR draft, ADR-0089 reciprocal navigation, and architecture
  citation record the narrow supersession.
- `cargo xtask test-local -- -p common` and `cargo xtask precommit` pass.

## Boundaries

- Do not add, remove, or alter AtomPub routes.
- Do not change category validation, tag storage, or Collection membership.
- Do not change Service Document category serialization; strengthen its existing
  test only to preserve the retained inline contract.
- Do not modify archived plans or introduce a new protocol capability.
