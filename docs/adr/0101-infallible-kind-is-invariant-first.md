# ADR-0101: The infallible newtype kind is invariant-first, and a trusted door can be typed away

- Status: accepted
- Date: 2026-08-05
- Issue: [#830](https://github.com/jaunder-org/jaunder/issues/830)

## Context

[ADR-0063](0063-domain-value-newtype-convention.md) defines two shapes for a
string newtype. §2 frames the choice **invariant-first**:

> `FromStr` — the single validating/normalizing chokepoint. Fallible **when the
> value has an invariant** …

but §3 defines the infallible kind by a test on the **constructor's signature**:

> For a value whose invariant never rejects (only normalizes, or wraps
> verbatim), `#[str_newtype(infallible)]` … First users: `PostBody`/`PostTitle`
> (#402).

Those two tests disagree, and the disagreement is not academic. `PostTitle`
passed the signature test — its `From<String>` only trimmed, and never rejected
— while plainly having an invariant: a blank title is nonsense, distinct from an
_absent_ one. The codebase believed in that invariant and enforced it
**everywhere except in the type**: three separate call-site filters, one of them
re-checking a title already read back out of the database and calling itself
"its one invariant gap"; a `debug_assert` in `PostSummary::truncated` that
leaned on a guarantee `PostTitle` did not make; and a data migration
(`0010_nullable_post_titles.sql`) that had already swept `title = ''` rows to
`NULL` once.

A definition that asks "does the constructor reject?" invites this, because the
answer is a property of the code you have already written. The invariant-first
question — "does this value have a rule?" — is the one that produces the right
type.

Separately, §2's **truncating door** paragraph describes
`PostSummary::truncated` as a _trust_ door: infallible, guaranteeing the length
half of the invariant but not the non-emptiness half, with a `debug_assert`
standing in for the part it could not enforce and callers obliged to remember
the rest. That was the best available shape while the caller's inputs were
untyped strings. It is not the best available shape once they aren't.

## Decision

**1. The infallible kind is defined invariant-first.**
`#[str_newtype(infallible)]` is for a value with **no rule that can fail** — one
that normalizes, or wraps verbatim, and for which _no input is invalid_. It is
not for a value whose rule happens to live at its call sites. If a value has an
invariant, it gets a validating `FromStr`, even when today's construction sites
all happen to satisfy it.

The reviewer's question is not "does the constructor reject?" but "is there a
string this type should refuse?"

**2. `PostTitle` and `PostBody` are removed from §3's first-users list.**
`PostTitle` becomes validating in #830. `PostBody` follows in
[#811](https://github.com/jaunder-org/jaunder/issues/811), which makes a blank
body unrepresentable on the same reasoning — this amendment covers both so the
correction is written once rather than re-derived. Until #811 lands, `PostBody`
remains infallible in code; that gap is deliberate and bounded by the issue.

**3. A trusted door should be replaced by a typed proof where the caller can
supply one.** When a door is infallible only because its caller promises
something, prefer a **seed type** whose constructors are infallible _because
each source proves the property_, over a `debug_assert` plus a documented
precondition.

`PostSummary::truncated` becomes the worked example: it now takes a
`SummarySeed`, whose three constructors — from a `Slug`, from a `PostTitle`,
from the first non-blank line of a `PostBody` — are each infallible because
their source is already non-blank. The `debug_assert` is gone, and so is the
caller's obligation to remember it. What remains is a plain length-capping door,
which is honest: length is the one half of the invariant this door genuinely
coerces.

This does **not** retire the truncating/trusted door generally. It is the right
shape when no caller can supply a proof — a value arriving from outside the type
system, or a source too diffuse to name. The rule is: reach for the trusted door
when the proof cannot be typed, not when typing it would be work.

## Consequences

- ADR-0063 §3's definition and first-users list change, and §2's truncating-door
  paragraph gains the seed alternative. The paragraph's `PostSummary` example
  inverts: it now demonstrates the typed proof rather than the trust door.
- A newtype review question changes shape. "Does the constructor reject?" is
  replaced by "is there a string this type should refuse?" — answerable from the
  domain, before any code exists.
- #811 inherits this amendment and does not need its own.
- New cost: a seed type is more vocabulary than a `debug_assert`. Justified only
  where a precondition is real and provable; a door whose callers genuinely
  cannot prove the property keeps the trusted shape.
- Existing infallible newtypes are not audited by this ADR. `PostTitle` was
  found by way of a performance issue
  ([#758](https://github.com/jaunder-org/jaunder/issues/758)), not a sweep. Any
  remaining mislabelled type stays mislabelled until someone looks — this
  changes the rule, not the code.
