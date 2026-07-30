# ADR-0083: Reactive components paint from a host-tested decision fold

- Status: accepted
- Date: 2026-07-30
- Issue: [#671](https://github.com/jaunder-org/jaunder/issues/671)

## Context

[ADR-0070](0070-web-vertical-wasm-only-component-files.md) §6 carries forward
ADR-0055's principle that pure, host-testable logic lives in **ungated,
host-tested, coverage-measured** files, extracted _before_ a gate goes on. #520
then retired the `#[component]` coverage exemption outright: a wasm-only
component never host-compiles, so nothing inside one is measured at all. #306
will _fail_ the gate on component bodies that exceed a complexity budget.

Between them these say a component must be thin, and that logic belongs outside
it — but none of them says how to extract the one thing components are actually
made of: the **render decision**. "Which of N states do I paint?" is not
validation, formatting, or a slug rule; it is a fold over reactive state, and
the obvious home for it is the view closure, which is exactly where the coverage
gate cannot see it.

The timeline vertical showed the cost. Five pages each re-derived the same error
→ loading → rows decision inline, four of them behind
`#[expect(clippy::too_many_lines)]`, and the state transitions those decisions
read (`adopt`/`fail`/the load-more fold) sat in a wasm-only file where no host
test could reach them. The decision and the state machine were both invisible,
and the only thing asserting either was e2e.

## Decision

A reactive widget splits into a **host-compiled state module** and a **wasm-only
view**, and the boundary is drawn so that everything except irreducible browser
wiring falls on the host side.

1. **The signal bundle is host-compiled.** An `RwSignal`-carrying state struct
   lives in an ungated, coverage-measured file and is exercised under a reactive
   `Owner` (`Owner::new()` + `.set()`), the convention `web::reactive`,
   `forms::Field`, and `tags::input_state` already follow. Only `Effect::new`
   and `spawn_local` stay wasm-only — an `Effect` does not run in a host test,
   so reactive _wiring_ is permanently e2e-only, but every transition it
   dispatches to is host-tested.

2. **The render decision is a fold, not a closure.** The state exposes a method
   returning a typed decision — for the timeline,
   `paint(context) -> WebResult<TimelinePaint>`. Failure travels on `Result`'s
   **error axis** rather than as a variant, so the success type keeps naming
   only successes and nothing threads an anonymous `Option<E>` around. The
   component body becomes a `Memo` plus a bare `match`, one arm per variant:
   thin by construction, so #306's guard passes without a special case.

3. **Per-page variation within a shared arm travels as data.** Where callers
   must render one outcome differently, they pass a **data enum**
   (`NoIdentity { Blank, Redirect(&'static str) }`), never a `ViewFn`/closure
   prop. A closure would push the choice back into uncovered per-caller code and
   re-thicken the body; a data value keeps the decision host-testable and the
   match bare.

4. **Chrome that must survive a paint transition is a sibling region.** Caller
   chrome passed as `children` is emitted from its **own** memo-gated region —
   `{move || show_chrome.get().then(|| children())}` — beside the match, never
   `{children}` repeated inside each arm. Repeating it tears the subtree down
   and rebuilds it on every arm change; when that subtree is
   projector-coincident markup (`inner_html` of a shared pure fn,
   [ADR-0041](0041-public-projector-and-csr-client.md) §2) the rebuild is a
   visible first-paint flash. #653 was exactly that regression.

## Consequences

- **Transitions and the render decision become coverage-measured.** They land in
  a measured file, so they must ship with tests rather than inherit a wasm-gate
  exemption. Note derives count as executable lines: a data enum only _matched_
  in wasm needs a host test that constructs and compares it, or the gate fails
  with no marker permitted.
- **Adding an outcome is a compile error everywhere it matters** — a new variant
  breaks every `match`, which is the point.
- **The cost is one enum and one fold per widget**, plus the discipline of
  deciding what "chrome" means for that widget. For a widget with one caller and
  two states this is overhead; the pattern earns its keep where several callers
  share a decision.
- **What it does not buy:** the sibling-region rule (4) is a construction that
  _prevents_ a rebuild, not one that can be observed on every caller. In the
  timeline only `home` has static chrome that spans a `Loading → Rows`
  transition; the cockpit's chrome reads a signal that flips at the same moment
  and is rebuilt regardless. Adopt it because it is the safe shape, and assert
  it where it is observable.
- First instance: `web/src/timeline` (#671), which converged five pages onto one
  gate and dropped three `#[expect(clippy::too_many_lines)]`.
