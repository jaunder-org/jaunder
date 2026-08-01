//! The `raw-html-door` static check (#333): pins `PreEscaped` — maud's raw,
//! unescaped-markup constructor — to an enumerated allowlist of production sites.
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
//! would be blind to the whole layer. Every member fails unless an [`ALLOWLIST`]
//! entry names its enclosing top-level fn *and* the entry's declared multiplicity
//! still covers it, so a second door added inside an already-allowed fn is a
//! failure rather than a silent absorption (ADR-0085 principle 4).
//!
//! The scan itself — test-code exemption, enclosing-fn tracking, the macro token
//! walk, the multiplicity rule and the tree-wide reconciliation — is
//! [`crate::steps::ident_gate`]; this module is the population, the allowlist and
//! the prose.
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
//! review as editing the allowlist. The classes inherent to the shared scan (the
//! unwalked attribute-macro tokens, the fn-name-keyed allowlist, the absent call
//! graph) are stated in [`crate::steps::ident_gate`]. A `syn` parse failure is a
//! **hard error** (a file we cannot walk could hide a door — a false pass).

use crate::result::CommandResult;
use crate::steps::ident_gate::{self, Allowed, AnyOf, Gate, Report};

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

/// Every production raw door, each with its reason.
///
/// One entry, and the design intends it to stay that way: `Markup` is the render
/// layer's trusted carrier and composes into `html!` unescaped by construction, so
/// ordinary markup never needs a door. Only a value whose safety was *established*
/// elsewhere does.
///
/// The count on each entry is load-bearing — see [`Allowed`].
const ALLOWLIST: &[Allowed] = &[
    // `web/src/html.rs` — `Markup::from_rendered_html`, the crate's single raw door,
    // carrying the `// XSS SAFETY:` comment beside it. It re-wraps a
    // `common::render::RenderedHtml`, whose invariant `RenderedHtml::sanitize`
    // established by scrubbing (ADR-0079); this only inherits it, which is why the
    // value may be emitted unescaped. Everything else in `web` builds markup with
    // `html!`, where maud escapes text for us.
    Allowed {
        function: "from_rendered_html",
        count: 1,
        reason: "re-wraps a RenderedHtml whose safety sanitization established (ADR-0079)",
    },
];

/// The gate: population, allowlist, roots and prose.
const GATE: Gate<AnyOf> = Gate {
    step: "raw-html-door",
    roots: POLICED_ROOTS,
    population: AnyOf(DOORS),
    allowlist: ALLOWLIST,
    report: Report {
        subject: "`PreEscaped`",
        verdict: "is not an allowlisted raw-HTML door — markup minted here reaches the DOM \
                  unescaped (XSS) (#333)",
        noun: "raw door(s)",
        vanished: "The door is gone — delete the entry.",
        recovery: "  recovery: `PreEscaped` asserts trust rather than establishing it. Trusted \
                   markup already has a carrier — build it with `html!` and wrap it in `Markup`, \
                   which composes unescaped by construction and needs no door. The only value \
                   that legitimately needs the raw door is a `RenderedHtml`, whose safety \
                   `RenderedHtml::sanitize` established; reach it through \
                   `Markup::from_rendered_html`. If this really is a new door, add an ALLOWLIST \
                   entry with its multiplicity and a written reason in \
                   xtask/src/steps/raw_html_door_check.rs. Currently exempt:",
    },
};

/// 1-based `(line, enclosing-fn)` of every mention the real [`ALLOWLIST`] does not
/// cover. Test-only: [`problems`] parses once and applies the allowlist itself, so
/// this is the single-source convenience the unit tests assert through.
#[cfg(test)]
fn violations(source: &str) -> Result<Vec<(usize, String)>, String> {
    GATE.violations(source)
}

/// The failure detail for every offending mention across the scanned files, or
/// `None` when the tree matches the allowlist exactly.
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
    /// the constructor plus the single call. Both name `PreEscaped`; only the call is
    /// in the population, which is what makes ONE entry the right cost.
    const THE_DOOR: &str = r#"
        use maud::{PreEscaped, Render};

        impl Markup {
            pub fn from_rendered_html(html: &RenderedHtml) -> Self {
                // XSS SAFETY: inherited from sanitization (ADR-0079).
                Self(PreEscaped(html.as_ref()).into_string())
            }
        }
    "#;

    #[test]
    fn the_allowed_door_at_its_declared_multiplicity_passes() {
        assert_eq!(violations(THE_DOOR).unwrap(), vec![]);
    }

    /// ADR-0085 principle 4: the entry is scoped to a site with a multiplicity, so a
    /// SECOND door inside the same allowed fn is a violation, not absorbed.
    #[test]
    fn a_second_door_inside_the_allowed_fn_is_a_violation() {
        let src = r#"
            impl Markup {
                pub fn from_rendered_html(html: &RenderedHtml) -> Self {
                    let _ = PreEscaped("<b>".to_string());
                    Self(PreEscaped(html.to_string()))
                }
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    /// A nested fn shadowing an allowed name must not borrow the entry's exemption —
    /// the allowlist is pinned to a *top-level* fn.
    #[test]
    fn a_nested_fn_shadowing_the_allowed_name_is_still_flagged() {
        let src = r#"
            fn outer(html: &RenderedHtml) -> Markup {
                fn from_rendered_html(html: &RenderedHtml) -> Markup {
                    Markup(PreEscaped(html.to_string()))
                }
                from_rendered_html(html)
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "from_rendered_html");
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
    /// inside would cost the one real door a second allowlist entry for its `use`.
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

    /// The tree as it stands: one door, in the one fn the allowlist names.
    fn the_real_tree() -> Vec<(String, String)> {
        vec![("web/src/html.rs".to_string(), THE_DOOR.to_string())]
    }

    #[test]
    fn problems_is_none_for_the_one_declared_door() {
        assert_eq!(problems(&the_real_tree()), None);
    }

    /// The cross-file hole a fn-name key would otherwise leave: a *second* file grows
    /// a fn named `from_rendered_html` with a door, and the per-file pass hands it the
    /// entry's exemption. The tree-wide reconciliation is what refuses it.
    #[test]
    fn a_same_named_fn_in_another_file_breaks_the_declared_multiplicity() {
        let mut scanned = the_real_tree();
        scanned.push((
            "web/src/posts/render.rs".to_string(),
            "fn from_rendered_html(s: &str) -> Markup { Markup(PreEscaped(s)) }\n".to_string(),
        ));
        let detail = problems(&scanned).expect("a problem");
        assert!(
            detail.contains("declares 1 raw door(s), the tree has 2"),
            "{detail}"
        );
    }

    /// A door that disappears leaves a stale entry, which is an exemption nobody is
    /// re-justifying. Deleting the entry is part of deleting the door.
    #[test]
    fn a_vanished_door_leaves_a_stale_entry_that_fails() {
        let detail = problems(&[]).expect("a problem");
        assert!(
            detail.contains("The door is gone — delete the entry."),
            "{detail}"
        );
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
        assert!(detail.contains("not an allowlisted raw-HTML door"));
        assert!(detail.contains("ALLOWLIST"));
        assert!(detail.contains("fn `from_rendered_html` ×1"));
    }

    #[test]
    fn problems_surfaces_a_parse_failure_with_the_file() {
        let scanned = vec![("web/src/broken.rs".to_string(), "fn ( {{{".to_string())];
        let detail = problems(&scanned).expect("a hard error");
        assert!(detail.contains("web/src/broken.rs"));
        assert!(detail.contains("cannot parse"));
    }
}
