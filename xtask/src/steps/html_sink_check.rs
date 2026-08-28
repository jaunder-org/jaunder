//! The `html-sink` static check (#333): pins every unescaped-HTML sink —
//! `inner_html` and `set_inner_html` — to enumerated, individually justified sites.
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
//! Every member fails unless the line **immediately above** it carries a
//! `// html-sink:allow <reason>` marker (#778). The marker is what keeps the
//! exemption scoped to one site rather than to a function (ADR-0085 principle 4):
//! `PostDisplay` genuinely holds two sinks, and each argues for itself instead of
//! sharing one fn-keyed entry that a third would silently join.
//!
//! The scan itself — test-code exemption, enclosing-fn tracking, the macro token
//! walk and the marker rule — is [`crate::steps::ident_gate`]; this module is the
//! population and the prose.
//!
//! Test/fixture code (anything under a `#[cfg(test)]` module/impl/fn, or a
//! `#[test]`/`#[rstest]` fn) is exempt.
//!
//! **Unreadable classes** (ADR-0085's honesty obligation) specific to this gate: a
//! rename or re-export evades ident matching — `use web_sys::Element as E` changes
//! nothing, but a wrapper method named something else does. A `use` declaration is
//! outside the population: it reaches no sink, and what it enables is its own ident
//! occurrence. The classes inherent to the shared scan (the unwalked attribute-macro
//! tokens, the unverifiable marker reason, and above all the absent call graph — a sink
//! reached through a helper is flagged at the helper, never at the caller that
//! supplied the untrusted string) are stated in [`crate::steps::ident_gate`]. A
//! `syn` parse failure is a **hard error** (a file we cannot walk could hide a sink —
//! a false pass).

use crate::result::CommandResult;
use crate::steps::ident_gate::{self, Gate, Report};

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

/// The gate: population, roots and prose. Exemptions are in-source markers on the
/// line above each sink (#778), so there is no list here.
///
/// **The reasons those markers carry all have the same shape, and that is the
/// point:** the injected value is the output of the *pure render layer* — the
/// identical fn the projector server-renders — so it is markup we built, escaped by
/// maud, never a string that arrived from outside. A marker that cannot say that is
/// a sink that should not exist.
///
/// (Each sink carries its own marker and its own reason (#778) — a reason list
/// keyed by enclosing fn cannot distinguish `PostDisplay`'s two sinks, the
/// anonymous and authored layouts of the same article.)
const GATE: Gate = Gate {
    step: "html-sink",
    roots: POLICED_ROOTS,
    population: SINKS,
    // `inner_html` is a Leptos macro attribute and `set_inner_html` a `web_sys` method
    // reached through `.` on a runtime receiver: neither carries a path qualifier to
    // resolve (#790).
    owner: None,
    report: Report {
        subject: "an unescaped-HTML sink",
        verdict: "is not marked — whatever string reaches it is parsed as markup (XSS) (#333)",
        recovery: "  recovery: an unescaped sink is only safe when the string was built by our own \
                   render layer — a `Markup`, or a `RenderedHtml` that `host::render::sanitize` \
                   scrubbed. If the value came from anywhere else, do not inject it: render it as \
                   text, where maud escapes it. If this is a genuine coincidence sink (the \
                   projector paints the same markup), say so in a \
                   `// html-sink:allow <reason>` comment on the line IMMEDIATELY ABOVE the sink \
                   — not trailing it, which the formatters move. Currently marked:",
    },
};

/// 1-based `(line, enclosing-fn)` of every unmarked sink, plus every orphan marker
/// (empty fn name).
/// Test-only: [`problems`] parses once and classifies itself, so this is
/// the single-source convenience the unit tests assert through.
#[cfg(test)]
fn violations(source: &str) -> Result<Vec<(usize, String)>, String> {
    GATE.violations(source)
}

/// The failure detail for every offending sink across the scanned files, or `None`
/// when every sink is marked.
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

    /// `PostDisplay` legitimately holds TWO sinks, and each now argues for itself.
    #[test]
    fn two_sinks_in_one_fn_each_need_their_own_marker() {
        let src = r#"
            fn PostDisplay(view: PostView) -> AnyView {
                if a {
                    // html-sink:allow anonymous layout — posts::render output (#179)
                    view! { <article inner_html=inner></article> }.into_any()
                } else {
                    // html-sink:allow authored layout — posts::render output (#181)
                    view! { <div inner_html=inner_content></div> }.into_any()
                }
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// A fn name buys nothing: only a per-sink marker exempts.
    #[test]
    fn a_formerly_allowlisted_fn_name_grants_nothing() {
        let src = r#"
            fn PostDisplay(view: PostView) -> AnyView {
                view! { <article inner_html=inner></article> }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_marked_sink_passes() {
        let src = r#"
            fn anything(html: Markup) -> AnyView {
                // html-sink:allow pure render output
                view! { <div inner_html=html></div> }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_bare_marker_fails() {
        let src = "fn f() {\n    // html-sink:allow\n    el.set_inner_html(h);\n}\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn an_orphan_marker_fails() {
        let src = "// html-sink:allow stale\nfn f() { harmless(); }\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    /// Trailing is the form the formatters relocate, so it must never appear to
    /// work: the site is unmarked AND the marker is an orphan.
    #[test]
    fn a_trailing_marker_does_not_exempt() {
        let src = "fn f() { el.set_inner_html(h); } // html-sink:allow trailing\n";
        assert_eq!(violations(src).unwrap().len(), 2);
    }

    #[test]
    fn a_doc_comment_marker_does_not_exempt() {
        let src = "/// html-sink:allow prose\nfn f() { el.set_inner_html(h); }\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn two_sinks_on_the_marked_line_fail() {
        let src = "fn f() {\n    // html-sink:allow r\n    el.set_inner_html(a); el.set_inner_html(b);\n}\n";
        assert_eq!(violations(src).unwrap().len(), 2);
    }

    /// A call whose arguments wrap: the ident is still on the statement's first
    /// line, so the marker above that line is correct and the sink passes.
    #[test]
    fn a_wrapped_call_is_marked_above_its_ident_line() {
        let src = "fn f() {\n    // html-sink:allow pure render output\n    el.set_inner_html(\n        h,\n    );\n}\n";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// But the marker binds to the IDENT's line, not to the statement's. When the
    /// sink is nested deeper, marking the outer statement exempts nothing — an
    /// orphan marker plus an unmarked sink.
    #[test]
    fn marking_the_statement_instead_of_the_ident_line_exempts_nothing() {
        let src = "fn f() {\n    // html-sink:allow wrong line\n    wrap(\n        el.set_inner_html(h),\n    );\n}\n";
        assert_eq!(violations(src).unwrap().len(), 2);
    }

    #[test]
    fn a_raw_html_door_marker_does_not_exempt_a_sink() {
        let src = "fn f() {\n    // raw-html-door:allow wrong gate\n    el.set_inner_html(h);\n}\n";
        assert_eq!(violations(src).unwrap().len(), 1);
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

    /// The five real sites, in their four fns — the shape the tree has today, each
    /// carrying the marker it carries in production.
    fn the_real_tree() -> Vec<(String, String)> {
        vec![
            (
                "web/src/posts/component.rs".to_string(),
                r#"
                    fn PostDisplay(view: PostView) -> AnyView {
                        match children {
                            // html-sink:allow anonymous layout — posts::render output (#179)
                            None => view! { <article inner_html=inner></article> }.into_any(),
                            // html-sink:allow authored layout — posts::render output (#181)
                            Some(c) => view! { <div inner_html=inner_content></div> }.into_any(),
                        }
                    }
                    fn permalink_first_paint(seed: Option<PostResponse>) -> AnyView {
                        // html-sink:allow permalink_article output — the projector's own paint
                        view! { <div inner_html=html></div> }.into_any()
                    }
                "#
                .to_string(),
            ),
            (
                "web/src/home/component.rs".to_string(),
                r#"
                    fn HomePage() -> impl IntoView {
                        // html-sink:allow render_masthead output — the shared pure fn (ADR-0041 §2)
                        view! { <div inner_html=masthead.clone()></div> }
                    }
                "#
                .to_string(),
            ),
            (
                "web/src/sidebar/component.rs".to_string(),
                r#"
                    fn Sidebar() -> impl IntoView {
                        // html-sink:allow render_sidebar output — the anonymous paint
                        view! { <div inner_html=anon_html.clone()></div> }
                    }
                "#
                .to_string(),
            ),
        ]
    }

    #[test]
    fn problems_is_none_for_the_fully_marked_tree() {
        assert_eq!(problems(&the_real_tree()), None);
    }

    /// The cross-file hole a fn-name key left: a *second* file grows a fn named
    /// `Sidebar` with a sink and inherits the entry's exemption. There is no name to
    /// inherit now — the new sink is simply unmarked.
    #[test]
    fn a_same_named_fn_in_another_file_gets_no_exemption() {
        let mut scanned = the_real_tree();
        scanned.push((
            "web/src/admin/component.rs".to_string(),
            "fn Sidebar() -> AnyView { view! { <div inner_html=raw></div> }.into_any() }\n"
                .to_string(),
        ));
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("web/src/admin/component.rs:1"), "{detail}");
    }

    /// An empty tree has no sinks and no markers, so there is nothing to reconcile
    /// and nothing to fail — no declared list means no staleness to report.
    #[test]
    fn an_empty_tree_is_clean() {
        assert_eq!(problems(&[]), None);
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
        assert!(detail.contains("an unescaped-HTML sink in fn `sneaky` is not marked"));
        assert!(detail.contains("html-sink:allow"));
    }

    /// The census is derived from the scan, so it lists the sites the tree actually
    /// has — it cannot describe a site that is gone.
    #[test]
    fn problems_ends_with_the_derived_census() {
        let mut scanned = the_real_tree();
        scanned.push((
            "web/src/x.rs".to_string(),
            "fn sneaky(html: String) -> AnyView { view! { <div inner_html=html></div> }.into_any() }\n"
                .to_string(),
        ));
        let detail = problems(&scanned).expect("a problem");
        assert!(
            detail.contains("web/src/home/component.rs:4 — render_masthead output — the shared pure fn (ADR-0041 §2)"),
            "{detail}"
        );
    }

    #[test]
    fn problems_reports_a_bare_marker_distinctly() {
        let scanned = vec![(
            "web/src/x.rs".to_string(),
            "fn f() {\n    // html-sink:allow\n    el.set_inner_html(h);\n}\n".to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("bare"), "{detail}");
    }

    #[test]
    fn problems_reports_a_shared_line_distinctly() {
        let scanned = vec![(
            "web/src/x.rs".to_string(),
            "fn f() {\n    // html-sink:allow r\n    el.set_inner_html(a); el.set_inner_html(b);\n}\n"
                .to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("split the line"), "{detail}");
    }

    #[test]
    fn problems_reports_an_orphan_marker_distinctly() {
        let scanned = vec![(
            "web/src/x.rs".to_string(),
            "// html-sink:allow stale\nfn f() { harmless(); }\n".to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("stale exemption"), "{detail}");
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
