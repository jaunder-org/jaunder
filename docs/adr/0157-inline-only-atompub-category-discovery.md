# ADR-0157: Keep AtomPub category discovery inline

- Status: accepted
- Date: 2026-08-27
- Issue: [#928](https://github.com/jaunder-org/jaunder/issues/928)

## Context

Jaunder's AtomPub Service Document already advertises each applicable
Collection's categories inline as an open set. A separate serializer for an
out-of-line Categories Document was also built, but no Service Document
`app:categories href`, server route, or production caller ever made that
document reachable.

RFC 5023 makes both the inline and out-of-line forms optional alternatives.
Serving the second form without a Protocol Client requirement would add another
resource and protocol contract while duplicating discovery information already
available inline.

[ADR-0089](0089-upstream-atom-document-io.md) Decision 6 retained the Categories
Document writer alongside the Service Document and RSD writers. That inventory
became a decision-log commitment even though the renderer remained unreachable.

## Decision

Jaunder supports AtomPub category discovery only through inline `app:categories`
elements in the Service Document. Applicable Collections keep their existing
`fixed="no"` open-set declaration and inline category terms.

Jaunder does not advertise an `app:categories href` reference and does not serve
an out-of-line Categories Document. The unused standalone renderer and its
public module surface are removed.

This decision narrowly supersedes [ADR-0089](0089-upstream-atom-document-io.md)
Decision 6 only where it says the Categories Document keeps its `quick-xml`
writer. ADR-0089's upstream Atom I/O decision and its retention of the Service
Document and RSD writers remain accepted.

## Consequences

- Category discovery remains available to Protocol Clients through the Service
  Document without adding a route or independently cacheable resource.
- Removing the unreachable renderer reduces production and test surface without
  changing AtomPub conformance or existing wire behavior.
- `common` retains its direct `quick-xml` dependency for the Service Document,
  RSD, and their shared XML helpers.
- Adding an out-of-line Categories Document later requires a demonstrated
  Protocol Client need and an explicit protocol-surface decision.
- `CONTEXT.md` is unchanged because this decision does not alter Jaunder's
  ubiquitous language.
