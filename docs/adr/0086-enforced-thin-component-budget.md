# ADR-0086: Component thinness is enforced, not assumed

- Status: proposed
- Date: 2026-07-30
- Issue: [#306](https://github.com/jaunder-org/jaunder/issues/306)

## Context

[ADR-0050](0050-stateless-coverage-gate.md) reasons about coverage on the
premise that a `#[component]` body is _thin_ — that its `view!` tree only
renders in a browser, and that any real logic lives in host-testable functions
`nextest` exercises. Nothing enforced that premise.

#520 then made the premise load-bearing in a new way. It retired the
`#[component]` coverage exemption outright, because it had become unnecessary:
every component now lives in a wasm-only `component.rs` behind
`#[cfg(target_arch = "wasm32")] mod component;`
([ADR-0070](0070-web-vertical-wasm-only-component-files.md)), so its lines are
never host-compiled and never enter the denominator at all. Not measured, not
exempt. That is a _better_ exemption and a _worse_ blind spot: logic that drifts
into a component body is simultaneously unmeasured and unassertable, and no
coverage gate can notice — a marker at least leaves a trace.

[ADR-0083](0083-reactive-paint-fold.md) says how a reactive component should be
_built_ so it stays thin: paint from a host-tested decision fold. It does not
say what happens to one that isn't.

## Decision

A `thin-components` step in `check`/`validate` **fails** when a `#[component]`
body carries more than **2** units of control flow on either of two surfaces.

1. **Two surfaces, counted by two mechanisms, because `syn` cannot see inside a
   macro.** _Setup_ is the body outside any macro, counted over the AST. _View_
   is a `view!` macro's contents, counted over its token stream. They are
   reported separately because the remedies differ: setup complexity belongs in
   a host-tested function; view complexity wants a subcomponent.

2. **A unit is raw Rust control flow:** `if`, `match`, `for`, `while`, `?`,
   `let … else`, and a guarded `match` arm. One per construct — a 10-arm `match`
   is one, `else if` nests. The last two are named explicitly because neither is
   an expression node: `let … else` is a `Local` with a `diverge` arm and a
   guard is `Arm.guard`, so an expression-only visitor scores both zero, and one
   `match` could otherwise carry unlimited invisible branching.

3. **Leptos's declarative constructs cost nothing.** `<Show>`, `<For>`, and
   child components are free. This is incentive design: the cheapest way to
   satisfy the gate is the idiomatic form or a subcomponent, both improvements.
   A gate that counted `<Show>` would push authors toward hand-rolled
   `move || if`, which is the thing it is trying to discourage.

4. **The remedy must land somewhere still measured.** The gate measures only
   `#[component]` bodies, so extracting a branch into a plain fn that returns
   `impl IntoView` moves it somewhere nothing looks — an escape hatch by
   accident. Markup extractions are `#[component]` subcomponents, which the gate
   then measures like any other. Non-markup extractions go to a host-compiled
   module and ship with assertions.

5. **No suppression mechanism.** There is no `thin:allow`. A component that
   genuinely needs more budget is a design conversation, and an opt-out
   introduced at the first inconvenience becomes the default. Revisit only
   against a concrete case.

## Consequences

- **The invariant ADR-0050 assumes is now checked**, and ADR-0083's pattern has
  a gate behind it: a component that paints from a fold passes by construction.
- **Some branching is irreducible and stays uncovered.** An `Effect` polling a
  `Resource`, a `NodeRef` click, a `web_sys` handle — none run on the host. The
  gate pushes these into named one-line shells, which is thinner and clearer but
  buys no coverage. Enforcement makes bodies thin; it does not make everything
  assertable.
- **The counting rules are lexical, not semantic.** Two consequences are worth
  knowing: an HTML attribute named like a keyword (`<label for="x">`) is an
  `Ident` inside a token stream, so it is skipped when followed by `=`; and a
  component that moves logic into a helper reduces its score without necessarily
  improving testability. The gate measures the shape it can see.
- **Remediating the existing tree took the count from 19 over-budget surfaces to
  zero**, across nine components in six verticals, and removed two
  `#[expect(clippy::too_many_lines)]` suppressions as a side effect.
