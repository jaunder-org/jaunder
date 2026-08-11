# Spec — `rendered-html-from-trusted` resolves qualifiers (#790)

Issue: [#790](https://github.com/jaunder-org/jaunder/issues/790) Milestone: Web:
canonical Leptos CSR convergence

## Problem

The gate's population is the bare leaf ident `from_trusted`
(`xtask/src/steps/rendered_html_from_trusted_check.rs:82`), so it contains every
`from_trusted` in policed code regardless of owning type.
`ContentType::from_trusted` (`common/src/media.rs:873`) mints a media type, can
never be HTML, and can never reach the unescaped sink — yet it and its call
sites carry four marker comments whose only content is "this is a different
door".

The justification for leaf matching is that path matching fails open under
aliasing. That does not hold: a `use` alias is the most syntactically visible
thing in a file, the gate already parses these files with `syn`, and the
specific hole #778 cited (`use RenderedHtml as ContentType`) is itself a `use`
alias.

The cost is structural, not specific to `ContentType`: **any** type that wants a
conventional `from_trusted` inherits the same friction.

## What was rejected, and why

- **Renaming `ContentType::from_trusted`** — the issue as originally filed. It
  makes the innocent party pay for the gate's approximation and fixes nothing
  structurally; the next type wanting a conventional `from_trusted` hits the
  identical friction.
- **Renaming `RenderedHtml::from_trusted`** — moves the problem instead of
  removing it, and carries a larger documentation blast radius (ADR-0079,
  ADR-0091, ADR-0094, and the gate's own step name).
- **Perfect Rust resolution** — unworkable and not the goal. The goal is that
  the gate stop restricting people from naming a function consistently with
  similar functions elsewhere.

## Decisions

### D1 — The rule: prove it is not the door, or flag

At each `from_trusted` site the gate must establish that it is **not**
`RenderedHtml::from_trusted`. Inability to establish that is itself a failure,
so the gate never fails open.

| site                                               | behaviour                             |
| -------------------------------------------------- | ------------------------------------- |
| qualifier ident is in the **owner set** (D2)       | in population; needs a marker         |
| `Self::from_trusted` inside `impl RenderedHtml`    | in population; needs a marker         |
| `fn from_trusted` defined in `impl RenderedHtml`   | in population; needs a marker         |
| qualifier resolves to any other type               | **not** this door; ignored, no marker |
| `fn from_trusted` defined in any other `impl`      | **not** this door; ignored, no marker |
| qualifier **unresolvable** (D3)                    | **flagged**                           |
| unqualified `from_trusted(…)` call                 | **flagged**                           |
| `fn from_trusted` free at module scope (no `impl`) | **flagged**                           |

The unresolvable rows are the whole trick: obscuring a qualifier buys a gate
failure, not an exemption.

### D2 — The owner set, harvested across policed roots

Built once per run, before classification:

- the owner's own name, `RenderedHtml`;
- every ident `X` where any policed file contains `use …::RenderedHtml as X;` —
  a **renaming re-export**, which is the one case per-file resolution would
  miss;
- every ident `X` where any policed file contains `type X = RenderedHtml;`.

Both harvests are **tree-wide**, not per-file. Widening the owner set only ever
moves sites _into_ the population, so it is the fail-closed direction; keeping
the two harvests symmetric is simpler than resolving one globally and one
per-file, and it subsumes the per-file alias case entirely.

This costs one extra cheap pass over files the runner already reads, and means
the change **loses no coverage relative to today's leaf matching** — including
`pub use crate::render::RenderedHtml as Doc;` in one module and
`Doc::from_trusted(…)` in another.

_Rejected:_ per-file resolution only. Simpler, and it would still delete all
four markers, but a renaming re-export would slip past — coverage the gate has
today.

### D3 — What counts as resolvable

A qualifier ident is **resolved** when it is:

- in the owner set (D2) → the door; or
- **spelled out** in a multi-segment path
  (`crate::media::ContentType::from_trusted`) → the segment before the leaf
  names the type, so it resolves by construction; or
- bound by a non-glob `use` item in this file — including a nested group,
  `use crate::{media::ContentType, tag::Tag};` → resolves to that path's final
  segment; or
- defined in this file as a `struct`, `enum`, `union` or `type` alias → resolves
  to itself; or
- `Self` inside an `impl` whose self-type is a path → resolves to that path's
  final segment.

The multi-segment rule resolves the type _name_; it does not exempt it. A
spelled-out owner path (`crate::render::RenderedHtml::from_trusted`) resolves to
the owner and stays in the population.

Anything else is **unresolvable** and is flagged — notably a qualifier that
could only have arrived through a glob import (`use foo::*`), and a generic
parameter (`T::from_trusted`).

Residual blind spot, accepted and to be documented: a chain of re-exports that
renames `RenderedHtml` **more than once** across modules. D2 harvests single
renames; a rename of a rename evades. This is the same class of blind spot the
house already accepts elsewhere (`xtask/src/steps/proffered_secret_check.rs:89`,
`xtask/src/coverage/exempt.rs:15`), and it is strictly narrower than the blind
spot #778 removed.

### D4 — Macro bodies are always flagged

`walk_macro_tokens` (`xtask/src/steps/ident_gate.rs:311-325`) sees a flat token
stream. It does **not** read qualifiers; every `from_trusted` in a macro body
stays in the population and needs a marker.

Under D1 this is fail-closed, not a hole. It is also today's behaviour — pinned
by the existing `a_bare_from_trusted_inside_a_macro_body_is_flagged` test — and
it affects zero current sites, since every `ContentType::from_trusted` in the
repo is plain code. The token-index seam the module doc describes stays
available if a real site ever appears.

### D5 — Qualifier resolution is opt-in per gate

`ident_gate::Gate` (`xtask/src/steps/ident_gate.rs:415-433`) gains one optional
field naming the owning type. When it is absent the engine behaves exactly as
today.

This is required, not stylistic: the feature is meaningless for the two sibling
gates, whose behaviour must not change (they each name the new field once and
nothing more — see AC9). `raw-html-door` polices `PreEscaped`, which **is** the
type and appears as a path head; `html-sink` polices `inner_html` (a Leptos
macro attribute, never qualified) and `set_inner_html` (a `web_sys` method
reached through `.` on a runtime receiver, not a path qualifier).

### D6 — A new ADR records the principle

Identifying a gate's population correctly is **structural**; exempting a site
from it needs a human marker. #778 conflated the two — it deleted the qualifier
logic as a "pattern-decided exemption" (ADR-0085 principle 3) when the qualifier
is part of _identifying the door_, not of exempting anything from it.

A numberless draft goes in `docs/adr/drafts/`, numbered at ship by
`cargo xtask adr promote`. It states: population membership is structural, and a
gate must fail closed when it cannot determine membership — which is what makes
reading the qualifier legitimate rather than a self-exemption.

_Rejected:_ amending ADR-0085 in place (this is a decision layered on top of its
principles, not a correction to their text) and code-comments-only (the next
person to see a qualifier check in a gate would re-derive the question #778 got
wrong).

## Acceptance criteria

Unlike a Leptos component, this is **host-compiled and unit-testable**:
`ident_gate` has a fixture-source harness (`scan`/`classify`, and
`Gate::violations(src)` under `#[cfg(test)]`). Every behavioural criterion below
is a real test, not an eyeball.

**Engine — `xtask/src/steps/ident_gate.rs`**

- **AC1** `Gate` carries an optional owner-type field. With it absent,
  `scan`/`classify` behaviour is byte-identical to today — pinned by the
  existing shared `marker_tests` continuing to pass unchanged.
- **AC1a** **Exactly one hook records a site.** `visit_ident` stays the sole
  recorder, so no form it catches today is lost: `fn` definition idents (a `fn`
  ident is not a `syn::Path`), method-call idents, and macro tokens. Qualifier
  resolution only ever _suppresses_ — it never records — so a site can never be
  counted twice and `Why::Shared` cannot fire spuriously.
- **AC2** Fixture tests, with a synthetic owner and synthetic population, cover
  every row of D1's table: owner-qualified, `Self` in the owner's `impl`,
  owner's definition site, other-type-qualified, other type's definition site,
  unresolvable qualifier, unqualified call, and free module-scope definition.
- **AC3** Fixture tests cover each D3 resolution source: owner set, non-glob
  `use` binding, in-file type definition, in-file `type` alias, and `Self`.
- **AC4** A fixture with `use foo::*;` and an otherwise-unbound qualifier is
  **flagged**.
- **AC5** A fixture with `use path::Owner as Alias;` and `Alias::from_trusted`
  is **in the population** (the #778 hole, now closed by resolution rather than
  by over-approximation).
- **AC6** A fixture placing an other-type door inside a macro body is
  **flagged** (D4).

**Owner-set harvest**

- **AC7** The harvest is a pure function over `(path, source)` pairs,
  unit-tested: it collects `use …::Owner as X`, ignores unrelated `use` items,
  and is order-independent.
- **AC8** A two-file fixture — `pub use …::Owner as Doc;` in one,
  `Doc::from_trusted(…)` in the other — puts the second site **in the
  population**. This is the D2 coverage claim, and it must fail if the harvest
  is removed.

**Gate wiring — `xtask/src/steps/rendered_html_from_trusted_check.rs`**

- **AC9** The gate declares `RenderedHtml` as its owner.
  `raw_html_door_check.rs` and `html_sink_check.rs` gain **`owner: None` plus a
  short comment saying why the feature is meaningless for them, and nothing
  else** — additions only, no deletions, no behavioural change. `Gate` is a
  `const` struct literal in both with no `Default`, so a new field is a compile
  error until they name it; "untouched" was never achievable. Behaviour being
  unchanged is the property that matters, and their own tests pin it.
- **AC10** The three fixtures that currently assert a `ContentType` door needs a
  marker (`:194`, `:200`, `:206`) are inverted or re-pointed: an other-type door
  is now clean with no marker. The `Widget::from_trusted` case (`:206`) becomes
  "resolves to another type → ignored", and a new fixture keeps the
  "unresolvable → flagged" coverage it used to provide.
- **AC11** The `Report` prose names `RenderedHtml` rather than hedging about the
  bare ident, and `recovery` no longer carries the "a `from_trusted` on a
  different type (`ContentType`, #584) is not this door at all" sentence. The
  verdict test at `:397` is updated to match, and still asserts the verdict
  claims nothing false.

**Production code — the payoff**

- **AC12** All four markers are gone: `common/src/media.rs:872`, `:968`, `:972`,
  and `common/src/feed/feed_path.rs:97`. **Both** `common/src/render.rs` markers
  — `:111` on the owner's own definition and the second at `:156` — **remain**,
  and the gate pins them: were resolution to stop reaching either, it would fail
  as `Unmarked` rather than pass quietly.
- **AC13** The paragraph in `ContentType`'s doc comment that exists to explain
  the collision (`common/src/media.rs`, the parenthetical about
  `#398`/`#778`/`#790`) is deleted. The rest of that doc — including the "grep
  `ContentType::from_trusted` to enumerate every mint site" instruction — is
  rewritten to be true without the gate enforcing it.
- **AC14** Doc prose naming the collision is updated: `common/src/media.rs:939`
  and `common/src/feed/feed_path.rs:87`.
- **AC15** `ContentType::from_trusted` and `RenderedHtml::from_trusted` both
  keep their names. No production API is renamed.
- **AC16** `cargo xtask check` passes, and the gate reports **zero** violations
  on the tree with the four markers removed — i.e. removing them is not merely
  tolerated but correct. Removing a marker that is still required fails as
  `Unmarked`; leaving a marker whose site is gone fails as an orphan, so AC12
  and AC16 pin each other.

**Documentation**

- **AC17** A numberless ADR draft exists in `docs/adr/drafts/` stating D6's
  principle.
- **AC18** Every sentence the change makes stale is fixed. There are six sites,
  not two:
  - ADR-0079 §89 — "the `from_trusted` ident wherever it appears (#778 widened
    it to definitions and to other types' doors)";
  - `common/src/render.rs:216-218` — the **same sentence, verbatim**, in code;
  - ADR-0094 §229 (the note that `ident_gate` lost the free `ContentType`
    coverage) and §122-127 (that the affected sites "take ordinary markers like
    anything else", which is what turns the doc-comment instruction into
    something "the gate enforces" — the exact claim AC13 walks back);
  - `xtask/src/steps/ident_gate.rs` module doc §29-33, §50-52 (unreadable class
    1: "A `use … as` rename … evades ident matching — `syn` has no name
    resolution", now false for an owner-configured gate) and §69-73;
  - `rendered_html_from_trusted_check.rs` module doc §21-37 and §55-59, which
    says "#790 tracks removing the collision at its source instead".
- **AC19** The accepted residual blind spot (D3, rename-of-a-rename) is
  documented where a reader will meet it — the gate's module doc — and
  classified alongside the existing acknowledged evasions.

## Verification

- `cargo nextest run -p xtask` for the engine and gate unit tests.
- `cargo xtask check` for the whole static ladder, including this gate running
  over the real tree with the markers removed (AC16).
- No e2e involvement: this changes xtask and `common` doc comments only. No
  runtime behaviour changes, so `validate --no-e2e` is the meaningful gate — but
  the branch touches `common/`, so full `validate` still runs before the PR.

## Out of scope

- Reading qualifiers inside macro bodies (D4).
- Adding qualifier resolution to `raw-html-door` or `html-sink` (D5 —
  meaningless for both).
- Renaming either door (D1, AC15).
- Resolving a rename-of-a-rename chain (D3).
