# Thin-Component Enforcement Implementation Plan (#306)

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking. **Stage this plan
> file with each task's commit** as you tick its boxes — otherwise the ticks
> accumulate uncommitted and trip `validate`'s clean-tree precheck at ship.

**Goal:** Land a `thin-components` gate that fails when a `#[component]` body
carries more than 2 units of control flow on either of two surfaces, and
remediate every component that violates it — setup logic into host-tested
functions, branchy markup into subcomponents.

**Architecture:** One new `xtask` step module, `steps/thin_components.rs`,
mirroring `steps/target_arch_placement_check.rs`: pure `violations()` /
`problems()` unit-tested directly, a thin `run()` that scans `web/src` and
pushes a `StepResult`. Remediation follows the repo's proven three-tier
decomposition (pure logic → sibling `*_logic.rs`; signal logic → `Copy` state
struct host-tested under an `Owner`; irreducible `web_sys`/`Effect` glue →
one-line shells in the view).

**Tech Stack:** Rust, `xtask` host workspace
(`--manifest-path xtask/Cargo.toml`), `syn` + `proc-macro2` (both already in
`xtask/Cargo.toml`); `web` crate with `nextest` host tests.

**Spec:**
[2026-07-29-issue-306-thin-component-guard.md](../specs/2026-07-29-issue-306-thin-component-guard.md)
— the "what" and "why". This plan is the "how"; decisions are cited as
**D1**–**D11** and criteria as **AC1**–**AC15** rather than restated.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Every commit must be green.** `.githooks/pre-commit` runs the full
  `cargo xtask check`. Run it yourself first (**`jaunder-commit`**).
- **`xtask`: a new `pub` item in the private `mod steps { … }` is dead-code
  under `-D warnings` until a NON-test caller exists** (`#[cfg(test)]` use does
  not count). This is why Task 1 lands the counter **and** its `run()` **and**
  both `lib.rs` call sites together.
- **Extracted helpers consumed only by the wasm `component` look like
  `dead_code` on the host lib build** — make them `pub` and re-export at the
  vertical's `mod.rs` (the `posts/parse` precedent).
- **wasm-clippy — use the gate's own arguments, verbatim.** Copied from
  `xtask/src/steps/static_checks.rs:76-99`; a paraphrase omits `client`/`csr`
  and the two temporarily-allowed lints, giving either spurious failures or
  false confidence:

  ```bash
  cargo clippy -p web -p client -p csr --features csr \
    --target wasm32-unknown-unknown -- \
    -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations
  ```

- **CRAP threshold is T=30**; at zero coverage a fn needs cyclomatic ≤ 5. A
  6-way match extracted from a component **must** be covered, not merely moved.
- **`xtask` is a separate workspace.** Test with
  `cargo test --manifest-path xtask/Cargo.toml`, never a bare workspace
  `nextest`.
- **No editing while a gated commit is in flight** — serialize edit → gate →
  commit.
- **`cargo xtask check` auto-fixes formatting.** Re-check
  `git status --porcelain` after a green run and stage anything rustfmt/prettier
  rewrote.

---

## Review header

**Scope — in:**

- The `thin-components` step: AST setup counting + token-stream view counting
  (**D2**, **D3**, **D3a**, **D3b**, **D3c**), wired into `check` and `validate`
  (**D1**).
- Remediation of every violating component, three-tier (**D4**).
- ADR draft + `CONTRIBUTING.md` ladder entry (**D9**).

**Scope — out:** `exempt.rs` and the coverage gate (**D1**, **AC12**); #671's
`TimelineGate` design (**D8**); #301's lint suppressions; a `thin:allow` escape
hatch (**D11**); any wasm instrumentation.

**No separable concerns to file.** The overlaps this touches (#671 timeline,
#301 suppressions, #569 post DTOs) are already separate issues; this plan defers
to them.

**Tasks:**

1. Land the counter + the step in **report-only** mode; it prints the
   authoritative violator list without failing.
2. Remediate `posts/` — the heaviest vertical.
3. Remediate `media/`, `cockpit/`, `home/`.
4. Remediate the remaining verticals; the report must reach zero.
5. Flip the step to **enforcing**, and prove it bites.
6. Document: ADR draft + `CONTRIBUTING.md`.

**Key risks / decisions:**

- **Report-only first is what makes this orderable.** Three constraints bind: a
  gate cannot land red (the hook refuses the commit); the counter cannot land
  without a non-test caller (dead-code); and it cannot sit uncommitted either,
  for the same reason. Landing the step **wired but non-failing** satisfies all
  three at once — a real caller exists from commit 1, every commit is green, and
  the counter's own output becomes the authoritative violator list. An earlier
  draft of this plan instead used an out-of-tree prototype to get that list;
  that was unnecessary ceremony and carried a real prototype-vs-implementation
  divergence risk. The report-only state exists only inside this branch — `main`
  never sees a non-gating gate, because the branch merges atomically.
- **The spec's 19-component figure is a regex estimate and is an UNDER-count.**
  It missed `let … else` entirely (**D3**), which is this codebase's dominant
  early-return idiom — 9 occurrences in `posts/component.rs` alone. Task 1's
  report replaces the estimate; Tasks 2–4 name _verticals_, not a frozen
  component list.
- **Three counting traps, each with a fixture (Task 1 Step 1):** `let … else` is
  a `Local`, not an `Expr` (**D3**); `<label for="x">` tokenizes as `Ident(for)`
  and must not count (**D3b**); a statement-position `view!` is `Stmt::Macro`,
  not `Expr::Macro`, and must not escape both surfaces (**D3c**).
- **`EditPostPage` needs three independent moves, not one.** Its excess is
  spread across one outer `view!`: a
  `Suspend::new(async move { match post.await {…} })`, an `is_published`
  if/else, and a separate bottom-level
  `{move || …map(|r| match r { Ok(u) if … => … })}`. All tokens in that one
  macro count together (**D3a**), so `<Show>`-ing the `is_published` branch
  alone still leaves two. Budget every component individually against the
  report, not by eye.
- **`MediaPage` lands exactly at budget** with ~3 subcomponent extractions — no
  margin. Expect to iterate against the report rather than one-shot it.
- **File contention.** `posts/component.rs` (Task 2) is also #569's territory
  and the timeline pages are #671's. Neither is claimed; if either starts, the
  later rebases.

---

## Task 1: The counter and the step, report-only

Counter, `run()`, and **both** `lib.rs` call sites in one commit (Global
Constraints). `run()` pushes `StepResult::ok(...)` carrying the violator list as
detail — so the step is a real, non-test consumer from the first commit while
the tree is still red on content. Task 5 flips it to fail.

**Files:**

- Create: `xtask/src/steps/thin_components.rs` (counter + `run()` +
  `#[cfg(test)]`)
- Modify: `xtask/src/lib.rs` — add `pub mod thin_components;` to the `steps`
  block (`:19-35`); call `steps::thin_components::run(&mut result);` after
  `steps::target_arch_placement_check::run(&mut result);` in **both** the
  `Check` (`:308`) and `Validate` (`:343`) arms
- Test: in-file `#[cfg(test)]` (the `xtask` convention)

**Interfaces:**

- Consumes: `syn`, `proc-macro2`, `crate::result::{CommandResult, StepResult}`.
- Produces:

```rust
/// Per-surface control-flow budget for a `#[component]` body (D2). Both surfaces
/// share one number; they are reported separately because the remedies differ (D4).
const BUDGET: u32 = 2;

/// Which surface a count came from — it selects the remedy named in the failure, so
/// it is part of the message, not a detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Setup,
    View,
}

/// One over-budget component.
pub struct Violation {
    pub component: String,
    pub line: u32,
    pub surface: Surface,
    pub count: u32,
}

/// Every over-budget `#[component]` in `src`. `Err` if the file cannot be parsed —
/// the caller reports that as a failure, never a skip (D6).
pub fn violations(src: &str) -> syn::Result<Vec<Violation>>;

/// Every component's counts, over budget or not — the report-only detail and the
/// authoritative violator list for Tasks 2-4.
pub fn report(scanned: &[(String, String)]) -> String;

/// The failure detail across `(path, source)` pairs, or `None` when clean. Pure, so
/// it is unit-tested directly — the `target_arch_placement_check` shape.
pub fn problems(scanned: &[(String, String)]) -> Option<String>;

/// Scan `web/src` and push the `thin-components` step.
pub fn run(result: &mut CommandResult);
```

- [ ] **Step 1: Write the failing tests**

Every AC gets one. The three trap fixtures (`let … else`, `for=`,
statement-position `view!`) are the ones most likely to be wrong, so they are
written from the **real** shapes in `posts/component.rs`, not invented:

```rust
// --- setup surface (AC1) ---
#[test] fn setup_over_budget_is_flagged() {
    let src = "#[component]\nfn Fat() -> impl IntoView {\n\
               let a = if p { 1 } else { 2 };\n\
               let b = match q { _ => 0 };\n\
               for _ in v {}\n\
               view! { <p></p> }\n}\n";
    let v = violations(src).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].component, "Fat");
    assert_eq!(v[0].surface, Surface::Setup);
    assert_eq!(v[0].count, 3);
}
// --- view surface (AC2) ---
#[test] fn view_over_budget_is_flagged_as_view() {
    let src = "#[component]\nfn V() -> impl IntoView {\n\
               view! { {move || if a {1} else {2}} {move || if b {1} else {2}}\n\
                       {move || match c { _ => 0 }} }\n}\n";
    let v = violations(src).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].surface, Surface::View);
    assert_eq!(v[0].count, 3);
}
// --- thin passes (AC3) ---
#[test] fn thin_component_passes() {
    let src = "#[component]\nfn Thin() -> impl IntoView {\n\
               let n = count.get();\n view! { <p>{n}</p> }\n}\n";
    assert!(violations(src).unwrap().is_empty());
}
// --- Leptos declaratives are free (AC4) ---
#[test] fn show_for_and_child_components_are_free() {
    let src = "#[component]\nfn D() -> impl IntoView {\n\
               view! { <Show when=move || r><For each=xs key=k let:x><Row x=x/></For></Show>\n\
                       <Show when=move || s><Other/></Show><Show when=move || t><Third/></Show> }\n}\n";
    assert!(violations(src).unwrap().is_empty(), "Show/For/child components must not count");
}
// --- TRAP 1: keyword-named HTML attributes (AC7b, D3b) ---
#[test] fn html_for_attributes_do_not_count() {
    // The real `posts/component.rs` shapes: three `for=` labels, one with an
    // expression value (`:420`). Under a bare Ident match these score 3 and push the
    // component over budget with NO control flow to extract — unfixable by any
    // remediation, which is why this fixture exists.
    let src = "#[component]\nfn Labels() -> impl IntoView {\n\
               view! { <label for=\"edit-slug\">\"S\"</label>\n\
                       <label for=\"edit-summary\">\"U\"</label>\n\
                       <label for=input_id.clone()>\"I\"</label> }\n}\n";
    assert!(violations(src).unwrap().is_empty(), "`for=` is an attribute, not a loop");
}
// --- TRAP 2: let-else is a Local, not an Expr (AC5, D3) ---
#[test] fn let_else_counts() {
    // The real `UserTagPage` shape (`posts/component.rs:941,951,957`) — three
    // consecutive early returns. A visitor listing only Expr nodes scores this 0.
    let src = "#[component]\nfn Tag() -> impl IntoView {\n\
               let Some(username) = username else { return None };\n\
               let Some(date) = date else { return None };\n\
               let Some(slug) = slug else { return None };\n\
               view! { <p></p> }\n}\n";
    let v = violations(src).unwrap();
    assert_eq!(v.len(), 1, "three let-else must exceed budget 2");
    assert_eq!(v[0].surface, Surface::Setup);
    assert_eq!(v[0].count, 3);
}
// --- TRAP 3: statement-position macro (AC7c, D3c) ---
#[test] fn a_statement_position_view_is_counted_on_the_view_surface() {
    // `Stmt::Macro`, not `Expr::Macro`. Hooking only the latter lets these tokens
    // escape BOTH surfaces — a silent under-count.
    let src = "#[component]\nfn S() -> impl IntoView {\n\
               view! { {move || if a {1} else {2}} {move || if b {1} else {2}}\n\
                       {move || if c {1} else {2}} };\n\
               unreachable!(\"shape fixture\")\n}\n";
    let v = violations(src).unwrap();
    assert_eq!(v.len(), 1, "statement-position view! must still be counted");
    assert_eq!(v[0].surface, Surface::View);
}
// --- each construct counts, by VALUE (AC5) ---
#[test] fn every_counted_construct_is_recognized() {
    for (frag, what) in [
        ("let a = if p {1} else {2}; let b = if q {1} else {2}; let c = if r {1} else {2};", "if"),
        ("let a = match p { _ => 1 }; let b = match q { _ => 1 }; let c = match r { _ => 1 };", "match"),
        ("for _ in a {} for _ in b {} for _ in c {}", "for"),
        ("while a {} while b {} while c {}", "while"),
        ("let _ = (f()?, g()?, h()?);", "?"),
        ("let Some(a) = x else { return None }; let Some(b) = y else { return None }; \
          let Some(c) = z else { return None };", "let-else"),
    ] {
        let src = format!("#[component]\nfn C() -> impl IntoView {{\n{frag}\nview! {{ <p></p> }}\n}}\n");
        let v = violations(&src).unwrap();
        assert_eq!(v.len(), 1, "{what} must count");
        // Assert the VALUE: `len()==1` alone passes against any mis-count that still
        // exceeds the budget.
        assert_eq!(v[0].count, 3, "{what} must count exactly 3");
    }
}
// --- else-if nests; a big match is one unit (AC7) ---
#[test] fn else_if_counts_twice_and_a_big_match_counts_once() {
    let nested = "#[component]\nfn C() -> impl IntoView {\n\
                  let a = if p {1} else if q {2} else if r {3} else {4};\n\
                  view! { <p></p> }\n}\n";
    assert_eq!(violations(nested).unwrap()[0].count, 3, "3 nested ifs");
    let big = "#[component]\nfn C() -> impl IntoView {\n\
               let a = match p { 1=>1, 2=>2, 3=>3, 4=>4, 5=>5, _=>0 };\n\
               view! { <p></p> }\n}\n";
    assert!(violations(big).unwrap().is_empty(), "one match is one unit regardless of arms");
}
// --- literals do not count (AC7a) ---
#[test] fn the_word_if_inside_a_string_literal_does_not_count() {
    let src = "#[component]\nfn C() -> impl IntoView {\n\
               view! { <p>\"if if if if\"</p> <p>\"match for while\"</p> }\n}\n";
    assert!(violations(src).unwrap().is_empty(), "literals are one token");
}
// --- only components are measured ---
#[test] fn a_plain_fn_with_control_flow_is_not_measured() {
    let src = "fn helper(p: bool) -> u8 {\n if p {1} else if p {2} else {3}\n}\n";
    assert!(violations(src).unwrap().is_empty(), "only #[component] bodies are measured");
}
// --- parse failure is a hard failure (AC6) ---
#[test] fn unparseable_file_is_an_error_not_a_silent_pass() {
    assert!(violations("fn (").is_err());
}
#[test] fn problems_reports_a_parse_failure() {
    let d = problems(&[("web/src/x.rs".into(), "fn (".into())]).expect("a problem");
    assert!(d.contains("web/src/x.rs") && d.contains("parse"));
}
// --- the message names the remedy (D4) ---
#[test] fn problems_names_file_component_surface_and_remedy() {
    let src = "#[component]\nfn Fat() -> impl IntoView {\n\
               let a = if p {1} else {2};\n let b = if q {1} else {2};\n\
               let c = if r {1} else {2};\n view! { <p></p> }\n}\n";
    let d = problems(&[("web/src/posts/component.rs".into(), src.into())]).expect("a problem");
    assert!(d.contains("web/src/posts/component.rs"));
    assert!(d.contains("Fat"));
    assert!(d.contains("setup"));
    assert!(d.contains("extract"), "the remedy must be in the message: {d}");
}
#[test] fn clean_tree_reports_none() {
    assert_eq!(problems(&[("web/src/a.rs".into(),
        "#[component]\nfn T() -> impl IntoView { view! { <p></p> } }\n".into())]), None);
}
// --- report-only lists every component, passing or not (Task 1) ---
#[test] fn report_lists_counts_for_passing_and_failing_components() {
    let src = "#[component]\nfn Thin() -> impl IntoView { view! { <p></p> } }\n\
               #[component]\nfn Fat() -> impl IntoView {\n\
               let a = if p {1} else {2};\n let b = if q {1} else {2};\n\
               let c = if r {1} else {2};\n view! { <p></p> }\n}\n";
    let r = report(&[("web/src/a.rs".into(), src.into())]);
    assert!(r.contains("Thin"), "passing components appear too: {r}");
    assert!(r.contains("Fat"));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

`cargo test --manifest-path xtask/Cargo.toml thin_components` — FAIL, symbols
undefined.

- [ ] **Step 3: Implement the counter**

Structure mirrors `target_arch_placement_check`. Specifics that the fixtures pin
but the shape does not:

- Find components by walking **all** items (recursing into inline
  `mod x { … }`), not just top-level, and matching an `ItemFn` whose `attrs`
  contain a path `is_ident("component")`. `syn` does not expand macros, so the
  attribute survives.
- **Setup surface:** a `Visit` impl counting `ExprIf`, `ExprMatch`,
  `ExprForLoop`, `ExprWhile`, `ExprTry`, plus a `visit_local` that counts a
  `Local` whose `LocalInit.diverge` is `Some` (**D3** — `let … else`). Override
  `visit_expr_macro`, `visit_stmt_macro`, **and** `visit_item_macro` to **not**
  recurse, handing each macro's `tokens` to the view counter instead (**D3c**).
- **View surface:** a recursive `proc_macro2::TokenStream` walk. Count an
  `Ident` equal to `if`/`match`/`for`/`while`, and a `Punct('?')` — **unless**
  the _next_ token is `Punct('=')`, which is the HTML-attribute position
  (**D3b**). Recurse into `Group`s. Because a nested `view!` is part of the
  outer macro's single token stream, it is walked once and never double-counted.
- Two remedy strings, one per `Surface`, each naming the fix (**D4**):

```rust
const SETUP_REMEDY: &str = "extract this logic into a host-tested function in the \
                            vertical's host-compiled module (#306)";
const VIEW_REMEDY: &str = "extract a subcomponent, or use <Show>/<For> instead of \
                           `move || if` (#306)";
```

- [ ] **Step 4: Run the tests, verify they pass**

`cargo test --manifest-path xtask/Cargo.toml thin_components` — PASS.

- [ ] **Step 5: Wire the step, report-only**

`run()` scans `web/src` (the `rust_files` recursion from
`target_arch_placement_check`) and pushes:

```rust
// Report-only until #306's remediation lands (Task 5 flips this to `problems()`).
// Wired now so the counter has a real non-test caller — see Global Constraints — and
// so its own output is the authoritative violator list for Tasks 2-4.
result.push(StepResult::ok("thin-components").detail(report(&scanned)));
```

An unreadable or unparseable file still **fails** here (**D6**) — report-only
applies to over-budget components, not to "the guard could not look".

- [ ] **Step 6: Capture the authoritative violator list**

```bash
cargo xtask check --no-test
jq -r '.steps[] | select(.name=="thin-components") | .detail' .xtask/last-result.json
```

Record the over-budget components, grouped by vertical, in Tasks 2/3/4's step 1.
Note the delta against the spec's regex estimate — expect it to be **larger**
(the estimate missed `let … else`). Classify each violator's excess by tier:
**T1** pure logic → sibling `*_logic.rs` with plain `#[test]`s; **T2** signal
logic → `#[derive(Clone, Copy)]` state struct host-tested under
`Owner::new()`/`.set()`/`drop`; **T3** irreducible `web_sys`/`Effect` glue →
one-line shell in the view. View-surface excess is usually a **subcomponent**
extraction (**D4**).

- [ ] **Step 7: Commit**

Run `cargo xtask check` first (**`jaunder-commit`**).

```bash
git add xtask/src/steps/thin_components.rs xtask/src/lib.rs docs/superpowers/plans
git commit -m "feat(xtask): measure #[component] complexity (report-only)"
```

---

## Task 2: Remediate `posts/`

The heaviest vertical, and the one whose real counts most exceed the estimate (9
of the 10 `let … else` are here).

**Files:**

- Modify: `web/src/posts/component.rs`, `web/src/posts/mod.rs` (re-exports)
- Create: `web/src/posts/*_logic.rs` and/or a state module, per Task 1 Step 6's
  tiers
- Test: in-file `#[cfg(test)]` in each new module (the `web` convention)

**Interfaces:**

- Consumes: Task 1's report and tier classification.
- Produces: extracted `pub` functions / state structs, re-exported from
  `posts/mod.rs` so the host lib build does not see them as `dead_code`.

- [ ] **Step 1: Record this vertical's violators from Task 1's report**

Paste the report's `posts/` lines here with counts, worst first, before editing
anything — the list is the task's definition of done.

- [ ] **Step 2: Extract setup logic, test-first**

Write the `#[cfg(test)]` assertions for the extracted function _before_ moving
the body, run them red, then move the logic and run them green.

T1 shape:

```rust
// web/src/posts/<name>_logic.rs
/// Pure: no signals, no `web_sys`. Extracted from `<Component>` (#306).
pub fn <name>(…) -> … { … }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn <asserts the branch that used to live in the component>() { … }
}
```

T2 shape (signal logic under a reactive `Owner`):

```rust
#[derive(Clone, Copy)]
pub struct <Name>State { pub field: RwSignal<…>, … }

impl <Name>State {
    pub fn <action>(self, …) -> bool { … }   // returns e.g. prevent_default?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn <action>_transitions() {
        let owner = leptos::prelude::Owner::new();
        owner.set();
        let state = <Name>State { field: RwSignal::new(…), … };
        assert!(state.<action>(…));
        drop(owner);
    }
}
```

The `let … else` clusters (`:941`, `:951`, `:957` in `UserTagPage`; `:1094`;
`:1593`; `:1635`; `:1712`; `:1756`; `:1759`) are the T1 bulk: each is param
parsing that becomes a pure function returning `Option`/`Result`, asserted
directly.

- [ ] **Step 3: Decompose view-surface excess into subcomponents**

Extract a `#[component]` subcomponent rather than moving markup into Rust
(**D4**); each new subcomponent must itself be within both budgets. Prefer
`<Show>`/`<For>` over `move || if` where the branch is presentational — both are
free (**D3**), so this is the cheapest fix.

**`EditPostPage` needs three independent moves** (see Key risks): the
`Suspend::new(async move { match post.await {…} })`, the `is_published` if/else,
and the bottom-level `{move || …map(|r| match r { Ok(u) if … => … })}`. Re-run
the report after each; do not assume one move suffices.

- [ ] **Step 4: Re-export and check visibility**

Add `pub use` lines to `web/src/posts/mod.rs` for every new `pub` item, or the
host lib build reports `dead_code`.

- [ ] **Step 5: Verify**

```bash
cargo nextest run -p web
cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations
cargo xtask check --no-test
```

Expected: tests pass; wasm-clippy clean; the `thin-components` report lists
**no** `posts/` violators.

- [ ] **Step 6: Commit**

```bash
git add web/src/posts docs/superpowers/plans
git commit -m "refactor(web): thin the posts components onto host-tested logic"
```

---

## Task 3: Remediate `media/`, `cockpit/`, `home/`

**Files:**

- Modify: `web/src/{media,cockpit,home}/component.rs` and their `mod.rs`
- Create: sibling logic/state modules per tier
- Test: in-file `#[cfg(test)]`

**Interfaces:** as Task 2, per vertical.

- [ ] **Step 1: Record these verticals' violators from Task 1's report.**
- [ ] **Step 2: Extract setup logic, test-first** — same tier shapes as Task 2.
      Note `media/component.rs:56`'s `let … else` (T1) and `MediaUpload`'s
      `web_sys`/`spawn_local` glue (T3 — leave a one-line shell).
- [ ] **Step 3: Decompose view excess into subcomponents.** `MediaPage` needs ~3
      extractions (one per `Suspense` body) and lands **exactly** at budget —
      re-run the report after each, expect no margin.
- [ ] **Step 4: Re-export from each `mod.rs`.**
- [ ] **Step 5: Verify** — `cargo nextest run -p web`, the verbatim wasm-clippy
      command, `cargo xtask check --no-test` report clean for these three
      verticals.
- [ ] **Step 6: Commit**

```bash
git add web/src/media web/src/cockpit web/src/home docs/superpowers/plans
git commit -m "refactor(web): thin the media, cockpit, and home components"
```

---

## Task 4: Remediate the remaining verticals

Everything Task 1's report still lists — `audiences/`, `profile/`,
`registration/`, `auth/`, `sessions/`, `invites/`, `email/`, and the **timeline
pages at minimal extraction only** (**D8** — get them under budget; do not
invent `TimelineGate`, that is #671's).

**Files:**

- Modify: the listed `component.rs` + `mod.rs` files
- Create: sibling logic/state modules per tier
- Test: in-file `#[cfg(test)]`

**Interfaces:** as Task 2, per vertical.

- [ ] **Step 1: Record the remaining violators from Task 1's report.**
- [ ] **Step 2: Extract setup logic, test-first.**
- [ ] **Step 3: Decompose view excess into subcomponents.**
- [ ] **Step 4: Timeline pages — minimal only.** Record in the commit message
      what was deliberately left to #671, so that issue's scope stays legible.
- [ ] **Step 5: Re-export from each `mod.rs`.**
- [ ] **Step 6: Verify the report is now EMPTY tree-wide.** This is the
      precondition for Task 5 (**AC8**) — flipping to enforcing while any
      violator remains produces a commit the hook will refuse.

```bash
cargo nextest run -p web
cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations
cargo xtask check --no-test
```

- [ ] **Step 7: Commit**

```bash
git add web/src docs/superpowers/plans
git commit -m "refactor(web): thin the remaining components to the budget"
```

---

## Task 5: Flip the step to enforcing, and prove it bites

A small commit: `run()` switches from `report()` to `problems()`. The counter
and its fixtures already exist (Task 1), so this task adds behaviour, not
machinery.

**Files:**

- Modify: `xtask/src/steps/thin_components.rs` (`run()` only)
- Test: the existing `#[cfg(test)]` module already covers `problems()`

**Interfaces:**

- Consumes: `problems()` (Task 1).
- Produces: a failing `thin-components` step when any component is over budget.

- [ ] **Step 1: Flip `run()`**

```rust
let step = match (problems(&scanned), unreadable.is_empty()) {
    (None, true) => StepResult::ok("thin-components"),
    (found, _) => { /* same detail-composition shape as target_arch_placement_check */ }
};
result.push(step);
```

Drop the report-only comment and the now-unused `report()` **only if** nothing
else calls it — if `report()` becomes dead, either keep it as the `ok` detail
(useful: the counts stay visible on a green run) or delete it. Keeping it is
preferred; a passing gate that still shows each component's headroom is how the
next author notices they are at 2.

- [ ] **Step 2: Verify the gate passes**

`cargo xtask check --no-test` → PASS with `[ ok ] thin-components` (**AC8**, and
proof Tasks 2–4 finished).

- [ ] **Step 3: Verify it BITES (AC13)**

Add a third `if` to one real component, re-run, confirm `[FAIL] thin-components`
naming file, component, surface, and count — then restore and re-run green. A
gate never observed failing is not known to bite.

- [ ] **Step 4: Confirm the coverage boundary was not touched (AC12)**

`git diff --stat wt-base-issue-306..HEAD -- xtask/src/coverage/` → **empty**.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/steps/thin_components.rs docs/superpowers/plans
git commit -m "feat(xtask): enforce the thin-component budget"
```

---

## Task 6: Document the invariant

**Files:**

- Modify: `CONTRIBUTING.md` (the verify-ladder bullet list)
- Create: `docs/adr/drafts/enforced-thin-component-invariant.md` (numberless;
  `cargo xtask adr promote` numbers it at ship)
- Test: none — prose, gated by `prettier` and `doc-links`.

**Interfaces:**

- Consumes: the step name and both budgets (Task 1/5).
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Add the `CONTRIBUTING.md` ladder bullet**

State the step, both surfaces, the budget, and that `<Show>`/`<For>` are free —
the last is what a reader cannot guess and what makes the gate cheap to satisfy.

- [ ] **Step 2: Write the ADR draft**

Heading exactly `# ADR-DRAFT: <Title>`. Record: ADR-0050 _assumed_ thin
components; #520 removed the exemption and made the wasm-only layout the reason
they are invisible; this ADR makes thinness **enforced**, with the three-tier
remediation as the sanctioned response and **D11**'s no-escape-hatch stance.
Link sibling ADRs bare (`0050-…md`, not `../0050-…md`) per
`docs/adr/drafts/README.md` rule 4 — promotion strips one `../` level.

- [ ] **Step 3: Verify**

`cargo xtask check --no-test` — `prettier`, `adr-format`, and `doc-links` all
ok.

- [ ] **Step 4: Commit**

Prettier reflows Markdown during the gate; re-check `git status --porcelain` and
stage the rewrite.

```bash
git add CONTRIBUTING.md docs/adr/drafts docs/superpowers/plans
git commit -m "docs: record the enforced thin-component invariant"
```

---

## Final verification

- [ ] `cargo xtask validate --no-e2e` — the full local gate.
- [ ] **One e2e combo.** This branch restructures `web/` components, which the
      CSR bundle compiles and the browser renders, so host tests cannot see a
      broken view tree: run `cargo xtask e2e-local`. The full matrix is CI's
      (ADR-0034).
- [ ] **AC11 — no new CLI surface.** `cargo xtask --help`; the subcommand list
      is unchanged from `main`, and `thin-components` appears only as a step
      name.
- [ ] **AC12 — coverage untouched.**
      `git diff --stat wt-base-issue-306..HEAD -- xtask/src/coverage/` is empty.
- [ ] **AC9 — every extraction is asserted.** For each new `*_logic.rs`/state
      module, confirm `#[cfg(test)]` assertions exist, and that any new
      subcomponent is itself within both budgets (the gate proves the latter).
- [ ] **AC15 — no `TimelineGate`.** `rg -n 'TimelineGate' web/src` → no matches;
      #671's design was not pre-empted.
- [ ] Confirm `git status --porcelain` is clean.
