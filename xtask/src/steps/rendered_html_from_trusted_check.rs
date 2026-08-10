//! The `rendered-html-from-trusted` static check (#398, extended #445, #778): pins
//! every `from_trusted` in production code — above all
//! `RenderedHtml::from_trusted`, the *inheriting* door of the
//! `common::render::RenderedHtml` newtype.
//!
//! `RenderedHtml` marks HTML that is safe to emit **unescaped** into the DOM
//! (`inner_html`). Two doors carry that invariant, and they mean different things:
//! `sanitize` **establishes** it by scrubbing against an allowlist (the door for
//! anything from outside jaunder — a rendered post body via `render()`, an ingested
//! feed entry, any future inbound producer), while `from_trusted` merely
//! **inherits** it, asserting a value we already sanitized has round-tripped through
//! our own storage or wire.
//!
//! Because `from_trusted` is `pub` and cross-crate, Rust visibility cannot confine
//! it: a future inbound path could launder a stranger's HTML into "trusted" and
//! compile clean, reopening the #445 stored-XSS hole. This gate makes that a
//! host-side failure — every **non-test** `from_trusted` mention must carry a
//! marker, so choosing the wrong door breaks the build instead of silently
//! shipping. (`sanitize` needs no gate: it is safe wherever it is called.)
//!
//! **Population** (read structurally, ADR-0085 principle 1): every `from_trusted`
//! ident under [`POLICED_ROOTS`], in ordinary code and inside macro token streams.
//! Deliberately **not** "only paths qualified by `RenderedHtml`". Until #778 this
//! gate skipped any `ContentType::from_trusted` by matching the path's qualifier —
//! a pattern-decided exemption (ADR-0085 principle 3, *"Grants no automatic
//! exemption from a pattern. Nothing self-exempts"*) that also failed **open**
//! asymmetrically: an aliased *leaf* stayed guarded, but an aliased *qualifier*
//! (`use RenderedHtml as ContentType`) handed out the exemption. `ContentType`'s
//! door now says so in a marker like anything else. Two consequences follow:
//!
//! - `ContentType::from_trusted` sites cost one marker each. That is the friction
//!   ADR-0085 prices deliberately, and it turns that door's own doc instruction
//!   ("grep `ContentType::from_trusted` to enumerate every mint site") from a
//!   request to a human into something the gate enforces.
//! - A `from_trusted` **definition** is in the population too — `syn` visits a fn's
//!   own `sig.ident` — so `pub fn from_trusted` carries a marker saying it is the
//!   door itself. Fails closed, which is the direction this gate must err in.
//!
//! Every member fails unless the line **immediately above** it carries a
//! `// rendered-html-from-trusted:allow <reason>` marker. The scan, the marker rule
//! and the derived census are [`crate::steps::ident_gate`]; this module is the
//! population and the prose.
//!
//! Test/fixture code (anything under a `#[cfg(test)]` module/fn, or a `#[test]`/
//! `#[rstest]` fn) is exempt — fixtures legitimately mint `RenderedHtml` to stand
//! in for rendered output.
//!
//! **Macro bodies are scanned** (#333). `syn` itself does not descend into a macro
//! invocation, so the shared scan walks [`syn::Macro`]'s `.tokens` directly,
//! recursing through nested `Group`s. That was an accepted limitation until #333
//! rebuilt `web`'s render layer out of `html!`/`view!` bodies, which is exactly
//! where the unescaped sink lives — the residual gap the old doc called "the most
//! plausible" became the ordinary case, so it is no longer acceptable.
//!
//! **Unreadable classes** (ADR-0085's honesty obligation) specific to this gate: a
//! same-named `from_trusted` on an unrelated type is in the population and costs a
//! marker — deliberate, since distinguishing them by qualifier is the fail-open
//! this gate just removed, and #790 tracks removing the collision at its source
//! instead. The classes inherent to the shared scan (a `use … as` rename, the
//! unwalked attribute-macro tokens, the absent call graph, and that a marker is
//! trusted rather than verified) are stated in [`crate::steps::ident_gate`]. A
//! `syn` parse failure is a **hard error** (a file we cannot walk could hide a
//! spurious door — a false pass), matching
//! [`crate::steps::server_fn_registrar_check`].

use crate::result::CommandResult;
use crate::steps::ident_gate::{self, Gate, Report};

/// Source roots scanned recursively for `.rs` files — production `src` trees, not
/// the `tests/` integration crates (whose fixtures mint freely).
const POLICED_ROOTS: &[&str] = &[
    "common/src",
    "host/src",
    "storage/src",
    "web/src",
    "server/src",
    "csr/src",
    "macros/src",
];

/// The associated-fn ident this guard pins.
const DOORS: &[&str] = &["from_trusted"];

/// The gate: population, roots and prose. Exemptions are in-source markers on the
/// line above each door (#778), so there is no list here.
///
/// The prose names the **ident**, not one type. Since #778 the population includes
/// `ContentType::from_trusted` and the definition sites, and a verdict asserting
/// "a raw string minted here is emitted unescaped" would be false at every one of
/// them — a gate that fails with an inaccurate reason teaches the wrong lesson at
/// the exact moment someone is reading it.
const GATE: Gate = Gate {
    step: "rendered-html-from-trusted",
    roots: POLICED_ROOTS,
    population: DOORS,
    // Not yet pointed at `RenderedHtml`: Task 5 of #790 flips this, together with the
    // marker deletions it makes correct. Until then the gate polices the bare ident.
    owner: None,
    report: Report {
        subject: "a `from_trusted` door",
        verdict: "is not marked — this gate pins every `from_trusted` in production code, \
                  because `RenderedHtml`'s is the door that lets HTML reach the DOM unescaped \
                  (XSS) (#398)",
        recovery: "  recovery: `from_trusted` only *inherits* safety — it may reconstruct a value we \
                   already sanitized and round-tripped through our own store or wire. If the HTML \
                   comes from OUTSIDE jaunder (an ingested feed entry, a remote channel, any \
                   inbound producer), it must go through `RenderedHtml::sanitize`, which \
                   *establishes* safety by scrubbing; for a rendered post body that means \
                   `render()`. A `from_trusted` on a different type (`ContentType`, #584) is not \
                   this door at all — say so and move on. Either way, put the reason in a \
                   `// rendered-html-from-trusted:allow <reason>` comment on the line IMMEDIATELY \
                   ABOVE the site — not trailing it, which the formatters move. Currently marked:",
    },
};

/// 1-based `(line, enclosing-fn)` of every unmarked mention, plus every orphan
/// marker (empty fn name). Test-only: [`problems`] parses once and classifies
/// itself, so this is the single-source convenience the unit tests assert through.
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
/// missing root is a hard failure, so a moved/renamed tree can never quietly
/// disable the guard.
pub fn run(result: &mut CommandResult) {
    ident_gate::run_scan(result, GATE.step, GATE.roots, problems);
}

#[cfg(test)]
mod tests {
    use super::{problems, violations};

    #[test]
    fn a_marked_door_passes() {
        let src = "fn deserialize_rendered_html(s: String) -> RenderedHtml {\n    // rendered-html-from-trusted:allow wire DTO our own server serialized (#445)\n    RenderedHtml::from_trusted(s)\n}\n";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// The fn name bought the old exemption; it buys nothing now.
    #[test]
    fn a_formerly_allowlisted_fn_name_grants_nothing() {
        let src = "fn deserialize_rendered_html(s: String) -> RenderedHtml { RenderedHtml::from_trusted(s) }\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn call_in_a_non_allowlisted_fn_is_flagged() {
        let src = "\
fn sneaky(raw: String) -> RenderedHtml {
    RenderedHtml::from_trusted(raw)
}
";
        assert_eq!(violations(src).unwrap(), vec![(2, "sneaky".to_string())]);
    }

    #[test]
    fn an_inbound_shaped_fn_using_from_trusted_is_flagged() {
        // The #445 shape this guard exists to stop: a future inbound producer
        // (feed ingestion, #282) reaching for the *inheriting* door on HTML that
        // came from a stranger's server. `RenderedHtml::sanitize` is the only
        // correct door for outside data, and this is what makes choosing wrong a
        // build failure rather than a silent stored-XSS hole.
        let src = "\
fn ingest_feed_entry(remote_html: String) -> RenderedHtml {
    RenderedHtml::from_trusted(remote_html)
}
";
        assert_eq!(
            violations(src).unwrap(),
            vec![(2, "ingest_feed_entry".to_string())]
        );
    }

    #[test]
    fn map_reference_in_a_non_allowlisted_fn_is_flagged() {
        let src = "\
fn sneaky(raw: String) -> RenderedHtml {
    Some(raw).map(RenderedHtml::from_trusted).unwrap()
}
";
        assert_eq!(violations(src).unwrap(), vec![(2, "sneaky".to_string())]);
    }

    /// #778: the qualifier exemption is gone. `ContentType::from_trusted` is a
    /// different door, but it says so in a marker rather than self-exempting from a
    /// pattern (ADR-0085 principle 3) — which also closes the qualifier-alias
    /// fail-open (`use RenderedHtml as ContentType`).
    #[test]
    fn a_content_type_door_is_in_the_population_and_needs_a_marker() {
        let src = "fn detect(n: &str) -> ContentType { ContentType::from_trusted(n) }\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_marked_content_type_door_passes() {
        let src = "fn detect(n: &str) -> ContentType {\n    // rendered-html-from-trusted:allow mints a media type, never HTML (#584)\n    ContentType::from_trusted(n)\n}\n";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_from_trusted_on_an_unrelated_type_is_still_flagged() {
        let src = "\
fn sneaky(raw: String) -> Widget {
    Widget::from_trusted(raw)
}
";
        assert_eq!(violations(src).unwrap(), vec![(2, "sneaky".to_string())]);
    }

    /// Replaces `the_definition_site_has_no_path_mention`. Now that membership is the
    /// bare ident wherever it occurs, the door's own declaration is in the population —
    /// a deliberate behavior change (#778), failing closed. `visit_impl_item_fn` pushes
    /// the fn name before the signature's ident is visited, so the mention's enclosing
    /// fn is the door itself.
    #[test]
    fn the_definition_site_is_in_the_population() {
        let src = "\
impl RenderedHtml {
    pub fn from_trusted(html: impl Into<String>) -> Self {
        Self(html.into())
    }
}
";
        assert_eq!(
            violations(src).unwrap(),
            vec![(2, "from_trusted".to_string())]
        );
    }

    #[test]
    fn a_marked_definition_site_passes() {
        let src = "\
impl RenderedHtml {
    // rendered-html-from-trusted:allow the door's own definition; the gate pins its uses
    pub fn from_trusted(html: impl Into<String>) -> Self {
        Self(html.into())
    }
}
";
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_bare_marker_fails() {
        let src = "fn f(raw: String) -> RenderedHtml {\n    // rendered-html-from-trusted:allow\n    RenderedHtml::from_trusted(raw)\n}\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn an_orphan_marker_fails() {
        let src = "// rendered-html-from-trusted:allow stale\nfn f() { harmless(); }\n";
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn call_in_a_cfg_test_module_is_exempt() {
        let src = "\
#[cfg(test)]
mod tests {
    fn fixture() -> RenderedHtml {
        RenderedHtml::from_trusted(\"<p>x</p>\")
    }
}
";
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn a_cfg_not_test_production_fn_is_scanned() {
        let src = "\
#[cfg(not(test))]
fn prod_only(raw: String) -> RenderedHtml {
    RenderedHtml::from_trusted(raw)
}
";
        assert_eq!(violations(src).unwrap(), vec![(3, "prod_only".to_string())]);
    }

    #[test]
    fn call_in_a_test_fn_is_exempt() {
        let src = "\
#[test]
fn t() {
    let _ = RenderedHtml::from_trusted(\"<p>x</p>\");
}
";
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn module_scope_call_is_flagged() {
        // Not inside any fn — no enclosing fn to name.
        let src = "static X: () = { RenderedHtml::from_trusted(\"<p>x</p>\"); };\n";
        let v = violations(src).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].1, "");
    }

    /// #333: the render layer is macro bodies now, so the limitation this gate
    /// documented ("syn does not descend into macro bodies") is no longer
    /// acceptable. A `from_trusted` inside a `view!` must be seen.
    #[test]
    fn from_trusted_inside_a_macro_body_is_detected() {
        let src = r#"
            fn sneaky(s: &str) -> AnyView {
                view! { <div inner_html=RenderedHtml::from_trusted(s).to_string()></div> }.into_any()
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1, "macro body must be scanned: {hits:?}");
        assert_eq!(hits[0].1, "sneaky");
    }

    #[test]
    fn from_trusted_nested_deeper_in_macro_groups_is_detected() {
        // The token walk recurses through nested `Group`s, so depth is not a hiding
        // place — a `view!` body is groups all the way down.
        let src = r#"
            fn sneaky(s: &str) -> AnyView {
                view! { <div>{ move || RenderedHtml::from_trusted(s).to_string() }</div> }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_marked_site_inside_a_macro_body_passes() {
        let src = r#"
            fn seeded(s: &str) -> AnyView {
                // rendered-html-from-trusted:allow wire DTO our own server serialized (#445)
                view! { <div inner_html=RenderedHtml::from_trusted(s).to_string()></div> }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn a_bare_from_trusted_inside_a_macro_body_is_flagged() {
        // No qualifier at all (a `use`-imported or aliased door) — the leaf is the
        // population, so it is guarded regardless of how it was reached.
        let src = r#"
            fn sneaky(s: &str) -> AnyView {
                view! { <div inner_html=from_trusted(s)></div> }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![(3, "sneaky".to_string())]);
    }

    #[test]
    fn a_macro_body_in_a_test_fn_is_exempt() {
        let src = r#"
            #[test]
            fn t() {
                let _ = view! { <div inner_html=RenderedHtml::from_trusted("x")></div> };
            }
        "#;
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn parse_failure_is_an_error() {
        assert!(violations("fn broken( {{{ not valid").is_err());
    }

    /// The verdict fires at `ContentType::from_trusted` and at definition sites too,
    /// so it must not make claims that are false there: that *this* site is
    /// `RenderedHtml`'s door, or that a string minted here reaches the DOM. Naming
    /// `RenderedHtml` while explaining why the gate exists is fine and stays —
    /// the assertion is about what the message claims of the site, not about which
    /// words appear in it.
    ///
    /// Checked on the violation line alone; the recovery paragraph discusses
    /// `RenderedHtml` at length and should.
    #[test]
    fn the_verdict_claims_nothing_false_at_a_content_type_site() {
        let scanned = vec![(
            "common/src/media.rs".to_string(),
            "fn detect(n: &str) -> ContentType { ContentType::from_trusted(n) }\n".to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        let verdict = detail.lines().next().expect("a violation line");
        assert!(
            !verdict.contains("RenderedHtml::from_trusted"),
            "must not call this site RenderedHtml's door: {verdict}"
        );
        assert!(
            !verdict.contains("minted here"),
            "must not claim this site mints unescaped HTML: {verdict}"
        );
        assert!(
            !verdict.contains("trusted-rebuild door"),
            "the pre-#778 wording described a population this gate no longer has: {verdict}"
        );
        assert!(verdict.contains("from_trusted"), "{verdict}");
    }

    #[test]
    fn problems_reports_file_line_and_recovery() {
        let scanned = vec![(
            "web/src/x.rs".to_string(),
            "fn sneaky(raw: String) -> RenderedHtml { RenderedHtml::from_trusted(raw) }\n"
                .to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("web/src/x.rs:1"));
        assert!(detail.contains("is not marked"));
        assert!(detail.contains("rendered-html-from-trusted:allow"));
    }

    #[test]
    fn problems_is_none_for_a_fully_marked_tree() {
        let scanned = vec![
            (
                "common/src/render.rs".to_string(),
                "fn deserialize_rendered_html(s: String) -> RenderedHtml {\n    // rendered-html-from-trusted:allow wire DTO (#445)\n    RenderedHtml::from_trusted(s)\n}\n"
                    .to_string(),
            ),
            (
                "storage/src/posts.rs".to_string(),
                "#[cfg(test)]\nmod t {\n  fn f() { let _ = RenderedHtml::from_trusted(\"x\"); }\n}\n"
                    .to_string(),
            ),
        ];
        assert_eq!(problems(&scanned), None);
    }

    #[test]
    fn problems_surfaces_a_parse_failure_with_the_file() {
        let scanned = vec![("common/src/broken.rs".to_string(), "fn ( {{{".to_string())];
        let detail = problems(&scanned).expect("a hard error");
        assert!(detail.contains("common/src/broken.rs"));
        assert!(detail.contains("cannot parse"));
    }
}
