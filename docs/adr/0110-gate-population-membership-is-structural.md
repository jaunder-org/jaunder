# ADR-0110: A gate's population membership is structural, and must fail closed

- Status: accepted
- Date: 2026-08-10
- Issue: [#790](https://github.com/jaunder-org/jaunder/issues/790)

## Context

[ADR-0085](0085-static-type-safety-gates-enumerate.md) says a static gate
enumerates its population structurally, and its principle 3 says nothing
self-exempts: "An exemption is an entry a human wrote, or it does not exist."
[ADR-0094](0094-gate-exemptions-in-source-markers.md) makes those written
exemptions in-source markers.

Neither says how a gate decides **who is in the population in the first place**,
and #778 showed that the omission matters.

The retired trusted-HTML spelling policy exposed the omission: a gate that
matched a bare method name also captured unrelated domain types that used the
same conventional name. A qualifier-pattern exemption tried to compensate, but
that exemption failed open under an alias. The incident showed that membership
must be derived from syntax, rather than widened and then forgiven by policy.

## Decision

**Identifying a gate's population is structural. Exempting a site from it
requires a human marker. These are different operations and only the second is
what ADR-0085 principle 3 governs.**

So a gate may read whatever the AST plainly says in order to decide membership —
including a path's qualifier, a file's `use` bindings and type definitions, and
an enclosing `impl`'s self-type. Doing so is not a self-exemption: it is the
gate correctly identifying the door it guards, before any question of exempting
anything arises.

**A gate that cannot determine membership must fail closed** — keep the site in
the population and report it. That requirement is what makes narrowing safe:
obscuring a qualifier must buy a gate failure, not an exemption.

The goal is not to interpret all of Rust; that is unworkable and unnecessary.
The goal is that a gate stop restricting people from naming a function
consistently with similar functions elsewhere. The ordinary case — an explicit
qualifier naming a type the gate can see — is exactly the case a syntactic pass
resolves, and the hard cases are pushed back onto whoever wrote them.

## Consequences

- A gate may police a name another type also legitimately uses without taxing
  that type, provided its structural membership logic can distinguish the owner.
- ADR-0085 principle 3 is unchanged, and now applies only to exemptions — which
  is what it was always about. #778's deletion of `EXEMPT_QUALIFIERS` remains
  correct on its own terms; what was wrong was concluding that the qualifier
  must therefore go unread.
- Membership resolution is only as wide as the gate's roots. A renaming
  re-export living outside `POLICED_ROOTS` is never harvested, so a use site
  inside them could resolve to another type and be suppressed. That is the price
  of resolving names without a compiler, and it is why a gate's roots must cover
  every tree it claims to police.
- Narrowing a population is now a **behaviour change to review**, not a tidy-up.
  Each such change must be able to demonstrate the door it guards is still
  policed — for #790, that removing the owner's own marker still fails the gate.
- A gate opts in per population. Where the population is a type (`PreEscaped`)
  or a method reached through `.` on a runtime receiver (`set_inner_html`),
  there is no qualifier to read and the ident is the whole question; those gates
  are unaffected.
- The residual unreadable classes are enumerated in the gate's module doc, as
  ADR-0085's honesty obligation requires. Resolution replaces one blind spot (an
  aliased qualifier handing out an exemption) with three narrower ones, each
  fail-open and each recorded: a rename of a rename, a renaming re-export
  outside the roots, and a free `fn` nested inside another type's `impl`.
