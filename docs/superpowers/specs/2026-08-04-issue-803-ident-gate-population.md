# Spec — #803: collapse `ident_gate::Population` into a plain ident set

Issue: [#803](https://github.com/jaunder-org/jaunder/issues/803) Branch:
`worktree-issue-803-ident-gate-population`

## Problem

After [#778](https://github.com/jaunder-org/jaunder/issues/778) deleted
`TrustedDoor`, the `ident_gate::Population` trait has exactly one implementor —
`AnyOf` — and that implementor uses neither of the two parameters the trait
exists to pass:

```rust
fn macro_ident(&self, id: &Ident, _trees: &[TokenTree], _idx: usize) -> bool {
    self.ident(id)
}
```

The abstraction is paying for **zero** variation. All three gates spell their
population identically (`AnyOf(SINKS)`, `AnyOf(DOORS)`, `AnyOf(DOORS)`), and the
type parameter it forces threads through `scan<P>`, `Scanner<'p, P>`, `Gate<P>`,
and three `const GATE: Gate<AnyOf>` declarations across four files.

This is not a correctness problem — the gates pass, and their verdicts are
right. It is filed because #778's thesis was deleting machinery that had stopped
earning its keep, and a trait with one implementor and two dead parameters is
that same smell one layer up.

## Decision

**Collapse the trait** (issue option 1). `Population` and `AnyOf` are deleted; a
gate's population becomes a plain `&'static [&'static str]` of idents, carried
directly on `Gate`. `scan`, `Scanner`, and `Gate` all lose their type parameter.

### The field keeps the name `population`

`Gate`'s field is `population: &'static [&'static str]`, not `idents`. ADR-0085
principle 1 is stated in terms of a gate's _population_ being read structurally,
and each of the three gate modules opens with a
`**Population** (read structurally, ADR-0085 principle 1)` doc header. Keeping
the field name means the code goes on naming the concept the ADR and the module
docs name; the type change alone carries the simplification.

### Two seams are removed, deliberately

The trait stacked two independent seams. Both go:

| Seam                                 | What it allowed                                                    | Last user                 |
| ------------------------------------ | ------------------------------------------------------------------ | ------------------------- |
| Two hooks (`ident` vs `macro_ident`) | A gate answering differently in macro bodies than in ordinary code | Never used                |
| Positional context (`trees`, `idx`)  | Reading neighbouring tokens (e.g. a path qualifier to the left)    | `TrustedDoor`, until #778 |

After the collapse, membership is ident-identical in ordinary code and inside
macro tokens **by construction** rather than by an implementor's choice.

### `AnyOf`'s doc survives the deletion of `AnyOf`

The trait doc is not the only prose being deleted. `AnyOf`'s own doc
(`ident_gate.rs:173-179`) carries the **principle 3** argument — that matching
the ident rather than a call shape is what keeps the gate an enumeration instead
of a search for the spelling someone anticipated, so a builder call, a struct
field and a bare reference are all inside the population rather than silently
outside it. That is the paragraph justifying why these gates are ident-keyed
**at all**, and it must move onto the `population` field rather than die with
the type that currently hosts it.

### The trait doc's argument is preserved as a fact about the scan

The trait doc argued:

> Both hooks are required rather than defaulted: a gate must say what it does
> _not_ look at, because "I never implemented that hook" and "that construct is
> outside my population" are the same silence otherwise.

That principle is real, and it does not survive as a _per-gate obligation_ once
there is nothing per-gate to vary. It is restated in the module doc as a
property of the scan itself — every gate reads idents everywhere, so there is no
hook a gate can silently fail to implement — together with the re-plumbing path
if a positional-context gate is ever needed again (`walk_macro_tokens` already
holds `trees` and `idx`; a future gate re-introduces a seam there rather than
resurrecting the trait blind).

### ADR-0094 is not edited

`docs/adr/0094-gate-exemptions-in-source-markers.md:233` — "Deleting the
qualifier pattern also collapses the custom `Population` impl, so all three
gates become the same shape" — sits in **Consequences**, in the present tense,
so it is a statement about #778's effect rather than pure narrative. It
nonetheless stays **true** after #803: it describes `TrustedDoor`'s custom impl
collapsing into the shared one, which is what happened. ADRs record decisions at
a point in time and are not retconned, so it is left as written.

## Scope

**In scope** — `xtask/src/steps/ident_gate.rs` and the three gate modules
(`html_sink_check.rs`, `raw_html_door_check.rs`,
`rendered_html_from_trusted_check.rs`).

**Out of scope** — the marker mechanism (`classify`, `markers.rs`, orphan
detection), the traversal's test-code handling, the set of idents any gate
polices, the gates' report prose, and the roots they scan. No gate's verdict on
any file changes.

**Deliberately unchanged, not overlooked:** `run_scan` (`ident_gate.rs:561`)
never sees the population — it takes `step`, `roots` and a closure — so it needs
no edit. Recorded here so a conformance reviewer can tell the difference.

### Sites the collapse touches beyond the four named above

- `Gate::violations` (`ident_gate.rs:461-476`, `#[cfg(test)]`) calls
  `scan(source, &self.population)` and becomes `scan(source, self.population)`.
  It is in neither `tests` nor `marker_tests`, but it is the entry point every
  unit test in the three gate modules runs through (`html_sink_check.rs:108`,
  `raw_html_door_check.rs:97`, `rendered_html_from_trusted_check.rs:118`), so it
  must keep working unchanged in behavior.
- `Scanner` **keeps its lifetime parameter** `'p`, now holding
  `population: &'p [&'p str]`. Only the _type_ parameter goes. Taking `'static`
  here instead would contradict `scan`'s mandated signature and force the unit
  tests' literal arrays into consts.
- The three `impl<P: Population>` blocks (`ident_gate.rs:315`, `:355`, `:453`)
  lose their parameter and bound.
- The two intra-doc links ``[`Population`]`` in the module doc
  (`ident_gate.rs:10`, `:17`) point at an item that will no longer exist and
  must be rewritten. No rustdoc lint gate exists in this repo
  (`Cargo.toml:110-115` is clippy-only), so a broken link would rot silently
  rather than fail the gate.
- `rendered_html_from_trusted_check.rs:216-221` has a test doc comment reading
  "Under `AnyOf` the door's own declaration is in the population — a deliberate
  behavior change…". It records a real #778 behavior change, so its replacement
  must preserve that meaning while naming the mechanism rather than the deleted
  type — it is not a mechanical find-and-replace.
- The three gate modules' unit tests otherwise need **no** change: they go
  through `Gate::violations`, not through the population type.

## Acceptance criteria

1. `rg -n '\bPopulation\b' xtask/src/` returns **only** `//!` or `///` doc lines
   — no line matching `trait `, `impl .* for `, `<P`, or `: Population`. (The
   word is deliberately retained in prose and, lowercased, as the field name;
   the check is case-sensitive, so the `population` field is not among these
   hits.)
2. `rg -n '\bAnyOf\b' xtask/src/` returns nothing.
3. `Gate`, `scan`, and `Scanner` each have **no type parameter**. `Scanner`
   **keeps** its lifetime `'p` (`population: &'p [&'p str]`); `scan`'s signature
   is exactly
   `pub fn scan(source: &str, population: &[&str]) -> Result<Scan, String>`.
4. `Gate` has a field `population: &'static [&'static str]` whose doc comment
   cites ADR-0085 principle 1 **and** carries forward `AnyOf`'s principle-3
   argument (ident matching, not call-shape matching, is what makes the gate an
   enumeration — a builder call, a struct field and a bare reference are all
   inside the population).
5. Each of the three gate modules declares `const GATE: Gate = Gate { … }` with
   `population: SINKS` (html-sink) or `population: DOORS` (raw-html-door,
   rendered-html-from-trusted), and no longer imports `AnyOf`.
6. The `ident_gate` module doc states (a) that membership is ident-identical in
   ordinary code and in macro tokens by construction, and (b) that a future gate
   needing positional context re-introduces a seam **in `walk_macro_tokens`**,
   which already holds `trees` and `idx`. Both intra-doc links at
   `ident_gate.rs:10` and `:17` resolve to a live item.
7. The module's "Unreadable classes inherent to this scan" list still has **six
   numbered classes**, none added and none removed; wording changes only where
   it referenced the trait.
8. Every existing test in `ident_gate.rs` (`tests` and `marker_tests`) still
   exists and still asserts the same behavior, adapted only at the four sites
   that construct `AnyOf(&[…])` (`ident_gate.rs:597`, `:605`, `:617`, `:622`).
   `Gate::violations` still compiles and every unit test in the three gate
   modules still passes unmodified.
9. Macro-body matching remains covered by test: a site inside a `m! { … }` body
   is still found by `scan` and still exempted by a marker on the line above
   (`mentions_come_back_in_line_order`,
   `a_site_inside_a_macro_body_is_exempted_from_the_line_above`). These are the
   regression lock on the seam being removed — the collapse must not be able to
   silently drop the hand-rolled macro token walk.
10. **The derived census is identical before and after**, checked mechanically
    rather than inferred from a green gate. A green
    `cargo xtask validate --no-e2e` proves only that the gates _pass_; a gate
    whose population silently shrank also passes green — exactly the ADR-0085
    principle-6 failure the module doc names at `ident_gate.rs:63-65`. So:
    across the policed roots, each gate's classification must come back **12
    marked, 0 unexempt, 0 orphans in total** — the census measured at this
    branch's fork point (`wt-base-issue-803`): `common/src/media.rs` ×3,
    `common/src/feed/feed_path.rs` ×1, `common/src/render.rs` ×2,
    `web/src/posts/component.rs` ×3, `web/src/sidebar/component.rs` ×1,
    `web/src/html.rs` ×1, `web/src/home/component.rs` ×1. If the branch rebases
    onto a moved `main`, re-take the snapshot at the new fork point and compare
    against that; the invariant is _equality across the change_, not the literal
    number 12.
11. `cargo xtask validate --no-e2e` is green.

## Notes

Raised in the #778 whole-branch review and deliberately left out of scope there.
The counter-argument — that the trait is the seam a fourth gate would extend,
and that `TrustedDoor` proves such a gate is not hypothetical — was weighed and
accepted as a cost: re-adding a parameter later is cheap, git history preserves
`TrustedDoor`'s shape, and criterion 6 leaves the next author a signpost rather
than a blank.
