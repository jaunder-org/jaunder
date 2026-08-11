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

`rendered-html-from-trusted` (#398) guards `RenderedHtml::from_trusted`, the
door that lets HTML reach the DOM unescaped. It matched the bare leaf ident
`from_trusted`, so its population held every `from_trusted` in policed code —
including `ContentType::from_trusted`, which mints a media type and can never
reach the unescaped sink. Before #778 a qualifier-keyed `EXEMPT_QUALIFIERS` list
absorbed that. #778 deleted the list, correctly: it granted exemptions by
pattern, and it failed **open** on an aliased qualifier
(`use RenderedHtml as ContentType`).

But #778 read the qualifier check as an _exemption mechanism_ and removed it,
leaving the over-approximation in place. The cost landed on the codebase: four
marker comments whose entire content was "this is a different door", one more
for every future mint site, and — structurally — a rule that no other type in
the repo may give a function a sensible name that this gate happens to police.

The justification for matching the leaf rather than the path was that path
matching fails open under aliasing. That does not survive scrutiny. A `use`
alias is the most syntactically visible thing in a file; the gate already parses
those files with `syn`; and the specific hole #778 cited is itself a `use`
alias. The gate over-approximated by choice, not by necessity.

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
the population and report it. That requirement is what makes narrowing safe, and
it is not optional: it is the whole reason reading the qualifier is legitimate
rather than a hole. Concretely, in `rendered-html-from-trusted`: a qualifier
resolving to the owner is the door; one resolving to another named type is not;
one that cannot be resolved at all — a glob import, a generic parameter, an
unqualified call, a macro body — stays in the population. Obscuring a qualifier
buys a gate failure, not an exemption.

The goal is not to interpret all of Rust; that is unworkable and unnecessary.
The goal is that a gate stop restricting people from naming a function
consistently with similar functions elsewhere. The ordinary case — an explicit
qualifier naming a type the gate can see — is exactly the case a syntactic pass
resolves, and the hard cases are pushed back onto whoever wrote them.

## Consequences

- A gate may police a name another type also legitimately uses, without taxing
  that type. `ContentType::from_trusted` and `RenderedHtml::from_trusted`
  coexist, both keeping the name their conventions call for
  ([ADR-0063](0063-domain-value-newtype-convention.md)'s door taxonomy), and no
  future type inherits the friction.
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
