//! The `raw-html-door` static check (#333): pins `PreEscaped` — maud's raw,
//! unescaped-markup constructor — to enumerated, individually justified sites.
//!
//! `web`'s render layer carries trusted HTML in one type, `web::html::Markup`, and
//! `Markup` is the only thing maud will splice into an `html!` without escaping it.
//! That makes the compiler, not a scanner, the thing stopping a hand-built string
//! from reaching the DOM — **except** at `PreEscaped`, which mints trust out of an
//! arbitrary `String`. It is the render layer's counterpart to
//! [`crate::steps::rendered_html_from_trusted_check`]'s `from_trusted`: one door
//! that *asserts* an invariant nothing checked.
//!
//! **Population** (read structurally, ADR-0085 principle 1): every `PreEscaped`
//! ident under [`POLICED_ROOTS`], in ordinary code **and inside macro token
//! streams** — the render layer is `html!` bodies, so a gate blind to macro tokens
//! would be blind to the whole layer. Every member fails unless the line
//! **immediately above** it carries a `// raw-html-door:allow <reason>` marker
//! (#778), so a second door added inside an already-marked fn is a failure rather
//! than a silent absorption (ADR-0085 principle 4).
//!
//! The scan itself — test-code exemption, enclosing-fn tracking, the macro token
//! walk and the marker rule — is [`crate::steps::ident_gate`]; this module is the
//! population and the prose.
//!
//! The one construct outside the population is a `use` declaration. An import names
//! the constructor but cannot mint anything — the mint is the call, which is its own
//! ident occurrence and is in the population.
//!
//! Because the scan sees author-written macro **invocation** tokens and never
//! expansions, maud's own internal `PreEscaped` use inside the expanded `html!` is
//! invisible here and needs no exemption.
//!
//! Test/fixture code (anything under a `#[cfg(test)]` module/impl/fn, or a
//! `#[test]`/`#[rstest]` fn) is exempt — fixtures legitimately mint raw markup.
//!
//! **Unreadable classes** (ADR-0085's honesty obligation) specific to this gate: a
//! `use maud::PreEscaped as Raw` rename evades ident matching, as does a re-exported
//! alias under any other name — `syn` has no name resolution. That is as visible in
//! review as adding a marker. The classes inherent to the shared scan (the unwalked
//! attribute-macro tokens, the unverifiable marker reason, the absent call graph)
//! are stated in [`crate::steps::ident_gate`]. A `syn` parse failure is a
//! **hard error** (a file we cannot walk could hide a door — a false pass).

use crate::result::CommandResult;
use crate::steps::ident_gate::{self, Gate, Report};

/// Source roots scanned recursively for `.rs` files — the same production `src`
/// trees [`crate::steps::rendered_html_from_trusted_check`] polices, not the
/// `tests/` integration crates (whose fixtures mint freely). The door only makes
/// sense in `web` today, but the population is defined by the tree, not by where we
/// expect the door to appear: a `PreEscaped` sprouting in `server` or `csr` must
/// fail rather than sit outside the gate's reach.
const POLICED_ROOTS: &[&str] = &[
    "common/src",
    "host/src",
    "storage/src",
    "web/src",
    "server/src",
    "csr/src",
    "macros/src",
];

/// The constructor ident this guard pins.
const DOORS: &[&str] = &["PreEscaped"];

/// The gate: population, roots and prose. Exemptions are in-source markers on the
/// line above each door (#778), so there is no list here.
///
/// One door today, and the design intends it to stay that way: `Markup` is the
/// render layer's trusted carrier and composes into `html!` unescaped by
/// construction, so ordinary markup never needs a door. Only a value whose safety was
/// *established* elsewhere does — a `RenderedHtml` that `RenderedHtml::sanitize`
/// scrubbed (ADR-0079). `web/src/html.rs`'s `Markup::from_rendered_html` is that
/// door, and its marker sits beside the `// XSS SAFETY:` prose that explains it.
const GATE: Gate = Gate {
    step: "raw-html-door",
    roots: POLICED_ROOTS,
    population: DOORS,
    report: Report {
        subject: "`PreEscaped`",
        verdict: "is not a marked raw-HTML door — markup minted here reaches the DOM \
                  unescaped (XSS) (#333)",
        recovery: "  recovery: `PreEscaped` asserts trust rather than establishing it. Trusted \
                   markup already has a carrier — build it with `html!` and wrap it in `Markup`, \
                   which composes unescaped by construction and needs no door. The only value \
                   that legitimately needs the raw door is a `RenderedHtml`, whose safety \
                   `RenderedHtml::sanitize` established; reach it through \
                   `Markup::from_rendered_html`. If this really is a new door, say why in a \
                   `// raw-html-door:allow <reason>` comment on the line IMMEDIATELY ABOVE it — \
                   not trailing it, which the formatters move. Currently marked:",
    },
};

/// 1-based `(line, enclosing-fn)` of every unmarked mention, plus every orphan
/// marker (empty fn name). Test-only: [`problems`] parses once and classifies itself,
/// so this is the single-source convenience the unit tests assert through.
#[cfg(test)]
fn violations(source: &str) -> Result<Vec<(usize, String)>, String> {
    GATE.violations(source)
}

/// The failure detail for every offending mention across the scanned files, or
/// `None` when every door is marked.
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

    /// The crate's one door, as `web/src/html.rs` writes it — the `use` that reaches
    /// the constructor plus the single marked call. Both name `PreEscaped`; only the
    /// call is in the population, which is what makes ONE marker the right cost.
    const THE_DOOR: &str = r#"
        use maud::{PreEscaped, Render};

        impl Markup {
            pub fn from_rendered_html(html: &RenderedHtml) -> Self {
                // XSS SAFETY: inherited from sanitization (ADR-0079).
                // raw-html-door:allow re-wraps a RenderedHtml whose safety sanitization established (ADR-0079)
                Self(PreEscaped(html.as_ref()).into_string())
            }
        }
    "#;

    #[test]
    fn the_marked_door_passes() {
        assert_eq!(violations(THE_DOOR).unwrap(), vec![]);
    }

    /// ADR-0085 principle 4: the exemption is scoped to a line, so a SECOND door in
    /// the same fn is a violation rather than absorbed by a fn-keyed entry.
    #[test]
    fn a_second_door_inside_the_marked_fn_is_a_violation() {
        let src = r#"
            impl Markup {
                pub fn from_rendered_html(html: &RenderedHtml) -> Self {
                    let _ = PreEscaped("<b>".to_string());
                    // raw-html-door:allow inherits sanitize's invariant (ADR-0079)
                    Self(PreEscaped(html.to_string()))
                }
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    /// The fn name bought the old exemption; it buys nothing now.
    #[test]
    fn a_formerly_allowlisted_fn_name_grants_nothing() {
        let src = r#"
            fn from_rendered_html(html: &RenderedHtml) -> Markup {
                Markup(PreEscaped(html.to_string()))
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_bare_marker_fails() {
        let src =
            "fn f(s: &str) -> Markup {\n    // raw-html-door:allow\n    Markup(PreEscaped(s))\n}\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn an_orphan_marker_fails() {
        let src = "// raw-html-door:allow stale\nfn f() { harmless(); }\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn an_html_sink_marker_does_not_exempt_a_door() {
        let src = "fn f(s: &str) -> Markup {\n    // html-sink:allow wrong gate\n    Markup(PreEscaped(s))\n}\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    /// The whole reason this gate exists: the render layer is macro bodies now.
    #[test]
    fn a_door_inside_an_html_macro_body_is_detected() {
        let src = r#"
            fn render_thing(s: &str) -> Markup {
                Markup::new(html! { div { (PreEscaped(s)) } })
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1, "macro body must be scanned: {hits:?}");
        assert_eq!(hits[0].1, "render_thing");
    }

    #[test]
    fn a_door_nested_deeper_in_macro_groups_is_detected() {
        let src = r#"
            fn render_thing(s: &str) -> Markup {
                Markup::new(html! { div { @if true { (PreEscaped(s)) } } })
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_mention_in_a_comment_is_not_a_token_and_does_not_trip() {
        let src = r#"
            /// See `PreEscaped` for the raw door.
            fn render_thing() -> Markup { Markup::empty() }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// An import names the constructor but mints nothing, so it is outside the
    /// population — the call it enables is its own occurrence and is not. Being
    /// inside would cost the one real door a second marker for its `use`.
    #[test]
    fn an_import_alone_is_outside_the_population() {
        let src = "use maud::PreEscaped;\n";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_door_in_a_cfg_test_module_is_exempt() {
        let src = r#"
            #[cfg(test)]
            mod tests {
                fn fixture() -> Markup {
                    Markup(PreEscaped("<p>x</p>".to_string()))
                }
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_door_in_a_test_fn_is_exempt() {
        let src = r#"
            #[test]
            fn t() {
                let _ = html! { (PreEscaped("<p>x</p>")) };
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_cfg_not_test_production_fn_is_scanned() {
        let src = r#"
            #[cfg(not(test))]
            fn prod_only(s: &str) -> Markup {
                Markup(PreEscaped(s.to_string()))
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_door_at_module_scope_is_flagged() {
        let src = "static X: () = { let _ = PreEscaped(\"<p>x</p>\"); };\n";
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "");
    }

    #[test]
    fn unparseable_source_is_a_hard_error() {
        assert!(violations("fn broken( {").is_err());
    }

    /// The tree as it stands: one door, marked.
    fn the_real_tree() -> Vec<(String, String)> {
        vec![("web/src/html.rs".to_string(), THE_DOOR.to_string())]
    }

    #[test]
    fn problems_is_none_for_the_marked_door() {
        assert_eq!(problems(&the_real_tree()), None);
    }

    /// The cross-file hole a fn-name key left: a *second* file grows a fn named
    /// `from_rendered_html` with a door and inherits the entry's exemption. There is
    /// no name to inherit now — the new door is simply unmarked.
    #[test]
    fn a_same_named_fn_in_another_file_gets_no_exemption() {
        let mut scanned = the_real_tree();
        scanned.push((
            "web/src/posts/render.rs".to_string(),
            "fn from_rendered_html(s: &str) -> Markup { Markup(PreEscaped(s)) }\n".to_string(),
        ));
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("web/src/posts/render.rs:1"), "{detail}");
    }

    /// An empty tree has no doors and no markers, so there is nothing to fail — the
    /// staleness class the declared list created is gone with the list.
    #[test]
    fn an_empty_tree_is_clean() {
        assert_eq!(problems(&[]), None);
    }

    #[test]
    fn problems_reports_file_line_and_recovery() {
        let mut scanned = the_real_tree();
        scanned.push((
            "web/src/x.rs".to_string(),
            "fn sneaky(s: &str) -> Markup { Markup(PreEscaped(s.to_string())) }\n".to_string(),
        ));
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("web/src/x.rs:1"));
        assert!(detail.contains("not a marked raw-HTML door"));
        assert!(detail.contains("raw-html-door:allow"));
        assert!(
            detail.contains("web/src/html.rs:8 — re-wraps a RenderedHtml"),
            "the derived census names the real door: {detail}"
        );
    }

    #[test]
    fn problems_surfaces_a_parse_failure_with_the_file() {
        let scanned = vec![("web/src/broken.rs".to_string(), "fn ( {{{".to_string())];
        let detail = problems(&scanned).expect("a hard error");
        assert!(detail.contains("web/src/broken.rs"));
        assert!(detail.contains("cannot parse"));
    }
}
