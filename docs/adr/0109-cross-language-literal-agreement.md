# ADR-0109: Cross-language literal agreement is enforced by a declared pair table

- Status: accepted
- Date: 2026-08-10
- Issue: [#767](https://github.com/jaunder-org/jaunder/issues/767)

## Context

A handful of constants in this repo are necessarily spelled twice, once in Rust
and once in TypeScript, because no import can span the boundary between them:

- The CSR mount marker. `csr/src/lib.rs` sets `data-mounted` from inside a
  `wasm_bindgen(inline_js = …)` string — an opaque JS literal as far as Rust is
  concerned — and `end2end/tests/mount.ts` declares `MOUNTED_ATTR` for the whole
  Playwright suite to read.
- The boot-mark prefix. `client::perf::MARK_PREFIX` is `"jaunder."`, and
  `end2end/tests/capture-trace.ts` re-declares the same prefix to discover marks
  in the browser.

#251 (D6) already decided that these duplications are necessary rather than
accidental, and gave each side a comment naming its counterpart. A comment is
all that held them together.

**Drift in this class is silent at build time and maximally expensive at test
time.** Nothing fails to compile. `cargo xtask check` never runs e2e, so it
cannot see it. The whole `{sqlite,postgres}×{chromium,firefox}` matrix goes red
with dozens of Playwright timeouts — twenty-five minutes of CI whose output
contains no mention of the one-word typo that caused it. This is precisely the
failure profile the repo builds static gates for.

The obvious gate is a bespoke check comparing the two mount-marker literals. The
`mark-prefix` pair shows why that is the wrong shape: the class already has two
members, discovered independently (#794 recorded the prefix's drift risk as an
accepted limitation), and a bespoke check per member means the second one is
written from scratch.

The design tension is with
[ADR-0085](0085-static-type-safety-gates-enumerate.md), which forbids a gate
from deciding violations by searching for anticipated spellings. Locating a
literal in a source file looks like exactly that. (0085's Decision is scoped to
gates enforcing a **type-safety** invariant, so it does not strictly reach this
one — but the reasoning is about gate honesty, and deserves an answer rather
than a jurisdictional dodge.)

## Decision

**Cross-language literal agreement is enforced by one gate driven by a declared
table of pairs.** Each entry names a key and two sites; each site is a file
path, a literal **anchor** string, and the quote character that opens the
literal following it. The gate loops the table and fails a pair whose two
extracted literals are not string-equal.

**The anchor locates a site; it never decides a violation.** The violation is
exact string inequality between two extracted literals. This is what reconciles
the design with ADR-0085, and the reconciliation is the failure direction, not a
concession:

- An incomplete **violation** detector is silent — the defect ADR-0085 exists to
  prevent.
- An out-of-date **locator** is loud. It matches zero times, or two, and both
  are hard failures.

So the gate must fail on **zero anchor occurrences and on more than one**, and
on an unreadable file (ADR-0085 principle 6). A locator that has quietly stopped
locating anything is the one thing this gate must never treat as a pass — a
renamed constant, a moved file, or a reformatted line disarms the gate, and the
gate must say so rather than report green.

**On ADR-0085 principle 5, "parse rather than scan."** That principle exists
because a line-based scan cannot relate a decode's type on one line to the SQL
on the next, and the workarounds for that limitation are the violation-pattern
searches 0085 forbids. This invariant spans no lines: each literal is a
single-line declaration, and the comparison is exact equality between two
extracted strings, not a judgement about surrounding code. So a scan is adequate
here in the precise sense principle 5 cares about — there is no cross-line
relation for it to get wrong. A parser (`syn` for the Rust sides) would buy
robustness against reformatting, at the cost of two extractor kinds and no help
at all on the TypeScript sides, which no Rust parser reaches.

**The anchor is the syntax that introduces the literal, never the literal's own
value.** `setAttribute(`, not `data-mounted`. (Usually that is a declaration —
`MOUNTED_ATTR = `. The CSR site is the exception: its literal exists only inside
a JS string, so there is no Rust constant to name and the anchor is the call.)

This is not incidental. Every policed site carries a prose comment naming its
counterpart, and at least one of them — `csr/src/lib.rs`, which quotes
`body[data-mounted]` — mentions the value. A value-anchor would match that
comment too, so editing documentation could change the gate's verdict. One such
comment is enough to rule the approach out.

**The gate asserts agreement, not a value.** Renaming a marker consistently on
both sides passes. The gate has no opinion about what the literal should be; it
refuses only to let the two sides disagree.

**The gate's population is the table, and the table's completeness is not
claimed.** A third duplicated literal that nobody adds to the table is unpoliced
and recorded nowhere. Per ADR-0085's honesty obligation, the gate states this
limit in its own module documentation rather than letting a green run imply it
looked everywhere. This is an inherent property of a declared-pair design: there
is no structural property of the tree that distinguishes "a literal duplicated
across languages on purpose" from "two files that happen to contain the same
string."

**The counterpart comments stay, and name the gate.** The comment on each site
is now the pointer a reader follows to learn what enforces the agreement; it is
no longer the enforcement itself.

## Consequences

**What this commits us to.** A new cross-language duplicated literal is expected
to arrive with a table row. The cost is one line plus two anchors, which is low
enough that "I'll add it later" has no excuse — and a test that resolves the
whole table against the real tree means a refactor that breaks an anchor fails
in `cargo xtask check` rather than months later.

It also commits us to **no new dependency for this**. `xtask` is its own cargo
workspace, so a crate added for a gate is compiled into every `cargo xtask`
invocation — including the pre-commit `check` that runs dozens of times a day.
Anchor extraction is forty lines of `&str` work; `regex` would be three crates
on that path. Where a future pair genuinely cannot be anchored, the answer is
`syn` (already a dependency) for the Rust side, not a matcher crate.

**What it rules out.** A bespoke per-constant agreement check. Also an anchor
written against a literal's _value_ rather than its declaration, which would let
a comment mentioning the value break the gate. Also a gate that tries to
_discover_ cross-language duplication heuristically — scanning for string
literals appearing in both a `.rs` and a `.ts` file would produce a population
of coincidences, and the exemption list needed to quiet it would be the
region-scoped kind ADR-0085 forbids.

**What it does not claim.** That the table is complete (above). That agreement
is correctness — two sides can agree on a value that is wrong for some third
reason; this gate would pass. And that the literal is the only thing that can
drift: a site whose _semantics_ change while its spelling holds is invisible
here, as it is to every string-equality check.

It also compares **source spellings, not decoded values**: a backslash escape is
kept verbatim rather than decoded, so two sides agreeing on the decoded string
while spelling the escape differently would read as drift. That is deliberate —
decoding would mean modelling two languages' escape vocabularies inside the gate
— and it errs in the safe direction, a false failure rather than a false pass.
No shipped pair contains an escape.

**Relation to [ADR-0094](0094-gate-exemptions-in-source-markers.md).** That ADR
governs where a gate's _exemptions_ live, and prefers in-source markers to a
central list because a derived set cannot go stale. This gate has no exemptions
— a pair either agrees or it does not, and there is nothing to excuse — so the
discriminator does not apply. What is central here is the _population_, and a
central population is what makes the honesty limit above statable.
