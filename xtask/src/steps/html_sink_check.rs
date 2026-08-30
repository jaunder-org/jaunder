//! The `html-sink` static check (#333): pins every unescaped-HTML sink —
//! `inner_html` and `set_inner_html` — to enumerated, individually justified sites.
//!
//! These are the DOM's raw doors. Whatever string reaches them is parsed as markup,
//! so a value that was never escaped or sanitized becomes script. `web` reaches one
//! deliberately through a typed adapter because the projector's server-painted
//! markup and the CSR client's first paint must coincide (ADR-0041 §2/§4), and the
//! only way to reuse the *same* pure render output is to inject it. That is a good
//! reason, and it is exactly why the audited sink stays singular and written down.
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
//! `// html-sink:allow <reason>` marker (#778). Ordinary trusted rendering routes
//! through `web::html::Markup::inject_into`, so its adapter sink owns the one production
//! marker. A genuinely different future sink must still justify itself at its
//! own source site (ADR-0085 principle 4).
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
/// The ordinary render path has one marked adapter sink: `Markup::inject_into`
/// accepts the render layer's typed `Markup`, then performs the final conversion
/// and injection. Callers choose their exact host element without handling an
/// arbitrary string or naming the raw attribute. A separate sink must carry its
/// own marker and reason rather than silently joining the adapter's exemption.
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
        recovery: "  recovery: route trusted render output through \
                   `web::html::Markup::inject_into`, whose interface accepts `Markup` rather than an \
                   arbitrary string. If the value came from anywhere else, do not inject it: \
                   render it as text, where maud escapes it. If a genuinely different raw sink \
                   is unavoidable, say why in a `// html-sink:allow <reason>` comment on the \
                   line IMMEDIATELY ABOVE the sink — not trailing it, which the formatters move. \
                   Currently marked:",
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

    const ADAPTER_SOURCE: &str = r#"
        impl Markup {
            fn inject_into(self, element: Element) -> AnyView {
                // html-sink:allow accepts only the render layer's trusted Markup output
                element.inner_html(self.into_string())
            }
        }
    "#;

    /// Ordinary callers use the typed adapter, whose one sink carries the marker.
    #[test]
    fn typed_adapter_owns_the_marked_sink() {
        assert_eq!(violations(ADAPTER_SOURCE).unwrap(), vec![]);
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

    /// The one real production site, carrying the marker it carries in the tree.
    fn the_real_tree() -> Vec<(String, String)> {
        vec![("web/src/html.rs".to_string(), ADAPTER_SOURCE.to_string())]
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
            detail.contains(
                "web/src/html.rs:5 — accepts only the render layer's trusted Markup output"
            ),
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
