//! The `html-sink` static check (#333): pins every unescaped-HTML sink in `web` —
//! `inner_html` and `set_inner_html` — to an enumerated allowlist.
//!
//! These are the DOM's raw doors. Whatever string reaches them is parsed as markup,
//! so a value that was never escaped or sanitized becomes script. `web` reaches them
//! deliberately, in a handful of places, because the projector's server-painted
//! markup and the CSR client's first paint must coincide (ADR-0041 §2/§4) and the
//! only way to reuse the *same* pure render output is to inject it. That is a good
//! reason, and it is exactly why the set must stay small and written down.
//!
//! **Population** (read structurally, ADR-0085 principle 1): every `inner_html` or
//! `set_inner_html` **ident** anywhere under [`POLICED_ROOT`], in ordinary code and
//! inside macro token streams. Deliberately *not* "attributes inside a `view!`": a
//! `web_sys` `set_inner_html`, a builder-API call, or a bare reference is then
//! inside the population rather than silently outside it. Deciding the population by
//! the syntax that reaches the sink would be a search for the spelling we
//! anticipated, which is the failure mode ADR-0085 exists to stop.
//!
//! Every member fails unless an [`ALLOWLIST`] entry names its enclosing top-level fn
//! *and* the entry's declared multiplicity still covers it. The multiplicity is what
//! keeps an entry site-scoped rather than fn-scoped (ADR-0085 principle 4):
//! `PostDisplay` genuinely holds two sinks, its entry says two, and a third is a
//! failure rather than an absorption.
//!
//! Test/fixture code (anything under a `#[cfg(test)]` module/impl/fn, or a
//! `#[test]`/`#[rstest]` fn) is exempt.
//!
//! **Unreadable classes** (ADR-0085's honesty obligation): (1) a sink reached
//! through a helper that takes the element and the string as parameters — `syn` has
//! no call graph, so the helper is flagged and the *caller* that supplied the
//! untrusted string is not; the gate can detect, not attribute. (2) A rename or
//! re-export (`use web_sys::Element as E` changes nothing, but a wrapper method
//! named something else does) evades ident matching. (3) Tokens inside an
//! *attribute* macro's argument list are not walked, only `syn::Macro` invocations.
//! (4) A `use` declaration is outside the population — it reaches no sink, and what
//! it enables is its own ident occurrence. (5) The allowlist is keyed by enclosing fn name, not by file, so a same-named fn
//! in another file matches the entry — the tree-wide multiplicity reconciliation in
//! [`problems`] catches the resulting count drift, but the per-file report will not
//! name it as a shadow. A `syn` parse failure is a **hard error** (a file we cannot
//! walk could hide a sink — a false pass), matching
//! [`crate::steps::rendered_html_from_trusted_check`].

use std::collections::HashMap;
use std::path::Path;

use crate::files;
use crate::result::{CommandResult, StepResult};

/// Source root scanned recursively for `.rs` files. `web` is the only crate that
/// touches the DOM's raw doors — `csr` mounts, `server` serializes — so the root is
/// where the sinks are, and a sink appearing elsewhere would be a different gate's
/// argument to make.
const POLICED_ROOT: &str = "web/src";

/// The sink idents this guard pins: leptos' `inner_html=` attribute and `web_sys`'
/// `Element::set_inner_html`.
const SINKS: &[&str] = &["inner_html", "set_inner_html"];

/// A sink permitted in production code, keyed by its enclosing **top-level**
/// function plus how many identical sites that key covers.
///
/// **The count is load-bearing, not decoration.** A bare function-scoped exemption
/// is a region exemption in disguise: a second `inner_html` added inside an allowed
/// fn would pass silently, which is the precise defect ADR-0085 principle 4 forbids
/// (and which #778 records against `rendered-html-from-trusted`'s `ALLOWED_FNS`).
/// `PostDisplay` is why the count cannot simply be 1: its two sinks are the
/// anonymous and authored layouts of the same article, indistinguishable to any key
/// a human would keep correct. Declaring the multiplicity means gaining a third is a
/// mismatch and a failure.
struct Allowed {
    /// Enclosing top-level function name.
    function: &'static str,
    /// How many sink sites this entry covers, tree-wide.
    count: usize,
    /// Why the value injected there is trusted.
    reason: &'static str,
}

/// Every production sink, each with its reason. The reasons all have the same shape,
/// and that is the point: the injected value is the output of the *pure render
/// layer* — the identical fn the projector server-renders — so it is markup we
/// built, escaped by maud, never a string that arrived from outside.
const ALLOWLIST: &[Allowed] = &[
    // `web/src/posts/component.rs:155`; sinks at `:189` (anonymous layout, the whole
    // article inner) and `:204` (authored layout, the content column the action
    // overlay sits beside). Both inject `posts::render::render_post_inner` /
    // `render_post_content` output — the same pure fn the public projector paints
    // (#179/#181, ADR-0041 §4), which is what makes the seeded first paint and the
    // reactive re-render coincide.
    Allowed {
        function: "PostDisplay",
        count: 2,
        reason: "posts::render output — the same pure render the projector paints (#179/#181)",
    },
    // `web/src/posts/component.rs:884`; sink at `:891`. Injects
    // `posts::render::permalink_article` output for the projector-seeded permalink,
    // so the `Suspense` fallback is byte-for-byte the paint it replaces.
    Allowed {
        function: "permalink_first_paint",
        count: 1,
        reason: "posts::render::permalink_article output — the projector's own permalink paint",
    },
    // `web/src/home/component.rs:15`; sink at `:69`. Injects
    // `home::render::render_masthead` output into the timeline gate's `children`
    // slot — the shared pure fn the projector renders too (ADR-0041 §2), with no
    // `view!` twin to drift.
    Allowed {
        function: "HomePage",
        count: 1,
        reason: "home::render::render_masthead output — the shared pure fn (ADR-0041 §2)",
    },
    // `web/src/sidebar/component.rs:53`; sink at `:69`. Injects
    // `sidebar::markup::render_sidebar` output for the anonymous viewer, so a
    // marker-seeded first paint and the reactive re-render coincide (#181/#591).
    Allowed {
        function: "Sidebar",
        count: 1,
        reason: "sidebar::markup::render_sidebar output — the anonymous paint the projector emits",
    },
];

/// One sink mention: where it is, and what encloses it.
#[derive(Debug, Clone)]
struct Mention {
    /// 1-based source line.
    line: usize,
    /// Nearest enclosing fn name; empty at module scope.
    function: String,
    /// Whether that fn is top-level (`fn_stack.len() == 1`). Only a top-level fn can
    /// match an allowlist entry, so a nested fn shadowing an allowed name cannot
    /// borrow its exemption.
    top_level: bool,
}

/// Every **non-test** sink mention in the source, in line order. `Err` on a `syn`
/// parse failure (fail-loud). Pure given the source, so it is unit-tested directly.
fn mentions(source: &str) -> Result<Vec<Mention>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = Scanner {
        test_depth: 0,
        fn_stack: Vec::new(),
        hits: Vec::new(),
    };
    syn::visit::visit_file(&mut scanner, &file);
    scanner.hits.sort_by_key(|m| m.line);
    Ok(scanner.hits)
}

/// 1-based `(line, enclosing-fn)` of every sink the [`ALLOWLIST`] does not cover.
/// Test-only: [`problems`] parses once and applies [`unjustified`] itself, so this is
/// the single-source convenience the unit tests assert through.
#[cfg(test)]
fn violations(source: &str) -> Result<Vec<(usize, String)>, String> {
    Ok(unjustified(&mentions(source)?, ALLOWLIST))
}

/// The mentions `allowlist` does not cover: everything outside an allowlisted
/// top-level fn, plus everything **beyond** an entry's declared multiplicity (the
/// later sites in line order, so the first `count` keep the exemption they were
/// written for).
fn unjustified(found: &[Mention], allowlist: &[Allowed]) -> Vec<(usize, String)> {
    let mut used: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for m in found {
        let entry = m
            .top_level
            .then(|| allowlist.iter().find(|a| a.function == m.function))
            .flatten();
        match entry {
            Some(a) => {
                let seen = used.entry(a.function).or_insert(0);
                *seen += 1;
                if *seen > a.count {
                    out.push((m.line, m.function.clone()));
                }
            }
            None => out.push((m.line, m.function.clone())),
        }
    }
    out
}

struct Scanner {
    /// >0 while inside a `#[cfg(test)]`/`#[test]` item — sinks there are exempt.
    test_depth: usize,
    /// Names of the enclosing functions; the last is the nearest.
    fn_stack: Vec<String>,
    hits: Vec<Mention>,
}

/// Whether an attribute list carries a test-enabling `#[cfg(test)]` (incl.
/// `cfg(all(test, …))` / `cfg(any(test, …))`). Pragmatic token scan: the attr is
/// `cfg`, its tokens mention `test`, and are not negated (`not(...)`). The
/// `not`-guard biases the rare `cfg(all(not(x), test))` toward being **scanned** (a
/// safe false-positive) rather than letting a production-only `cfg(not(test))` slip
/// through unscanned.
fn is_test_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| match &a.meta {
        syn::Meta::List(ml) if ml.path.is_ident("cfg") => {
            let toks = ml.tokens.to_string();
            toks.contains("test") && !toks.contains("not")
        }
        _ => false,
    })
}

/// Whether an attribute list carries a test-harness attribute (`#[test]`,
/// `#[tokio::test]`, `#[rstest]`). Belt-and-suspenders for a test fn that is not
/// wrapped in a `#[cfg(test)]` module.
fn has_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "test" || s.ident == "rstest")
    })
}

impl Scanner {
    /// Record a sink mention on `line`, unless it is test code.
    ///
    /// Allowlisting happens later, in [`unjustified`], because a multiplicity can
    /// only be judged once every site is in hand.
    fn record(&mut self, line: usize) {
        if self.test_depth > 0 {
            return;
        }
        self.hits.push(Mention {
            line,
            function: self.fn_stack.last().cloned().unwrap_or_default(),
            top_level: self.fn_stack.len() == 1,
        });
    }

    /// Walk a macro invocation's tokens, recursing through nested `Group`s, and
    /// record every sink ident. `syn` never parses these tokens, so nothing found
    /// here can duplicate a hit already found in the AST — and comments are not
    /// tokens, so prose mentioning `inner_html` cannot false-positive.
    fn walk_macro_tokens(&mut self, tokens: &proc_macro2::TokenStream) {
        for tt in tokens.clone() {
            match tt {
                proc_macro2::TokenTree::Group(g) => self.walk_macro_tokens(&g.stream()),
                proc_macro2::TokenTree::Ident(id) if SINKS.iter().any(|s| id == *s) => {
                    self.record(id.span().start().line);
                }
                _ => {}
            }
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for Scanner {
    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        let test = is_test_cfg(&i.attrs);
        self.test_depth += usize::from(test);
        syn::visit::visit_item_mod(self, i);
        self.test_depth -= usize::from(test);
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let test = is_test_cfg(&i.attrs);
        self.test_depth += usize::from(test);
        syn::visit::visit_item_impl(self, i);
        self.test_depth -= usize::from(test);
    }

    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let test = is_test_cfg(&i.attrs) || has_test_attr(&i.attrs);
        self.test_depth += usize::from(test);
        self.fn_stack.push(i.sig.ident.to_string());
        syn::visit::visit_item_fn(self, i);
        self.fn_stack.pop();
        self.test_depth -= usize::from(test);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        let test = is_test_cfg(&i.attrs) || has_test_attr(&i.attrs);
        self.test_depth += usize::from(test);
        self.fn_stack.push(i.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, i);
        self.fn_stack.pop();
        self.test_depth -= usize::from(test);
    }

    /// A `use` declaration is outside the population, for the same reason as in
    /// [`crate::steps::raw_html_door_check`]: it names something but reaches no sink,
    /// and whatever it enables is its own ident occurrence.
    fn visit_item_use(&mut self, _i: &'ast syn::ItemUse) {}

    /// The population is otherwise the **ident**, wherever it appears — a method
    /// call, a struct field, a bare reference. Matching the ident rather than a call
    /// shape is what keeps this an enumeration instead of a search for the spelling
    /// someone anticipated.
    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        if SINKS.iter().any(|s| i == *s) {
            self.record(i.span().start().line);
        }
    }

    /// `syn` stops at a macro invocation's boundary, so the tokens are walked by
    /// hand — leptos' `inner_html=` lives inside `view!`.
    fn visit_macro(&mut self, i: &'ast syn::Macro) {
        self.walk_macro_tokens(&i.tokens);
        syn::visit::visit_macro(self, i);
    }
}

/// The failure detail for every offending sink across the scanned files, or `None`
/// when the tree matches the allowlist exactly. A per-file parse failure is surfaced
/// (never swallowed). Pure given the `(path, source)` pairs, so it is unit-tested
/// directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    let mut found: Vec<(String, Mention)> = Vec::new();
    for (path, source) in scanned {
        match mentions(source) {
            Err(msg) => lines.push(format!(
                "{path}: {msg} — an unparsed file is invisible to this gate, which is exactly the \
                 blind spot it exists to close. Fix the file or the parser; do not skip it."
            )),
            Ok(ms) => {
                for (ln, enclosing) in unjustified(&ms, ALLOWLIST) {
                    let where_ = if enclosing.is_empty() {
                        "at module scope".to_string()
                    } else {
                        format!("in fn `{enclosing}`")
                    };
                    lines.push(format!(
                        "{path}:{ln}: an unescaped-HTML sink {where_} is not allowlisted — \
                         whatever string reaches it is parsed as markup (XSS) (#333)"
                    ));
                }
                found.extend(ms.into_iter().map(|m| (path.clone(), m)));
            }
        }
    }

    // Stale or drifted entries: an allowlist that stops tracking the tree has
    // silently become a region exemption. This is also what catches a *second* file
    // growing a same-named fn — the per-file pass would hand it the entry's
    // exemption, but the tree-wide total no longer matches.
    for e in ALLOWLIST {
        let seen = found
            .iter()
            .filter(|(_, m)| m.top_level && m.function == e.function)
            .count();
        if seen != e.count {
            lines.push(format!(
                "fn `{}`: allowlist entry declares {} sink(s), the tree has {}. {}",
                e.function,
                e.count,
                seen,
                if seen == 0 {
                    "The sink is gone — delete the entry."
                } else {
                    "Re-justify each site, then update the count."
                }
            ));
        }
    }

    if lines.is_empty() {
        return None;
    }
    lines.sort();
    lines.push(
        "  recovery: an unescaped sink is only safe when the string was built by our own render \
         layer — a `Markup`, or a `RenderedHtml` that `RenderedHtml::sanitize` scrubbed. If the \
         value came from anywhere else, do not inject it: render it as text, where maud escapes \
         it. If this is a genuine coincidence sink (the projector paints the same markup), add \
         an ALLOWLIST entry with its multiplicity and a written reason in \
         xtask/src/steps/html_sink_check.rs. Currently exempt:"
            .to_string(),
    );
    for a in ALLOWLIST {
        lines.push(format!(
            "    - fn `{}` ×{}: {}",
            a.function, a.count, a.reason
        ));
    }
    Some(lines.join("\n"))
}

/// Scan every Rust file under [`POLICED_ROOT`] and push the result step. A missing
/// root is a hard failure, so a moved/renamed tree can never quietly disable the
/// guard.
pub fn run(result: &mut CommandResult) {
    let files = match files::with_extension(Path::new(POLICED_ROOT), "rs") {
        Ok(found) => found,
        Err(e) => {
            result.push(
                StepResult::fail("html-sink").detail(format!("cannot scan {POLICED_ROOT}: {e}")),
            );
            return;
        }
    };
    let mut scanned = Vec::new();
    let mut read_errors = Vec::new();
    for p in &files {
        match std::fs::read_to_string(p) {
            Ok(s) => scanned.push((p.display().to_string(), s)),
            Err(e) => read_errors.push(format!("{}: cannot read: {e}", p.display())),
        }
    }
    let step = match (read_errors.is_empty(), problems(&scanned)) {
        (true, None) => StepResult::ok("html-sink"),
        (_, prob) => {
            read_errors.extend(prob);
            StepResult::fail("html-sink").detail(read_errors.join("\n"))
        }
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::{problems, violations};

    /// `PostDisplay` legitimately holds TWO sinks; its entry says so.
    #[test]
    fn an_allowlisted_fn_at_its_declared_multiplicity_passes() {
        let src = r#"
            fn PostDisplay(view: PostView) -> AnyView {
                if a {
                    view! { <article inner_html=inner></article> }.into_any()
                } else {
                    view! { <div inner_html=inner_content></div> }.into_any()
                }
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn an_unlisted_sink_is_a_violation() {
        let src = r#"
            fn sneaky(html: String) -> AnyView {
                view! { <div inner_html=html></div> }.into_any()
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "sneaky");
    }

    /// The population is the sink, not the syntax that reaches it: a `web_sys` call
    /// outside any `view!` must NOT self-exempt (ADR-0085 principle 3).
    #[test]
    fn set_inner_html_outside_a_macro_is_in_the_population() {
        let src = r#"
            fn direct(el: &web_sys::Element, html: &str) {
                el.set_inner_html(html);
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn exceeding_a_declared_multiplicity_is_a_violation() {
        let src = r#"
            fn Sidebar() -> AnyView {
                view! {
                    <div inner_html=anon_html.clone()></div>
                    <aside inner_html=anon_html.clone()></aside>
                }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_comment_mentioning_inner_html_does_not_trip() {
        let src = r#"
            /// Injected via `inner_html` so the paint coincides.
            fn harmless() {}
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn unparseable_source_is_a_hard_error() {
        assert!(violations("fn broken( {").is_err());
    }

    /// A nested fn shadowing an allowed name must not borrow the entry's exemption —
    /// the allowlist is pinned to a *top-level* fn.
    #[test]
    fn a_nested_fn_shadowing_an_allowed_name_is_still_flagged() {
        let src = r#"
            fn outer() -> AnyView {
                fn Sidebar() -> AnyView {
                    view! { <div inner_html=anon_html></div> }.into_any()
                }
                Sidebar()
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "Sidebar");
    }

    #[test]
    fn a_sink_in_a_cfg_test_module_is_exempt() {
        let src = r#"
            #[cfg(test)]
            mod tests {
                fn fixture(el: &web_sys::Element) {
                    el.set_inner_html("<p>x</p>");
                }
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_cfg_not_test_production_fn_is_scanned() {
        let src = r#"
            #[cfg(not(test))]
            fn prod_only(el: &web_sys::Element, html: &str) {
                el.set_inner_html(html);
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    /// The five real sites, in their four fns — the shape the tree has today. Both
    /// halves of the rule must hold at once: nothing unjustified, and every entry's
    /// declared multiplicity reconciled against the whole tree.
    fn the_real_tree() -> Vec<(String, String)> {
        vec![
            (
                "web/src/posts/component.rs".to_string(),
                r#"
                    fn PostDisplay(view: PostView) -> AnyView {
                        match children {
                            None => view! { <article inner_html=inner></article> }.into_any(),
                            Some(c) => view! { <div inner_html=inner_content></div> }.into_any(),
                        }
                    }
                    fn permalink_first_paint(seed: Option<PostResponse>) -> AnyView {
                        view! { <div inner_html=html></div> }.into_any()
                    }
                "#
                .to_string(),
            ),
            (
                "web/src/home/component.rs".to_string(),
                r#"
                    fn HomePage() -> impl IntoView {
                        view! { <div inner_html=masthead.clone()></div> }
                    }
                "#
                .to_string(),
            ),
            (
                "web/src/sidebar/component.rs".to_string(),
                r#"
                    fn Sidebar() -> impl IntoView {
                        view! { <div inner_html=anon_html.clone()></div> }
                    }
                "#
                .to_string(),
            ),
        ]
    }

    #[test]
    fn problems_is_none_for_the_four_allowlisted_fns_at_their_multiplicities() {
        assert_eq!(problems(&the_real_tree()), None);
    }

    /// The cross-file hole a fn-name key would otherwise leave: a *second* file
    /// grows a fn named `Sidebar` with a sink, and the per-file pass hands it the
    /// entry's exemption. The tree-wide reconciliation is what refuses it.
    #[test]
    fn a_same_named_fn_in_another_file_breaks_the_declared_multiplicity() {
        let mut scanned = the_real_tree();
        scanned.push((
            "web/src/admin/component.rs".to_string(),
            "fn Sidebar() -> AnyView { view! { <div inner_html=raw></div> }.into_any() }\n"
                .to_string(),
        ));
        let detail = problems(&scanned).expect("a problem");
        assert!(
            detail.contains("declares 1 sink(s), the tree has 2"),
            "{detail}"
        );
    }

    /// A sink that disappears leaves a stale entry, which is an exemption nobody is
    /// re-justifying. Deleting the entry is part of deleting the sink.
    #[test]
    fn a_vanished_sink_leaves_a_stale_entry_that_fails() {
        let detail = problems(&[]).expect("a problem");
        assert!(
            detail.contains("The sink is gone — delete the entry."),
            "{detail}"
        );
    }

    #[test]
    fn problems_reports_file_line_and_recovery() {
        let mut scanned = the_real_tree();
        scanned.push((
            "web/src/x.rs".to_string(),
            "fn sneaky(html: String) -> AnyView { view! { <div inner_html=html></div> }.into_any() }\n"
                .to_string(),
        ));
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("web/src/x.rs:1"));
        assert!(detail.contains("an unescaped-HTML sink in fn `sneaky` is not allowlisted"));
        assert!(detail.contains("ALLOWLIST"));
        assert!(detail.contains("fn `PostDisplay` ×2"));
    }

    #[test]
    fn problems_surfaces_a_parse_failure_with_the_file() {
        let scanned = vec![("web/src/broken.rs".to_string(), "fn ( {{{".to_string())];
        let detail = problems(&scanned).expect("a hard error");
        assert!(detail.contains("web/src/broken.rs"));
        assert!(detail.contains("cannot parse"));
    }

    #[test]
    fn an_import_alone_is_outside_the_population() {
        let src = "use crate::dom::inner_html;\n";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_sink_at_module_scope_is_flagged() {
        let src = "static X: () = { el.set_inner_html(\"<p>x</p>\"); };\n";
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "");
    }
}
