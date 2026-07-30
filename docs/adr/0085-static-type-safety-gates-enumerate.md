# ADR-0085: Static type-safety gates enumerate, they do not search

- Status: proposed
- Date: 2026-07-30
- Issue: [#715](https://github.com/jaunder-org/jaunder/issues/715)

## Context

The domain-value newtype convention
([ADR-0063](0063-domain-value-newtype-convention.md)) and the sqlx bridge
([ADR-0071](0071-sqlx-string-newtype-bridge.md)) are only as strong as the
checks that stop primitives leaking back in. We have written those checks three
times, and each time the check found exactly what its author's chosen spelling
could match, then reported done.

The record is unambiguous:

- #686 first audited by **column name**. It missed five sites, because tuple
  positions have no names.
- It then audited by **`query_as::<_, ( … )>` tuple shape**, and swept 18 sites.
- #715 found that pass had missed every `query_scalar` decode — a different
  spelling for the same act — plus a `query_as` site that wrote its type on the
  `let` rather than in a turbofish, and two `row.get` decodes. Eleven sites,
  invisible to a search built around one spelling.
- `sqlx-newtype-bind` (#438, #686, #696) decides a bind violation by asking
  whether the bind region contains `.as_ref()`, `&*`, or `i64::from(`. #716 is
  open because a strip laundered through an `i64` function parameter has none of
  those spellings.

This is one failure mode wearing four costumes, and it is worth naming
precisely, because it looks like diligence from the inside. **A check that
searches for the shape you expect can only confirm your hypothesis. It cannot
discover that reality is bigger than the hypothesis.** It reports green not
because the code is clean but because the code contained nothing the author had
already thought of. Every green run reinforces the belief that the enumeration
was complete, which is what made three successive passes each feel conclusive.

The tempting fixes are themselves searches. "Detect id columns" means grepping
the SQL projection for `*_id` — so `oid`, `owner`, `parent`, or any aliased
column is invisible. "Deny bare `i64`, but exempt counts" means grepping the SQL
for `COUNT(` — so `SELECT post_id FROM t WHERE (SELECT COUNT(*) …) > 0`
self-exempts. Moving the pattern match from the violation path to the exemption
path hides the blind spot; it does not remove it.

## Decision

**A static gate that enforces a type-safety invariant must enumerate the
population it governs and deny by default. It must not decide violations, or
exemptions, by searching for anticipated spellings.**

Concretely, such a gate:

1. **Defines its population structurally**, from a property the compiler or the
   AST can read directly — a decode target's type, a bind expression's argument
   — not from a pattern believed to characterise violations.
2. **Fails on every member of that population** unless the member appears in an
   enumerated allowlist carrying a written reason.
3. **Grants no automatic exemption from a pattern.** Nothing self-exempts. An
   exemption is an entry a human wrote, or it does not exist.
4. **Scopes each allowlist entry to a single site**, never to a region. An entry
   that exempts a function, a file, or "every line containing this substring"
   re-creates the blind spot one level down: a new violation added inside its
   reach passes silently. Where a population genuinely contains
   indistinguishable sites, the entry must state their **multiplicity**, so that
   gaining one more is a mismatch and a failure rather than a silent absorption.
5. **Parses rather than scans**, when the invariant spans more than one line. A
   line-based scan cannot relate a decode type on one line to the SQL on the
   next, and the workarounds for that limitation are exactly the pattern
   searches this ADR forbids.
6. **Fails on input it cannot read**, rather than skipping it. A source file
   that will not parse, or a scan root that has moved, must be a hard failure —
   a gate that quietly shrinks its own population reports green for the one
   reason it must never report green. This is the same rule as 2, applied to the
   gate's own inputs.

The test that a gate conforms is not "does it catch the known sites" — a search
passes that test. It is: **does an unanticipated member of the population
fail?** A conforming gate rejects a novel construct _because it recognised
nothing_, which is the only claim a static check can honestly make.

The cost is deliberate and load-bearing. An enumerating gate makes legitimate
primitive use cost a written allowlist entry, and that friction lands on common,
harmless operations. We accept it: the friction _is_ the mechanism. A gate that
is free to work around is a gate that reports green.

### Conformance

- **`sqlx-newtype-decode`** (#715) conforms. Its population is every sqlx decode
  under `storage/src` whose target resolves to the `i64` family — recursing
  through `Vec`, `Option`, `Result`, references and tuples — read from the AST
  at the nearest place the type is _written down_ (turbofish, then `let`
  ascription, then the enclosing `fn` return), plus every declared decode target
  (`FromRow` struct fields, tuple aliases). It inspects no SQL; each of the ten
  allowlist entries names one decode and its multiplicity, with a written
  reason; and an unparseable file under the root is a hard failure rather than a
  silent skip.

  Its honesty obligation (below) is discharged in its module doc: two constructs
  are **outside** the population because `syn` cannot resolve them — a
  `.get`/`try_get` with neither turbofish nor ascription (indistinguishable from
  `serde_json::Map::get`, and both live under the root), and a decode typed only
  by later use.

- **`sqlx-newtype-bind`** (#438, #686, #696) **does not conform**, on two
  counts.
  - It violates principle 3: a violation is `region.contains(".as_ref()")` or
    `"&*"` or `"i64::from("` — a search for the three strip spellings someone
    thought of. Its own module doc concedes it "cannot be called complete."
  - It violates principle 4: its `ALLOWLIST` matches by substring, and its own
    doc states the consequence — "a needle exempts every matching line under
    `POLICED_ROOT`, not one site." That is a region-scoped exemption.

  [#716](https://github.com/jaunder-org/jaunder/issues/716) is the outstanding
  work to rebuild it as an enumerating gate: deny bare-primitive binds in
  `storage/src` unless allowlisted, with site-scoped entries.

  **On what that rebuild would and would not achieve for #716's instance**,
  since the two are easy to conflate. #716 records `list_published_in_window`
  stripping a `FeedMinItems` to `i64` and passing it to
  `list_published_in_window_rows`, which binds the bare parameter. An
  enumerating bind gate **would flag** that bind — it is a bare-primitive bind
  in `storage/src` with no allowlist entry, and the gate does not need to know
  where the value came from to reject it. What it **could not** do is
  _attribute_ the failure to a newtype strip in the caller, or point at the line
  that did it; the author gets "this bind is untyped, fix it or justify it" and
  has to trace the seam themselves. Detection is within reach of an enumerating
  gate. Attribution across a function boundary needs call-graph analysis and
  remains out of reach.

Recording the non-conformance is the point. #716 was previously "a known limit
of a line-based scan"; under this ADR it is a violation of a decided principle,
with a decided direction. That issue is updated to carry this scope rather than
having it asserted for it here.

## Consequences

**What this commits us to.** New type-safety gates start from "what is the total
population, and how do I read it structurally?", not "what does a violation look
like?" Allowlists grow over time and that growth is expected, not a smell — each
entry is a deliberate, reviewed statement that a site is genuinely primitive,
and it lives next to the rule that would otherwise flag it. That co-location
also discharges the recurring "record why these sites are fine so nobody
re-audits them" requirement without a separate prose document to go stale.

**What it creates.** #716 is re-framed from an acknowledged gap into scheduled
work. Rebuilding `sqlx-newtype-bind` to enumerate will require an allowlist for
every legitimate primitive bind in `storage/src`, which is a larger population
than the decode side — that cost is now decided rather than debated.

**What it rules out.** Gates that grep for violation spellings; exemptions
inferred from SQL text or any other content heuristic; allowlist entries scoped
to a file, module, or function. It also rules out the "quick line-based check"
as an acceptable form for a multi-line invariant, which raises the floor cost of
adding a gate — a real trade, made knowingly, because the cheap form is what
produced four incomplete audits.

**What it does not claim.** This is a rule about static gates, not a claim that
static gates are sufficient. A conforming gate's population is bounded by what
its parser can read: `syn` has no type information, so a decode whose type is
never written down — `let value = row.get(…)`, where the receiver could be an
sqlx row or a JSON map — is outside the population, not exempt from it.
Attribution across a function boundary is likewise out of reach (see #716
above). Enumerating shrinks the blind spot to something statable; it does not
abolish it.

The obligation this creates is that a conforming gate must **state its
unreadable classes in its own documentation**, so the boundary is inherited by
the next audit rather than rediscovered. A gate that quietly omits what it
cannot see is back to implying, by a green run, that it looked everywhere.
