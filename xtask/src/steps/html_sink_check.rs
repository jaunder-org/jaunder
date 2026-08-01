//! The `html-sink` static check (#333): pins every unescaped-HTML sink —
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
//! `set_inner_html` **ident** anywhere under [`POLICED_ROOTS`], in ordinary code and
//! inside macro token streams. Deliberately *not* "attributes inside a `view!`": a
//! `web_sys` `set_inner_html`, a builder-API call, or a bare reference is then
//! inside the population rather than silently outside it. Deciding the population by
//! the syntax that reaches the sink would be a search for the spelling we
//! anticipated, which is the failure mode ADR-0085 exists to stop.
//!
//! The roots are the whole production tree, not `web/src` alone, for the same reason
//! [`crate::steps::raw_html_door_check`] scans them all: `csr` is the crate that
//! mounts to the DOM, so a `set_inner_html` sprouting there must **fail** rather
//! than sit outside the gate's reach. A population scoped to where we expect the
//! construct to appear is a hypothesis, not an enumeration.
//!
//! Every member fails unless an [`ALLOWLIST`] entry names its enclosing top-level fn
//! *and* the entry's declared multiplicity still covers it. The multiplicity is what
//! keeps an entry site-scoped rather than fn-scoped (ADR-0085 principle 4):
//! `PostDisplay` genuinely holds two sinks, its entry says two, and a third is a
//! failure rather than an absorption.
//!
//! The scan itself — test-code exemption, enclosing-fn tracking, the macro token
//! walk, the multiplicity rule and the tree-wide reconciliation — is
//! [`crate::steps::ident_gate`]; this module is the population, the allowlist and
//! the prose.
//!
//! Test/fixture code (anything under a `#[cfg(test)]` module/impl/fn, or a
//! `#[test]`/`#[rstest]` fn) is exempt.
//!
//! **Unreadable classes** (ADR-0085's honesty obligation) specific to this gate: a
//! rename or re-export evades ident matching — `use web_sys::Element as E` changes
//! nothing, but a wrapper method named something else does. A `use` declaration is
//! outside the population: it reaches no sink, and what it enables is its own ident
//! occurrence. The classes inherent to the shared scan (the unwalked attribute-macro
//! tokens, the fn-name-keyed allowlist, and above all the absent call graph — a sink
//! reached through a helper is flagged at the helper, never at the caller that
//! supplied the untrusted string) are stated in [`crate::steps::ident_gate`]. A
//! `syn` parse failure is a **hard error** (a file we cannot walk could hide a sink —
//! a false pass).

use crate::result::CommandResult;
use crate::steps::ident_gate::{self, Allowed, AnyOf, Gate, Report};

/// Source roots scanned recursively for `.rs` files — the production `src` trees,
/// not the `tests/` integration crates (whose fixtures inject freely). The sinks all
/// live in `web` today; the roots are the same seven
/// [`crate::steps::raw_html_door_check`] polices because the population is defined
/// by the tree, not by where we expect a sink to appear.
const POLICED_ROOTS: &[&str] = &[
    "common/src",
    "host/src",
    "storage/src",
    "web/src",
    "server/src",
    "csr/src",
    "macros/src",
];

/// The sink idents this guard pins: leptos' `inner_html=` attribute and `web_sys`'
/// `Element::set_inner_html`.
const SINKS: &[&str] = &["inner_html", "set_inner_html"];

/// Every production sink, each with its reason. The reasons all have the same shape,
/// and that is the point: the injected value is the output of the *pure render
/// layer* — the identical fn the projector server-renders — so it is markup we
/// built, escaped by maud, never a string that arrived from outside.
///
/// The count on each entry is load-bearing — see [`Allowed`]. `PostDisplay` is why
/// it cannot simply be 1: its two sinks are the anonymous and authored layouts of
/// the same article, indistinguishable to any key a human would keep correct.
///
/// Entries name **fns**, not lines: the fn name is the key the gate matches on, and
/// a line number in a comment is checked by nothing and rots on the next edit.
const ALLOWLIST: &[Allowed] = &[
    // `web/src/posts/component.rs`. Two sinks: the anonymous layout (the whole
    // article inner) and the authored layout (the content column the action overlay
    // sits beside). Both inject `posts::render::render_post_inner` /
    // `render_post_content` output — the same pure fn the public projector paints
    // (#179/#181, ADR-0041 §4), which is what makes the seeded first paint and the
    // reactive re-render coincide.
    Allowed {
        function: "PostDisplay",
        count: 2,
        reason: "posts::render output — the same pure render the projector paints (#179/#181)",
    },
    // `web/src/posts/component.rs`. Injects `posts::render::permalink_article`
    // output for the projector-seeded permalink, so the `Suspense` fallback is
    // byte-for-byte the paint it replaces.
    Allowed {
        function: "permalink_first_paint",
        count: 1,
        reason: "posts::render::permalink_article output — the projector's own permalink paint",
    },
    // `web/src/home/component.rs`. Injects `home::render::render_masthead` output
    // into the timeline gate's `children` slot — the shared pure fn the projector
    // renders too (ADR-0041 §2), with no `view!` twin to drift.
    Allowed {
        function: "HomePage",
        count: 1,
        reason: "home::render::render_masthead output — the shared pure fn (ADR-0041 §2)",
    },
    // `web/src/sidebar/component.rs`. Injects `sidebar::markup::render_sidebar`
    // output for the anonymous viewer, so a marker-seeded first paint and the
    // reactive re-render coincide (#181/#591).
    Allowed {
        function: "Sidebar",
        count: 1,
        reason: "sidebar::markup::render_sidebar output — the anonymous paint the projector emits",
    },
];

/// The gate: population, allowlist, roots and prose.
const GATE: Gate<AnyOf> = Gate {
    step: "html-sink",
    roots: POLICED_ROOTS,
    population: AnyOf(SINKS),
    allowlist: ALLOWLIST,
    report: Report {
        subject: "an unescaped-HTML sink",
        verdict: "is not allowlisted — whatever string reaches it is parsed as markup (XSS) (#333)",
        noun: "sink(s)",
        vanished: "The sink is gone — delete the entry.",
        recovery:
            "  recovery: an unescaped sink is only safe when the string was built by our own \
                   render layer — a `Markup`, or a `RenderedHtml` that `RenderedHtml::sanitize` \
                   scrubbed. If the value came from anywhere else, do not inject it: render it as \
                   text, where maud escapes it. If this is a genuine coincidence sink (the \
                   projector paints the same markup), add an ALLOWLIST entry with its \
                   multiplicity and a written reason in xtask/src/steps/html_sink_check.rs. \
                   Currently exempt:",
    },
};

/// 1-based `(line, enclosing-fn)` of every sink the [`ALLOWLIST`] does not cover.
/// Test-only: [`problems`] parses once and applies the allowlist itself, so this is
/// the single-source convenience the unit tests assert through.
#[cfg(test)]
fn violations(source: &str) -> Result<Vec<(usize, String)>, String> {
    GATE.violations(source)
}

/// The failure detail for every offending sink across the scanned files, or `None`
/// when the tree matches the allowlist exactly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    GATE.problems(scanned)
}

/// Scan every Rust file under each [`POLICED_ROOTS`] and push the result step. A
/// missing root is a hard failure, so a moved/renamed tree can never quietly disable
/// the guard.
pub fn run(result: &mut CommandResult) {
    ident_gate::run_scan(result, GATE.step, GATE.roots, problems);
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
