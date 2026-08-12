# ADR-0094: Gate exemptions are in-source site markers

- Status: accepted
- Date: 2026-08-01
- Issue: [#778](https://github.com/jaunder-org/jaunder/issues/778)

## Context

A static gate that denies by default needs a way to say "this site is
legitimate." This repo has grown two mechanisms for that, and never argued
between them.

**Coverage** (ADR-0050) exempts by an in-source marker on the line:
`// cov:ignore`, line form or block form, with roughly fifty live markers across
the tree.

**The static gates** — `sqlx-newtype-decode`, and the three ident-keyed XSS
gates `raw-html-door` / `html-sink` / `rendered-html-from-trusted` — exempt by a
central allowlist in the gate's own source file, keyed by enclosing function
name, each entry carrying a written reason and (in two of the three XSS gates) a
multiplicity.

The second mechanism was not chosen over the first. It was inherited: each new
gate copied the previous one. So when #778 found `rendered-html-from-trusted`'s
allowlist to be a bare list of function names — an
[ADR-0085](0085-static-type-safety-gates-enumerate.md) principle-4 region
exemption — the obvious fix was to make it look like its siblings and add a
multiplicity.

**That fix is the wrong key, not a smaller version of the right one.** Principle
4's rule is _"scopes each allowlist entry to a single site, never to a region."_
Multiplicity is its **fallback**, for _"where a population genuinely contains
indistinguishable sites."_ The site in question was one perfectly
distinguishable call, so the escape hatch did not apply. A count leaves the
entry region-scoped with a cardinality assertion bolted on, and three failure
modes open: a door **swapped** rather than added inside an allowed fn keeps the
count matching; a same-named fn appearing in a second file **while the original
is deleted** keeps the tree-wide total matching; and the key is a name, so
rename or file move silently transfers or drops the exemption.

Naming that made the real question visible: the two mechanisms differ, the
difference had never been justified, and the mechanism with the _weaker_ scoping
was the one the gates had standardised on.

## Decision

**A gate expresses an exemption as a marker comment on the line immediately
above the exempt site, read by the gate — not as an entry in a central list.**
The three XSS gates adopt this; coverage already had it (trailing, which its
line-oriented report format makes safe).

The marker is `// <gate-step-name>:allow <reason>`, anchored as the comment's
first whitespace-delimited token, matching `cov:ignore`'s existing shape.
Deriving the token from the gate's own step name means there is no separate
const and no way for the marker name to drift from the gate name.

**The two mechanisms share one primitive** — "is there a marker on this source
line, and what does it say" — and nothing above it. Coverage supplies its line
from an llvm-cov report, the gates from a file by span line; the vocabularies
and their strictness differ by design. Sharing the primitive is what keeps a
single, tested answer to where a comment legally begins, including inside string
and raw-string literals, rather than two divergent scanners guarding two
invariants. The token is **per gate**, not shared, because one line can hold two
populations — `view! { <div inner_html=PreEscaped(x).into_string()></div> }` is
an unescaped sink _and_ a raw door — and a shared marker would silence both from
one reason.

Rules that make the scoping structural rather than conventional:

- **Line form only. No block form.** A block is a region exemption, which is the
  defect being removed. (Coverage's `cov:ignore-start`/`-stop` is priced
  differently — see the discriminator below.)
- **A reason is required**; a bare marker fails. Nothing beyond non-emptiness is
  checked: prose quality is not machine-checkable and a length floor teaches
  padding.
- **The line a marker points at holds exactly one site of that gate.** Two or
  more fails, directing the author to split the line, so "one marker = one site"
  is true by construction.
- **An orphan marker fails.** A marker whose next line holds no site of its gate
  is a pre-approved exemption waiting for a future edit to land on it.
- **The marker sits directly above the line the gate reports** — the matched
  ident's span line, which is already the `file:line` the failure prints. No
  blank line and no second comment may intervene.

### The position is measured, not chosen

The obvious position is a trailing comment on the site's own line, and that is
what this decision originally specified. **The formatters own that position.**
`rustfmt` pushes a comment trailing an opening `{` down onto the first line of
the block; `leptosfmt` lifts or drops one depending on where it sits in a
`view!` body. Written trailing, 7 of the 12 live sites relocated — some above,
some below, deterministic per syntactic context but not predictable by any rule
an author could hold in their head.

Written as a standalone comment line directly above the site, all twelve stayed
put across repeated formatting runs, including the two `fn from_trusted`
signatures (the marker sitting between `#[must_use]` and `pub fn`) and every
`view!`-embedded sink.

So the rule is: **put the marker where the formatter will keep it, and read it
there.** Trailing is deliberately not accepted — it is precisely the form that
silently moves, and honoring it would let someone write a marker that stops
working on the next format. A gate that must live in a formatted tree does not
get to pick its own syntax; it gets to pick among the positions the formatter is
willing to preserve.

The cost, stated plainly: the key is no longer literally "the site" but "the
line the marker points at". That line must hold exactly one site, so the binding
is still 1 marker : 1 site and still never a region — but it is one level of
indirection off the strongest claim, and calling it site-scoped without that
caveat would be overselling it.

### Pattern-shaped exemptions go too

`rendered-html-from-trusted` also skipped every `ContentType::from_trusted` site
tree-wide, decided by a **pattern on the path qualifier**. That is ADR-0085
principle 3 — _"Grants no automatic exemption from a pattern. Nothing
self-exempts"_ — and it failed **open** asymmetrically: the gate deliberately
keeps guarding a leaf name that has been aliased (`use … as`), but aliasing the
_qualifier_ (`use RenderedHtml as ContentType`) handed out the exemption.

It is deleted. A pattern exemption is only ever a bulk discount on writing
entries, and markers make an entry nearly free, so the discount buys nothing and
costs a fail-open. The affected sites took ordinary markers like anything else.

**Amended by [#790](https://github.com/jaunder-org/jaunder/issues/790).**
Deleting the exemption was right; concluding that the qualifier must therefore
go _unread_ was not. Reading it decides **membership**, which is structural,
rather than granting an exemption — see
[ADR-0110](0110-gate-population-membership-is-structural.md). So as of #790 the
gate resolved the qualifier and those four markers went. Note the consequence
for the sentence this section used to end on: with `ContentType`'s door out of
the population, the "grep `ContentType::from_trusted` to enumerate every mint
site" instruction in its doc comment became once again a convention backed by
tests, not something the gate enforces — and that doc comment was updated to say
so.

### Inferred exemptions are tripwired; written exemptions are keyed

The first thing to get right is which exemptions a machine can police at all,
because the obvious answer is wrong. Coverage's A1 guard — a _covered_ line
inside an exempt span fails the gate — does **not** protect `cov:ignore`. A
`cov:ignore` line is dropped from the executable set before the gate ever sees
it, so it can never trip the guard. ADR-0050 says so in its own consequences:
_"`cov:ignore` is permanent. A marked line that later becomes covered and then
regresses is never re-flagged … The migration bakes in ~700 permanent (but
in-source, reviewable) blind spots."_

What the A1 guard actually protects is the **structural** exemptions — a
`#[component]` body, an `unreachable!("msg")` — the ones the machine _inferred_.
That distinction is the real rule, and it holds beyond coverage:

- An **inferred** exemption rests on a premise the machine asserted, so the
  machine can and must keep testing it. A `#[component]` body that gets
  exercised natively means the inference was wrong, and it fails.
- A **written** exemption rests on a premise a human asserted — "this line is
  unreachable," "this HTML round-tripped through our own store." No machinery
  can re-verify it, in coverage or in a security gate. It is permanent by
  nature.

So the question is not _which population gets a re-checkable mechanism_ — no
written exemption is re-checkable anywhere in this repo. It is: **given that a
written exemption can never be re-verified, what must it cost?**

### What decides marker vs. central list

Three things separate the mechanisms, and only the first two survive scrutiny:

**Keying.** A marker's key _is_ the site. A central entry must name the site
indirectly — by enclosing function, by file, by substring — and every
indirection is a region that absorbs new members silently. This is decisive and
it does not depend on the population.

**Census.** The central list's one genuine advantage was that a human could read
the whole exempt set in one place and ask "is this still small, and is each
still true?" — the only control available for a premise nothing re-verifies.
**That advantage dissolves once the census is derived.** The gate already visits
every site, so it can emit the complete set — `file:line — reason` — from the
scan itself. A derived census cannot go stale; a declared one can, and detecting
that drift is the _sole_ reason the multiplicity-reconciliation pass existed.

**Review weight.** A central entry forces an edit to a gate file, which reads
louder in review than a comment in the file the author was already changing.
This is real, and it is the one thing markers give up. It is not enough to
outweigh a wrong key.

Therefore: **an in-source marker is the default for any gate whose exempt sites
are source lines.** A central list earns its place only where a marker cannot
attach — the "site" is an absence, a configuration value, or a whole-file
property — or where the population is large enough that a printed census would
be unreadable.

### Stakes set strictness, not mechanism

Blast radius does not choose between marker and list; it prices the marker.
Coverage's population is ~700 sites whose worst case is an untested line, so
`cov:ignore` accepts a bare marker and offers a block form. The XSS doors are
twelve sites whose worst case is stored XSS, so their markers are strict: a
reason is required, there is no block form, one site per line, and an **orphan
marker fails**.

That last rule is the only thing a machine _can_ check about a written exemption
— not whether the reason is true, but whether the thing it exempts still exists.
A marker whose site was deleted or moved off the line is a live, pre-approved
exemption sitting on a line a future edit can re-populate. Failing it is the
closest available analogue to the tripwire that inferred exemptions get, and it
is where a strict population should spend its budget.

### Relationship to ADR-0085

ADR-0085's six principles are unchanged, and a marker satisfies them better than
the central list did: the population is still read structurally (principle 1),
every member still fails without a human-written reason (principles 2 and 3),
and the entry is now scoped to one site rather than a region (principle 4).

One sentence of ADR-0085 **is** superseded. Its Consequences argue for the
central form on co-location grounds — _"each entry … lives next to the rule that
would otherwise flag it. That co-location also discharges the recurring 'record
why these sites are fine so nobody re-audits them' requirement."_ That reasoning
was right about the requirement and wrong about which co-location serves it: the
reader who needs the justification is reading the **code**, not the gate. Under
this ADR the reason lives next to the site. ADR-0085 is amended to say so.

`sqlx-newtype-decode` keeps its per-entry multiplicity and its central list. Its
population _is_ principle 4's indistinguishable-sites case, which is what that
clause was written for.

## Consequences

**What this commits us to.** A new gate's author chooses a mechanism
deliberately, from the discriminator above, rather than by copying the last
gate. A gate whose population is un-recheckable must derive and print its
census. Exemption markers are per-gate, line-scoped, reason-bearing, and never
blocked.

**What it creates.** Twelve in-source markers — seven replacing the three
allowlists' entries, and five more where the deleted qualifier-pattern exemption
used to cover `ContentType::from_trusted` sites for free. (Four of those five
are gone again since [#790](https://github.com/jaunder-org/jaunder/issues/790),
which recovered that coverage by _resolving_ the qualifier rather than by
pattern; the fifth is `RenderedHtml`'s own definition, which is genuinely this
gate's door and keeps its marker.) `ident_gate` loses `Allowed`, `unjustified`,
the multiplicity reconciliation, and `Mention::top_level` — the last of which
existed only so a nested fn could not borrow a fn-name-keyed entry, a problem
markers do not have. Deleting the qualifier pattern also collapses the custom
`Population` impl, so all three gates become the same shape.

It collapses a duplication that was live: every sink's justification was written
twice, once in prose beside the code and once condensed in the gate's allowlist,
with nothing keeping the two in sync.

**What it rules out.** Function-, file-, or region-keyed exemption entries for
these gates. Block-form markers. Shared markers across gates. Pattern-decided
exemptions. And multiplicity as a _repair_ for a mis-keyed entry — it is a
fallback for genuinely indistinguishable sites, and reaching for it on a
distinguishable one hides a wrong key behind an arithmetic check.

**What it costs, accepted rather than solved.** Two things get worse:

1. **Review weight drops.** The exemption now lands in the author's own diff
   instead of forcing an edit to a gate file, which is a louder signal.
   Requiring a reason and failing a bare marker is the only mitigation; it is
   partial. The counterweight is that the gate's `recovery:` prose already
   teaches the author what they tripped, at the moment of failure, regardless of
   which mechanism carries the exemption — so the gate file was never the only
   place that teaching happened.
2. **A marker is trusted, not verified.** The gate checks that a reason exists
   and that its site still exists; it never checks that the reason is true. The
   central list had this blind spot equally, and so does every written exemption
   in this repo including `cov:ignore`. Stating it is ADR-0085's honesty
   obligation, and each gate's module doc now carries it.

**What it does not claim.** That markers are safer than a central list in
general — only that they are better keyed, and that the census argument which
favoured the central list dissolves once the census is derived from the tree. It
also does not claim to close the provenance question: no gate here has a call
graph, so a marked site is exempt regardless of what value flows into it.
Narrowing the exemption from a function to a line shrinks that window; it does
not shut it.
