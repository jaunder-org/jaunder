//! `thin-components` — a Leptos `#[component]` body must stay thin (#306).
//!
//! ADR-0050 reasons about coverage on the assumption that component bodies carry no
//! real logic. Nothing enforced it. The blind spot is structural: every component
//! lives in a wasm-only `component.rs` (`#[cfg(target_arch = "wasm32")] mod
//! component;`, ADR-0070), so its lines are never compiled for the host and never
//! enter the coverage denominator — not measured, not exempt. Logic that drifts in
//! there is invisible *and* unassertable, and no coverage gate can notice. This step
//! notices.
//!
//! **Two surfaces, two mechanisms, because `syn` cannot see inside a macro.** A
//! `view!` invocation's contents are an opaque `TokenStream` — there are no
//! `ExprIf`/`ExprMatch` nodes in there to visit. So:
//!
//! - **setup** — the body outside any macro, counted over the **AST**.
//! - **view** — a `view!` macro's contents, counted over its **token stream**.
//!
//! They are reported separately because the remedies differ: setup complexity belongs
//! in a host-tested function; view complexity wants a subcomponent.
//!
//! Token-level counting is *why* `<Show>`/`<For>` and child components are free —
//! they arrive as `Punct('<') Ident(Show) …`, matching nothing in the count set. That
//! is deliberate: the cheapest way to satisfy this gate is the idiomatic Leptos form
//! or a subcomponent, both improvements. A guard that counted `<Show>` would push
//! authors toward hand-rolled `move || if` instead.
//!
//! Unlike [`crate::coverage::exempt`], an unparseable file is a **hard failure**
//! here, not a fail-closed no-op: there, recognising nothing leaves lines measured
//! (safe); here, it would let a fat component through.

use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};

use crate::files;
use crate::result::{CommandResult, StepResult};

/// The only tree with Leptos components. Scanned recursively; a missing root is a hard
/// failure, so a moved directory cannot quietly disable the guard.
const POLICED_ROOT: &str = "web/src";

/// Control-flow units permitted per surface. Both surfaces share one number; they are
/// counted and reported independently.
const BUDGET: u32 = 2;

const SETUP_REMEDY: &str = "extract this logic into a host-tested function in the \
                            vertical's host-compiled module (#306)";
const VIEW_REMEDY: &str = "extract a subcomponent, or use <Show>/<For> instead of \
                           `move || if` (#306)";

/// Which surface a count came from. It selects the remedy named in the failure, so it
/// is part of the message rather than a detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Setup,
    View,
}

impl Surface {
    fn label(self) -> &'static str {
        match self {
            Surface::Setup => "setup",
            Surface::View => "view",
        }
    }

    fn remedy(self) -> &'static str {
        match self {
            Surface::Setup => SETUP_REMEDY,
            Surface::View => VIEW_REMEDY,
        }
    }
}

/// One over-budget component surface.
#[derive(Debug)]
pub struct Violation {
    pub component: String,
    pub line: u32,
    pub surface: Surface,
    pub count: u32,
}

/// Both surface counts for one component.
struct Counts {
    setup: u32,
    view: u32,
}

/// True when `attrs` carries `#[component]`. `syn` does not expand macros, so the
/// attribute is still present on the parsed `ItemFn`.
fn is_component(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("component"))
}

/// True for the `view!` macro, matching on the path's LAST segment so a qualified
/// `leptos::view!` is recognised too.
fn is_view_macro(mac: &syn::Macro) -> bool {
    mac.path
        .segments
        .last()
        .is_some_and(|seg| seg.ident == "view")
}

/// Count control-flow tokens in a macro's token stream.
///
/// A candidate `Ident` is skipped when the **next** token is `=`: that is the HTML
/// attribute-assignment position (`<label for="x">`, `<label for=id.clone()>`), and no
/// Rust control-flow keyword can occupy it (`for x = …` is not valid Rust — `for` is
/// always followed by a pattern). Without this, the eight `for=` labels in
/// `web/src/posts/component.rs` alone would score as loops, putting a component over
/// budget with no control flow to extract — an unfixable failure.
///
/// String and char literals arrive as a single `Literal`, so the word "if" inside
/// markup text never counts. A nested `view!` is part of this same stream and is
/// therefore walked exactly once.
fn count_tokens(tokens: TokenStream) -> u32 {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut count = 0;
    for (i, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                if matches!(name.as_str(), "if" | "match" | "for" | "while") {
                    let attribute_position = matches!(
                        trees.get(i + 1),
                        Some(TokenTree::Punct(p)) if p.as_char() == '='
                    );
                    if !attribute_position {
                        count += 1;
                    }
                }
            }
            TokenTree::Punct(p) if p.as_char() == '?' => count += 1,
            TokenTree::Group(g) => count += count_tokens(g.stream()),
            _ => {}
        }
    }
    count
}

/// Walks a component body, counting AST control flow as `setup` and each macro's
/// tokens as either `view` (a `view!`) or `setup` (anything else).
///
/// Attributing a non-`view!` macro's tokens to `setup` keeps anything from escaping
/// both surfaces while still labelling it honestly: control flow inside a `format!`
/// argument *is* setup logic, and the setup remedy is the right advice for it.
struct BodyVisitor {
    setup: u32,
    view: u32,
}

impl BodyVisitor {
    fn record_macro(&mut self, mac: &syn::Macro) {
        let count = count_tokens(mac.tokens.clone());
        if is_view_macro(mac) {
            self.view += count;
        } else {
            self.setup += count;
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for BodyVisitor {
    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        // `else if` is a nested `ExprIf`, so recursing counts a chain once per arm.
        self.setup += 1;
        syn::visit::visit_expr_if(self, node);
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        // One unit per `match`, not per arm.
        self.setup += 1;
        syn::visit::visit_expr_match(self, node);
    }

    /// A guarded arm (`Ok(list) if list.is_empty() => …`) is a branch the `match`
    /// itself does not account for. `syn` models the guard as `Arm.guard`, **not** as
    /// an `ExprIf`, so without this a `match` can carry unlimited invisible branching
    /// — the same class of hiding place as `let … else` and a keyword-named attribute.
    ///
    /// Setup-surface only, and deliberately so: on the view surface the guard's `if`
    /// arrives as a plain `Ident("if")` token that is not followed by `=`, so the
    /// token counter already catches it.
    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        if node.guard.is_some() {
            self.setup += 1;
        }
        syn::visit::visit_arm(self, node);
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.setup += 1;
        syn::visit::visit_expr_for_loop(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.setup += 1;
        syn::visit::visit_expr_while(self, node);
    }

    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        self.setup += 1;
        syn::visit::visit_expr_try(self, node);
    }

    /// `let … else` is a `Local` carrying a `diverge` arm — **not** an `Expr` — so a
    /// visitor listing only expression nodes scores it zero. It is a real branch with
    /// an early return, and this codebase's dominant idiom for the param parsing that
    /// should leave a component body.
    fn visit_local(&mut self, node: &'ast syn::Local) {
        if node
            .init
            .as_ref()
            .is_some_and(|init| init.diverge.is_some())
        {
            self.setup += 1;
        }
        syn::visit::visit_local(self, node);
    }

    // The three macro positions. Each hands its tokens to the token counter and does
    // NOT recurse — `syn` cannot parse macro contents, so recursion would find
    // nothing while the tokens went uncounted. Missing any one of these lets a macro
    // escape both surfaces entirely: a silent under-count.
    //
    // `visit_stmt_macro` is the one that matters, and not for the reason you would
    // guess. `syn` models a macro in STATEMENT position as `Stmt::Macro` — including a
    // block's trailing `view! { … }` with no semicolon, which reads like a tail
    // expression but is not `Expr::Macro`. Since that is how every component in this
    // tree ends, hooking only `visit_expr_macro` would score the view surface ZERO
    // everywhere and yield a confidently green half-gate. Verified by deleting the
    // hook: three tests fail, two of them tail-position. `visit_expr_macro` still
    // earns its place for a macro nested inside an expression (`let v = view!{…};`).
    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        self.record_macro(&node.mac);
    }

    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        self.record_macro(&node.mac);
    }

    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        self.record_macro(&node.mac);
    }
}

fn count_body(block: &syn::Block) -> Counts {
    let mut visitor = BodyVisitor { setup: 0, view: 0 };
    syn::visit::visit_block(&mut visitor, block);
    Counts {
        setup: visitor.setup,
        view: visitor.view,
    }
}

/// Every `#[component]` in `items`, recursing into inline `mod x { … }` bodies so a
/// component nested in a module block is measured like any other.
fn collect(items: &[syn::Item], out: &mut Vec<(String, u32, Counts)>) {
    for item in items {
        match item {
            syn::Item::Fn(f) if is_component(&f.attrs) => {
                out.push((
                    f.sig.ident.to_string(),
                    f.sig.ident.span().start().line as u32,
                    count_body(&f.block),
                ));
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect(inner, out);
                }
            }
            _ => {}
        }
    }
}

/// Every over-budget component surface in `src`.
///
/// Returns `Err` if the file cannot be parsed — the caller reports that as its own
/// failure rather than skipping the file.
pub fn violations(src: &str) -> syn::Result<Vec<Violation>> {
    let file = syn::parse_file(src)?;
    let mut components = Vec::new();
    collect(&file.items, &mut components);

    let mut found = Vec::new();
    for (component, line, counts) in components {
        if counts.setup > BUDGET {
            found.push(Violation {
                component: component.clone(),
                line,
                surface: Surface::Setup,
                count: counts.setup,
            });
        }
        if counts.view > BUDGET {
            found.push(Violation {
                component,
                line,
                surface: Surface::View,
                count: counts.view,
            });
        }
    }
    Ok(found)
}

/// Detail lines, split by whether the guard could look at all.
///
/// The split is what lets one function serve both the report-only phase and
/// enforcement. "Could not parse" is a failure in **both** phases — a guard that
/// cannot read a file has not cleared it — while "over budget" is a failure only once
/// enforcement is on. Merging the two into one `Option<String>` would force the
/// report-only phase either to tolerate an unparseable file or to fail on the
/// remediation it is meant to be merely reporting.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Findings {
    /// Files the guard could not parse. Always a failure.
    pub blocked: Vec<String>,
    /// Over-budget component surfaces, each naming its remedy.
    pub over_budget: Vec<String>,
}

/// Classify every scanned file. Pure given the `(path, source)` pairs, so it is
/// unit-tested directly — the `target_arch_placement_check` shape.
pub fn findings(scanned: &[(String, String)]) -> Findings {
    let mut out = Findings::default();
    for (path, src) in scanned {
        match violations(src) {
            Ok(found) => {
                for v in found {
                    out.over_budget.push(format!(
                        "{path}:{} {}: {} complexity {} exceeds budget {BUDGET} — {}",
                        v.line,
                        v.component,
                        v.surface.label(),
                        v.count,
                        v.surface.remedy()
                    ));
                }
            }
            Err(e) => out.blocked.push(format!(
                "{path}: cannot parse — the thin-component guard cannot verify this \
                 file: {e}"
            )),
        }
    }
    out
}

/// Scan [`POLICED_ROOT`] and push the `thin-components` step.
pub fn run(result: &mut CommandResult) {
    let files = match files::with_extension(Path::new(POLICED_ROOT), "rs") {
        Ok(files) => files,
        Err(e) => {
            result.push(
                StepResult::fail("thin-components")
                    .detail(format!("cannot scan {POLICED_ROOT}: {e}")),
            );
            return;
        }
    };

    // A file we cannot read is reported, not skipped: silently dropping it would
    // disable the guard for exactly the file someone made unreadable.
    let mut scanned: Vec<(String, String)> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    for path in &files {
        match std::fs::read_to_string(path) {
            Ok(src) => scanned.push((path.display().to_string(), src)),
            Err(e) => unreadable.push(format!("{}: cannot read — {e}", path.display())),
        }
    }

    // REPORT-ONLY (#306, Task 1): over-budget components are LISTED, not failed, while
    // the remediation lands — the list is the authoritative input to Tasks 2-4. Wiring
    // the step now also gives the whole counter a real non-test caller; a `pub` item in
    // this private module is dead code under `-D warnings` until one exists, and
    // `#[cfg(test)]` use does not satisfy the lint.
    //
    // A file the guard cannot READ or PARSE fails even now: "could not look" is not
    // the same as "nothing over budget".
    //
    // TASK 5 FLIPS THIS: move `over_budget` into the failing branch alongside
    // `blocked`. That one change is the whole of enforcement.
    let found = findings(&scanned);
    let mut blocking = unreadable;
    blocking.extend(found.blocked);
    let step = if !blocking.is_empty() {
        StepResult::fail("thin-components").detail(blocking.join("\n"))
    } else if found.over_budget.is_empty() {
        StepResult::ok("thin-components")
    } else {
        StepResult::ok("thin-components").detail(format!(
            "REPORT-ONLY ({} over budget; #306 Task 5 makes this fail)\n{}",
            found.over_budget.len(),
            found.over_budget.join("\n")
        ))
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- setup surface ---

    #[test]
    fn setup_over_budget_is_flagged() {
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

    // --- view surface ---

    #[test]
    fn view_over_budget_is_flagged_as_view() {
        let src = "#[component]\nfn V() -> impl IntoView {\n\
                   view! { {move || if a {1} else {2}} {move || if b {1} else {2}}\n\
                           {move || match c { _ => 0 }} }\n}\n";
        let v = violations(src).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].surface, Surface::View);
        assert_eq!(v[0].count, 3);
    }

    #[test]
    fn thin_component_passes() {
        let src = "#[component]\nfn Thin() -> impl IntoView {\n\
                   let n = count.get();\n view! { <p>{n}</p> }\n}\n";
        assert!(violations(src).unwrap().is_empty());
    }

    // --- Leptos declaratives are free ---

    #[test]
    fn show_for_and_child_components_are_free() {
        let src = "#[component]\nfn D() -> impl IntoView {\n\
                   view! { <Show when=move || r><For each=xs key=k let:x><Row x=x/></For></Show>\n\
                           <Show when=move || s><Other/></Show><Show when=move || t><Third/></Show> }\n}\n";
        assert!(
            violations(src).unwrap().is_empty(),
            "Show/For/child components must not count"
        );
    }

    // --- trap 1: keyword-named HTML attributes ---

    #[test]
    fn html_for_attributes_do_not_count() {
        // The real `posts/component.rs` shapes: `for="…"` labels plus one whose value
        // is an expression (`:420`). Under a bare Ident match these score 3 and put the
        // component over budget with NO control flow to extract — unfixable by any
        // remediation, which is why this test exists.
        let src = "#[component]\nfn Labels() -> impl IntoView {\n\
                   view! { <label for=\"edit-slug\">\"S\"</label>\n\
                           <label for=\"edit-summary\">\"U\"</label>\n\
                           <label for=input_id.clone()>\"I\"</label> }\n}\n";
        assert!(
            violations(src).unwrap().is_empty(),
            "`for=` is an attribute, not a loop"
        );
    }

    // --- trap 2: let-else is a Local, not an Expr ---

    #[test]
    fn let_else_counts() {
        // The real `UserTagPage` shape (`posts/component.rs:941,951,957`) — three
        // consecutive early returns. A visitor listing only Expr nodes scores this 0.
        let src = "#[component]\nfn Tag() -> impl IntoView {\n\
                   let Some(username) = username else { return None };\n\
                   let Some(date) = date else { return None };\n\
                   let Some(slug) = slug else { return None };\n\
                   view! { <p></p> }\n}\n";
        let v = violations(src).unwrap();
        assert_eq!(v.len(), 1, "three let-else must exceed the budget");
        assert_eq!(v[0].surface, Surface::Setup);
        assert_eq!(v[0].count, 3);
    }

    // --- trap 3: statement-position macro ---

    #[test]
    fn a_statement_position_view_is_counted_on_the_view_surface() {
        // `Stmt::Macro`, not `Expr::Macro`. Hooking only the latter lets these tokens
        // escape BOTH surfaces — a silent under-count.
        let src = "#[component]\nfn S() -> impl IntoView {\n\
                   view! { {move || if a {1} else {2}} {move || if b {1} else {2}}\n\
                           {move || if c {1} else {2}} };\n\
                   unreachable!(\"shape fixture\")\n}\n";
        let v = violations(src).unwrap();
        assert_eq!(v.len(), 1, "statement-position view! must still be counted");
        assert_eq!(v[0].surface, Surface::View);
        assert_eq!(v[0].count, 3);
    }

    // --- every construct counts, asserted by VALUE ---

    #[test]
    fn every_counted_construct_is_recognized() {
        for (frag, what) in [
            (
                "let a = if p {1} else {2}; let b = if q {1} else {2}; let c = if r {1} else {2};",
                "if",
            ),
            (
                "let a = match p { _ => 1 }; let b = match q { _ => 1 }; let c = match r { _ => 1 };",
                "match",
            ),
            ("for _ in a {} for _ in b {} for _ in c {}", "for"),
            ("while a {} while b {} while c {}", "while"),
            ("let _ = (f()?, g()?, h()?);", "?"),
            (
                "let Some(a) = x else { return None }; let Some(b) = y else { return None }; \
                 let Some(c) = z else { return None };",
                "let-else",
            ),
        ] {
            let src = format!(
                "#[component]\nfn C() -> impl IntoView {{\n{frag}\nview! {{ <p></p> }}\n}}\n"
            );
            let v = violations(&src).unwrap();
            assert_eq!(v.len(), 1, "{what} must count");
            // Assert the VALUE: `len() == 1` alone passes against any mis-count that
            // still happens to exceed the budget.
            assert_eq!(v[0].count, 3, "{what} must count exactly 3");
        }
    }

    #[test]
    fn a_guarded_match_arm_counts() {
        // Found during Task 2's remediation: rewriting `Ok(l) if l.is_empty() => …`
        // as a nested if/else CHANGED the score, which means the guard form was
        // hiding a branch. A `match` scoring 1 while carrying three guarded arms is
        // an escape hatch, so each guard counts.
        let src = "#[component]\nfn C() -> impl IntoView {\n\
                   let v = match r {\n\
                   Ok(l) if l.is_empty() => 1,\n\
                   Ok(_) => 2,\n\
                   Err(_) => 3,\n\
                   };\n view! { <p></p> }\n}\n";
        let found = violations(src).unwrap();
        // 1 (match) + 1 (one guard) = 2, exactly at budget — so no violation, but the
        // guard must be *counted*, which the next case proves by crossing the budget.
        assert!(
            found.is_empty(),
            "match + one guard is exactly 2: {found:?}",
        );

        let two_guards = "#[component]\nfn C() -> impl IntoView {\n\
                          let v = match r {\n\
                          Ok(l) if l.is_empty() => 1,\n\
                          Ok(l) if l.len() > 9 => 2,\n\
                          _ => 3,\n\
                          };\n view! { <p></p> }\n}\n";
        let found = violations(two_guards).unwrap();
        assert_eq!(found.len(), 1, "match + two guards = 3, over budget");
        assert_eq!(found[0].count, 3);
        assert_eq!(found[0].surface, Surface::Setup);
    }

    #[test]
    fn else_if_counts_twice_and_a_big_match_counts_once() {
        let nested = "#[component]\nfn C() -> impl IntoView {\n\
                      let a = if p {1} else if q {2} else if r {3} else {4};\n\
                      view! { <p></p> }\n}\n";
        assert_eq!(violations(nested).unwrap()[0].count, 3, "3 nested ifs");

        let big = "#[component]\nfn C() -> impl IntoView {\n\
                   let a = match p { 1=>1, 2=>2, 3=>3, 4=>4, 5=>5, _=>0 };\n\
                   view! { <p></p> }\n}\n";
        assert!(
            violations(big).unwrap().is_empty(),
            "one match is one unit regardless of arm count"
        );
    }

    #[test]
    fn the_word_if_inside_a_string_literal_does_not_count() {
        let src = "#[component]\nfn C() -> impl IntoView {\n\
                   view! { <p>\"if if if if\"</p> <p>\"match for while\"</p> }\n}\n";
        assert!(
            violations(src).unwrap().is_empty(),
            "a literal is one token"
        );
    }

    #[test]
    fn a_plain_fn_with_control_flow_is_not_measured() {
        let src = "fn helper(p: bool) -> u8 {\n if p {1} else if p {2} else {3}\n}\n";
        assert!(
            violations(src).unwrap().is_empty(),
            "only #[component] bodies are measured"
        );
    }

    #[test]
    fn a_component_nested_in_a_mod_block_is_measured() {
        let src = "mod inner {\n#[component]\nfn Fat() -> impl IntoView {\n\
                   let a = if p {1} else {2}; let b = if q {1} else {2};\n\
                   let c = if r {1} else {2};\n view! { <p></p> }\n}\n}\n";
        assert_eq!(violations(src).unwrap().len(), 1, "recurse into mod blocks");
    }

    // --- parse failure is a hard failure ---

    #[test]
    fn unparseable_file_is_an_error_not_a_silent_pass() {
        assert!(violations("fn (").is_err());
    }

    #[test]
    fn a_parse_failure_is_blocked_not_merely_over_budget() {
        // `blocked` is the class that fails in BOTH phases, so an unparseable file must
        // land there rather than in `over_budget` — otherwise report-only would tolerate
        // a file the guard could not read.
        let found = findings(&[("web/src/x.rs".to_string(), "fn (".to_string())]);
        assert_eq!(found.blocked.len(), 1);
        assert!(found.blocked[0].contains("web/src/x.rs"));
        assert!(found.blocked[0].contains("parse"));
        assert!(found.over_budget.is_empty());
    }

    // --- the message names the remedy ---

    #[test]
    fn a_setup_violation_names_file_component_surface_and_remedy() {
        let src = "#[component]\nfn Fat() -> impl IntoView {\n\
                   let a = if p {1} else {2};\n let b = if q {1} else {2};\n\
                   let c = if r {1} else {2};\n view! { <p></p> }\n}\n";
        let found = findings(&[("web/src/posts/component.rs".to_string(), src.to_string())]);
        assert_eq!(found.over_budget.len(), 1);
        let detail = &found.over_budget[0];
        assert!(detail.contains("web/src/posts/component.rs"));
        assert!(detail.contains("Fat"));
        assert!(detail.contains("setup"));
        assert!(
            detail.contains("extract"),
            "the remedy must be in the message: {detail}"
        );
    }

    #[test]
    fn a_view_violation_names_the_subcomponent_remedy() {
        // The two surfaces must hand out DIFFERENT advice; asserting only the setup
        // remedy above would pass even if both surfaces shared one message.
        let src = "#[component]\nfn V() -> impl IntoView {\n\
                   view! { {move || if a {1} else {2}} {move || if b {1} else {2}}\n\
                           {move || if c {1} else {2}} }\n}\n";
        let found = findings(&[("web/src/a.rs".to_string(), src.to_string())]);
        assert_eq!(found.over_budget.len(), 1);
        let detail = &found.over_budget[0];
        assert!(detail.contains("view"));
        assert!(
            detail.contains("subcomponent"),
            "the view remedy must name subcomponent extraction: {detail}"
        );
    }

    #[test]
    fn a_thin_tree_yields_no_findings_of_either_class() {
        let found = findings(&[(
            "web/src/a.rs".to_string(),
            "#[component]\nfn T() -> impl IntoView { view! { <p></p> } }\n".to_string(),
        )]);
        assert_eq!(found, Findings::default());
    }
}
