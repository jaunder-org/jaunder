# Thin-component enforcement (#306)

**Goal:** make "a `#[component]` body is thin" an enforced invariant instead of
an assumption, and remediate every component that violates it — pushing setup
logic into host-tested functions and branchy markup into subcomponents. (The
estimate below says 19; it is an under-count, and the counter's own report is
the authoritative list.)

## Problem

`docs/adr/0050` reasons about coverage on the assumption that `#[component]`
bodies are _thin_: their `view!` trees only render in a browser, and any real
logic lives in host-testable functions that `nextest` exercises. Nothing
enforces that assumption.

The invisibility is **structural, not an exemption**. Every component lives in a
wasm-only `component.rs` behind `#[cfg(target_arch = "wasm32")] mod component;`
(ADR-0070), so component lines are never compiled for the host and never enter
the coverage denominator at all. That is a _better_ exemption than a marker —
but a _worse_ blind spot: logic that drifts into a component body is
simultaneously unmeasured and unasserted, and no gate can notice.

### The issue's stated premise is stale — deliberately superseded here

#306 says ADR-0050 "exempts `#[component]` bodies from the coverage gate" and
proposes extending "the `syn`-based exemption parser" in
`xtask/src/coverage/exempt.rs`. **That exemption was retired in #520** (ADR-0050
Decision 1); `exempt.rs` documents the retirement and
`does_not_exempt_component_or_client_only_marked_fns` pins it so it cannot
return. There is no `#[component]` parser to extend. The concern survives —
sharper than when filed — so this spec keeps the _goal_ and replaces the
_mechanism_: a standalone gate, sibling to `target-arch-placement`, not a
coverage-exemption change. Nothing in `exempt.rs` or the coverage gate is
touched.

## Measured baseline (an estimate, not the counter's answer)

**These figures come from a regex approximation, not from D3a's AST+token
counter, which does not exist yet.** They are accurate enough to choose a budget
and size the work, and they are wrong in known directions: the regex counts
`if`/`for` occurring inside string literals and comments, and cannot distinguish
`?` in a path or a `Result` chain from a genuine `ExprTry`. Expect the real
violator list to shift by a few components in either direction. The plan's
**first** task is therefore to build the counter and re-measure — the
authoritative list is whatever it reports, and AC8 ("all components pass") is
the criterion that does not depend on this estimate.

58 components across `web/src`. Counting raw Rust control flow (`if` / `match` /
`for` / `while` / `?`) on two surfaces — **setup** (the body outside the `view!`
macro) and **view** (inside it):

| budget | setup fails | view fails | fails either axis |
| ------ | ----------- | ---------- | ----------------- |
| 2      | 11          | 13         | **19**            |
| 3      | 8           | 3          | 13                |
| 5      | 5           | 2          | 9                 |

39 of 58 are already thin on both axes at budget 2. Violations cluster in
`posts/` (`EditPostPage` 286 lines, `UserTagPage` setup 10, `PostCard` setup 8,
`UserTimelinePage` setup 7, `SiteTagPage` setup 7), with `cockpit/`, `home/`,
`media/`, `audiences/`, `profile/` behind it.

## Decisions

- **D1 — A standalone `thin-components` gate step.** New module, mirroring
  `xtask/src/steps/target_arch_placement_check.rs` and its `syn`-AST siblings.
  Not an `exempt.rs` change (see "stale premise"). Runs in both `check` and
  `validate`.
- **D2 — Two surfaces, two budgets, both 2.** `setup` = the component body
  outside the `view!` macro; `view` = inside it. Each may carry at most **2**
  units of raw control flow. Both are reported independently, because the two
  failures have different remedies (D4).
- **D3 — Count raw Rust control flow only: `if`, `match`, `for`, `while`, `?`,
  `let … else`, and a guarded `match` arm.** One unit per construct, not per
  line and not per unguarded `match` arm. `else if` nests, so it counts as two.

  **Guarded arms added mid-implementation.** A guarded arm
  (`Ok(list) if list.is_empty() => …`) counts one unit on top of its `match`.
  `syn` models the guard as `Arm.guard`, not as an `ExprIf`, so without this a
  single `match` could carry unlimited invisible branching — the same class of
  hiding place as `let … else`. Setup surface only, and deliberately: on the
  view surface the guard's `if` arrives as a plain `Ident` the token counter
  already sees. Found while remediating `DraftsPage`, when rewriting a guarded
  arm as a nested `if/else` _changed the score_ — which is only possible if the
  guard form had been hiding a branch.

  `let … else` is **not** an `Expr` node — `syn` models it as a `Local` whose
  `LocalInit` carries a `diverge` arm — so a visitor listing only
  `ExprIf`/`ExprMatch`/`ExprForLoop`/`ExprWhile`/`ExprTry` scores it **zero**.
  It must count: it is a real branch with an early return, and it is this
  codebase's dominant idiom for exactly the param-parsing logic that should
  leave a component. There are 10 today — 9 in `posts/component.rs` (`:941`,
  `:951`, `:957`, `:1094`, `:1593`, `:1635`, `:1712`, `:1756`, `:1759`) and one
  at `media/component.rs:56`, with three consecutive in `UserTagPage`. **The
  regex estimate below also missed them, so the real violator list is larger
  than the figure it reports, not smaller.**

  Leptos's _declarative_ constructs — `<Show>`, `<For>`, and child components —
  are **not** counted. This is deliberate incentive design: the cheapest way to
  satisfy the gate is to reach for `<Show>`/`<For>` or to extract a
  subcomponent, both of which are improvements. A guard that counted `<Show>`
  would penalise idiomatic Leptos and push authors toward hand-rolled
  `move || if` instead.

- **D3a — The two surfaces need two mechanisms, because `syn` cannot see inside
  a macro.** A `view!` invocation's contents are an opaque `TokenStream`; there
  are no `ExprIf`/`ExprMatch` nodes in there to visit. So:
  - **setup** is counted over the **AST** — visit the component body's
    expressions, skipping any `Macro` node, and count `ExprIf` / `ExprMatch` /
    `ExprForLoop` / `ExprWhile` / `ExprTry`. A skipped macro's tokens are not
    discarded: a **`view!`** hands them to the view surface, and **any other
    macro** hands them to the setup surface. Added mid-implementation, because
    "skip the macro node" alone would let control flow inside a `format!`
    argument escape _both_ surfaces — and when it is setup logic, the setup
    remedy is the advice that fits it.
  - **view** is counted over the macro's **token stream** — walk it recursively
    and count the `Ident` tokens `if`, `match`, `for`, `while` and the `?`
    `Punct`. Token-level counting is _why_ `<Show>` is free: it arrives as
    `Punct('<') Ident(Show) …`, which matches nothing in the count set. String
    and char literals are single `Literal` tokens, so the word "if" inside a
    label never counts — a property the regex estimate below does **not** have.
- **D3b — An HTML attribute named like a keyword must not count.** Rust's lexer
  has no keyword/identifier distinction inside a macro's token stream, so
  `<label for="edit-slug">` tokenizes as
  `… Ident(label) Ident(for) Punct('=') …` — indistinguishable from a `for` loop
  under D3a alone. This is **not hypothetical**: there are 10 such attributes
  today, 8 of them in `posts/component.rs` alone (`:360`, `:420`, `:638`,
  `:664`, `:698`, `:1276`, `:1303`, `:1319`), plus `sessions/component.rs:85`
  and `profile/component.rs:153`. Left uncorrected, `EditPostPage` would collect
  three phantom view units from its `for=` labels and sit over budget with **no
  control flow to extract** — an unfixable failure, which is the worst kind of
  false positive a gate can have.

  **Rule:** a candidate `Ident` does not count when the **next** token is
  `Punct('=')`. That is the attribute-assignment position, which no Rust
  control-flow keyword can occupy (`for x = …` is not valid; `for` is always
  followed by a pattern). The value may be a string _or_ an expression —
  `for=input_id.clone()` at `posts/component.rs:420` is the real second shape —
  and the rule covers both, because it keys on the `=` immediately after the
  name, not on what follows it.

- **D3c — Hook every macro position, not just the tail expression.** `syn`
  models a bare `view! { … };` statement as `Stmt::Macro`, which is a
  _different_ node from the `Expr::Macro` a tail-position `view!` produces. A
  counter that only intercepts `Expr::Macro` would let a statement-position
  macro's tokens escape **both** surfaces — invisible to the AST walk (opaque)
  and never handed to the token counter. That is a silent under-count, the
  direction D6 exists to prevent. Intercept `Expr::Macro`, `Stmt::Macro`, and
  `Item::Macro`. Every component in the tree today uses the tail-expression
  form, so this is robustness rather than a present bug. A component with no
  `view!` has a `view` count of 0; a `view!` in a `const` or helper outside a
  `#[component]` is not measured at all.
- **D4 — Remediation is two-shaped, and the failure message says which.** Setup
  complexity → extract to a pure function in the vertical's host-compiled
  module, with `nextest` assertions. View complexity → extract a
  **subcomponent**. Subcomponent decomposition is a first-class remedy, not a
  fallback: it has repeatedly turned untestable markup into testable units in
  this codebase.
- **D5 — Enumerate by parsing source, not by compiling.** The gate `syn`-parses
  files; `cfg(target_arch)` gating is irrelevant to it, which is exactly why it
  can see what coverage cannot. Scope: every `#[component]` under `web/src`,
  wherever it sits. (An earlier draft said `posts/mod.rs` and `media/mod.rs`
  "also hold some" — wrong: those files only _mention_ `#[component]` in prose.
  Every real component is in a `component.rs`. The gate still scans the whole
  tree rather than assuming that, so a component added elsewhere is caught
  rather than skipped.)
- **D6 — Fail-closed on parse failure.** An unparseable file **fails** the gate
  rather than passing silently. This is the opposite of `exempt.rs`'s
  fail-closed direction, and for the same reason: there, "recognise nothing"
  leaves lines measured (safe); here, "recognise nothing" would let a fat
  component through. Safety points at _failing_ in both cases.
- **D7 — The gate ships proven to bite.** A fat fixture must make it fail and a
  thin fixture must make it pass, as unit tests — plus one observed real run. A
  gate never seen red is not known to work.
- **D8 — Timeline pages get minimal extraction only.** #671 owns "thin the three
  timeline pages onto a shared `TimelineGate`". This issue extracts only as much
  as the budget requires and does **not** invent that abstraction.

  **Superseded by events: #671 landed first.** This issue was blocked behind it
  and resumed afterwards, so `TimelineGate` and a host-tested `TimelineState`
  already exist on `main` and are consumed at `posts/component.rs` and
  `cockpit/component.rs`. #671 also cleared every timeline _view_ violation and
  `HomePage` outright, and cut the tag/timeline setup counts (`UserTagPage` 8→6,
  the others 6→4). What remained here was folding those pages' residual setup
  into `posts/page_state.rs`. The carve-out is therefore moot rather than
  observed — see AC15.

- **D9 — Document the invariant.** A numberless ADR draft in `docs/adr/drafts/`
  (numbered at ship by `cargo xtask adr promote`) recording the enforced
  thin-component invariant and its relationship to ADR-0050's assumption, plus a
  `CONTRIBUTING.md` verify-ladder entry naming the step and both budgets.
- **D10 — No new CLI surface.** `doc-links`-style: a step name inside
  `check`/`validate`, not a `cargo xtask` subcommand.
- **D11 — No suppression escape hatch in v1.** No `thin:allow`. If a component
  genuinely needs more than the budget, that is a design conversation, and
  adding an opt-out before we know of a real case invites it to become the
  default. Revisit only when a concrete case appears. (Consistent with
  `CONTRIBUTING.md`'s fix-don't-silence rule, which requires explicit approval
  for any suppression.)

## Acceptance criteria

- **AC1** — The gate fails on a fixture component whose _setup_ exceeds the
  budget, naming the component, the file, the surface (`setup`), and the count.
- **AC2** — The gate fails on a fixture whose _view_ exceeds the budget, naming
  the surface as `view`.
- **AC3** — The gate passes a thin fixture (setup ≤ 2, view ≤ 2).
- **AC4** — `<Show>`, `<For>`, and child components do not count toward either
  budget (a fixture using them heavily, with no raw control flow, passes).
- **AC5** — `?`, `if`, `match`, `for`, `while`, `let … else`, and a guarded
  `match` arm each count; a fixture isolating each one proves it, asserting the
  **count value**, not merely that some violation was produced (an assertion
  that only checks "one violation exists" passes against any over- or
  under-count that still exceeds the budget).
- **AC6** — An unparseable file fails the gate (D6), with the parse error in the
  detail.
- **AC7** — Counting is per construct, not per line or per `match` arm: a 10-arm
  multi-line `match` is 1 unit, and `else if` is 2 (it nests).
- **AC7a** — A `view!` containing the word "if" only inside a string or char
  literal scores 0 on the view surface (D3a's token-level property, which the
  estimating regex lacks).
- **AC7b** — A `view!` whose only keyword-like tokens are HTML attribute names
  scores 0: `<label for="a">`, `<label for=id.clone()>`, and three of them
  together all pass (**D3b**). Asserted against the real `posts/component.rs`
  shapes, not a synthetic one.
- **AC7c** — A statement-position `view! { … };` is counted on the **view**
  surface, not silently dropped (**D3c**).
- **AC8** — **Every** component in `web/src` passes both budgets at the end of
  the branch — the gate reports zero over-budget surfaces tree-wide. Stated as
  "all 58" when written; the tree now holds 68, because #671 landed nine
  components and this branch added nine subcomponents. The criterion is "the
  report is empty", which does not depend on the count.
- **AC9** — Every extraction made for AC8 is covered by `nextest` assertions on
  the extracted function, or is a subcomponent whose own body is within budget,
  **or is irreducible browser wiring**: a named wasm-only helper whose branch is
  inside an `Effect::new` or `spawn_local`, which ADR-0083 §1 grants as
  permanently un-host-testable.

  **The third category was added mid-implementation, and it is narrow on
  purpose.** "Not host-tested" is not the qualifier — `Effect`/`spawn_local` is.
  A plain wasm-only fn holding an ordinary branch does **not** qualify merely by
  living in `component.rs`: that is the escape hatch this issue exists to close,
  and review caught exactly one instance (`notify`) where this spec's own
  allowance had been stretched to cover an `Option<Callback>` branch that the
  same branch host-tested elsewhere as `UploadCallbacks::notify`.

  The allowance never covers a fn that **returns markup**. The gate measures
  only `#[component]` bodies, so a view-returning plain fn hides a branch
  instead of moving it somewhere checked; markup extractions are `#[component]`
  subcomponents, which the gate then measures like any other.

- **AC10** — `cargo xtask check` and `cargo xtask validate` both report the
  step.
- **AC11** — No new `cargo xtask` subcommand (`--help` unchanged).
- **AC12** — `exempt.rs` and the coverage gate are untouched (`git diff` shows
  no change under `xtask/src/coverage/`).
- **AC13** — The gate is observed failing on the real tree (temporarily fatten a
  component), then restored green.
- **AC14** — `CONTRIBUTING.md` documents the step and both budgets; an ADR draft
  records the invariant.
- **AC15** — ~~The timeline pages are within budget without a `TimelineGate`
  abstraction (D8).~~ **Void, not met: #671 landed first**, so `TimelineGate`
  exists on `main` and these pages consume it. The criterion was written to stop
  this issue pre-empting that design; sequencing achieved the same end, and
  asserting its absence would now fail against `main`'s own code. The surviving
  obligation is D8's substance — this branch invented no timeline abstraction of
  its own, which `git diff main...HEAD -- web/src/timeline/` (empty) shows.

## Scope

**In:** the `thin-components` gate step and its tests; extraction/decomposition
of every violating component; the ADR draft and `CONTRIBUTING.md` entry.

The estimate below says "19 components"; the counter's own report found **19
findings across 15 components** — some over budget on both surfaces — and #671
then changed the composition again before the work resumed. The scope is
whatever the report lists, which is why AC8 is phrased as "the report is empty".

**Out:** `exempt.rs` and the coverage gate (D1/AC12); the #671 `TimelineGate`
design (D8); #301's lint-suppression work (`needless_pass_by_value`, display
casts) even where it touches the same files; a suppression escape hatch (D11);
any wasm instrumentation — every assertion added here runs on the host.

**Risk — file contention.** `posts/component.rs` carries the heaviest
remediation and is also touched by #569 (post DTO renames). #671 touches the
timeline pages. Neither is claimed right now; if either starts mid-cycle, the
later one rebases.

## Refs

- ADR-0050 (stateless coverage gate; its thinness assumption is what this
  enforces)
- ADR-0070 (four-file vertical layout; wasm-only `component.rs`)
- #520 (retired the `#[component]` exemption — why the premise moved)
- `xtask/src/steps/target_arch_placement_check.rs` (the sibling gate to mirror)
- #671 (timeline thinning), #301 (lint suppressions), #569 (post DTO renames)
