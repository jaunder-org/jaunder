# ADR-0117: `Labelled` takes erased validity signals, not `Field<T>`

- Status: accepted
- Date: 2026-08-11

## Context

`Labelled` is the chrome shared by every ADR-0065 validated field. Taking
`Field<T>` would be the tidier signature, but it would make it the repo's first
generic component _with children_: in the `view!` macro a generic close tag must
match its opening generics token-for-token, a hand-matched burden on every call
site.

## Decision

`Labelled` takes the validity as two erased signals, keeping the touched-gate in
exactly one place and the component non-generic.

An earlier, second half of the rationale is spent: leptosfmt once wrote generic
tags with a trailing comma, so a fix-mode pass could unbalance a hand-matched
open/close pair. leptosfmt is pinned past the upstream fix (#420) and the
trailing commas are gone tree-wide, so the formatter hazard no longer argues for
anything. The token-for-token matching burden is real and remains; whether it
alone still justifies the erased-signal shape is an **open question** this draft
records rather than answers — #420 only changed the formatter.

## Consequences

- New validated-field chrome goes through `Labelled`'s erased-signal interface.
- Revisiting the shape means answering the open question above, not re-deriving
  it.
