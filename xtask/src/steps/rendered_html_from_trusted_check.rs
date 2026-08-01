//! The `rendered-html-from-trusted` static check (#398, extended #445): pins
//! `RenderedHtml::from_trusted` — the *inheriting* door of the
//! `common::render::RenderedHtml` newtype — to an allowlist of production call
//! sites.
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
//! host-side failure — every **non-test** `from_trusted` mention must sit in an
//! allowlisted function, so choosing the wrong door breaks the build instead of
//! silently shipping. (`sanitize` needs no gate: it is safe wherever it is called.)
//!
//! The scan — test-code exemption, enclosing-fn tracking, the macro token walk — is
//! [`crate::steps::ident_gate`]; this module is the population, the allowlist and
//! the prose. Unlike its two siblings there, this gate's [`ALLOWED_FNS`] entries
//! carry **no multiplicity**, so a second door added inside an already-allowed fn is
//! absorbed silently — a region exemption in disguise, and a violation of ADR-0085
//! principle 4 that #778 tracks. Adopting [`crate::steps::ident_gate::Allowed`] is
//! what closes it.
//!
//! Test/fixture code (anything under a `#[cfg(test)]` module/fn, or a `#[test]`/
//! `#[rstest]` fn) is exempt — fixtures legitimately mint `RenderedHtml` to stand
//! in for rendered output.
//!
//! Matching is on the `from_trusted` path leaf (via `syn`), so it catches both a
//! direct call `RenderedHtml::from_trusted(x)` and a bare reference
//! `.map(RenderedHtml::from_trusted)`, at a direct or fully-qualified path, and
//! pins the allowlist to a **top-level** fn (a nested fn shadowing an allowed name
//! is still flagged).
//!
//! **Macro bodies are scanned** (#333). `syn` itself does not descend into a macro
//! invocation, so the shared scan walks [`syn::Macro`]'s `.tokens` directly,
//! recursing through nested `Group`s, and this gate applies the same qualifier rule
//! to the `Ident :: Ident` token shape. That was an accepted limitation until #333
//! rebuilt `web`'s render layer out of `html!`/`view!` bodies, which is exactly
//! where the unescaped sink lives — the residual gap the old doc called "the most
//! plausible" became the ordinary case, so it is no longer acceptable.
//!
//! **Unreadable classes** (ADR-0085's honesty obligation) specific to this gate:
//! (1) a same-named `from_trusted` on an unrelated type false-positives — except the
//! [`EXEMPT_QUALIFIERS`] types (e.g. `ContentType::from_trusted`, #584), recognised
//! by qualifier as distinct non-HTML doors. (2) Inside macro tokens the qualifier is
//! read positionally (the ident three tokens left of the leaf), so a qualifier
//! spelled across a `Group` — e.g. `<Foo as Bar>::from_trusted` — is treated as
//! unqualified and stays guarded (a safe bias). The classes inherent to the shared
//! scan (a `use … as` rename, the unwalked attribute-macro tokens, the fn-name-keyed
//! allowlist, the absent call graph) are stated in [`crate::steps::ident_gate`]. A
//! `syn` parse failure is a **hard error** (a file we cannot walk could hide a
//! spurious door — a false pass), matching
//! [`crate::steps::server_fn_registrar_check`].

use crate::result::CommandResult;
use crate::steps::ident_gate::{self, Population};

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

/// The associated-fn leaf name this guard pins.
const DOOR: &str = "from_trusted";

/// Type qualifiers whose `from_trusted` is a **different, non-HTML door** and is
/// exempt from this XSS guard. Only `RenderedHtml::from_trusted` reaches the
/// unescaped `inner_html` sink; a same-named door on another newtype is unrelated.
/// `ContentType::from_trusted` (#584) mints a validated media type — never HTML — so
/// its qualifier is recognised and skipped. Matched on the qualifier segment
/// immediately left of the leaf, so a bare or `use … as`-aliased `from_trusted`
/// (no matching qualifier) stays guarded.
const EXEMPT_QUALIFIERS: &[&str] = &["ContentType"];

/// The functions permitted to call `from_trusted` in production code — the two
/// trusted round-trip doors. A new site must be added here (visible in review,
/// with justification) or the gate fails.
const ALLOWED_FNS: &[&str] = &[
    // Rebuilds `RenderedHtml` from a wire DTO field our own server serialized and
    // embedded in the page the client is already executing — inherited trust, and
    // the one production door left. `common/src/render.rs`, used by the seed DTOs
    // in `common/src/seed.rs`.
    //
    // `build_post_record` was here until #445: the `rendered_html` column decodes
    // straight into `RenderedHtml`, so the DB read no longer rebuilds through this
    // door. That decode is `#[derive(SqlxBridge)]` since #746 — still expanded in the
    // type's own module, so it still reaches the private constructor without a door.
    "deserialize_rendered_html",
];

/// The population: a `from_trusted` path leaf whose qualifier is not a recognised
/// other-type door. Not a bare ident, because membership turns on the **qualifier**
/// — which is what keeps `ContentType::from_trusted` (#584) outside the population
/// instead of costing an allowlist entry.
struct TrustedDoor;

impl Population for TrustedDoor {
    /// Nothing: a bare ident carries no qualifier, so the AST side reads paths
    /// ([`Population::expr_path`]) instead.
    fn ident(&self, _id: &proc_macro2::Ident) -> bool {
        false
    }

    /// Whether a path is the guarded `RenderedHtml::from_trusted` door: leaf segment
    /// `from_trusted` with a qualifier that is not in [`EXEMPT_QUALIFIERS`]. A bare
    /// or aliased `from_trusted` (no exempt qualifier) stays guarded.
    fn expr_path(&self, path: &syn::Path) -> bool {
        let mut rev = path.segments.iter().rev();
        if rev.next().is_none_or(|leaf| leaf.ident != DOOR) {
            return false;
        }
        // The qualifier is the segment immediately left of the leaf, if any.
        !rev.next()
            .is_some_and(|qualifier| EXEMPT_QUALIFIERS.iter().any(|q| qualifier.ident == q))
    }

    fn macro_ident(
        &self,
        id: &proc_macro2::Ident,
        trees: &[proc_macro2::TokenTree],
        idx: usize,
    ) -> bool {
        *id == DOOR && !macro_qualifier_is_exempt(trees, idx)
    }
}

/// Whether the `from_trusted` leaf at `idx` in a flat macro token stream is
/// qualified by an [`EXEMPT_QUALIFIERS`] type. Tokens carry no path structure, so a
/// qualified path reads as `Ident : : Ident` and the qualifier is the ident three
/// positions left, behind the two `:` puncts. Anything else — a bare leaf, or a
/// qualifier assembled inside a nested `Group` — reads as unqualified and stays
/// guarded, which is the safe bias.
fn macro_qualifier_is_exempt(trees: &[proc_macro2::TokenTree], idx: usize) -> bool {
    let Some(qualifier) = idx.checked_sub(3) else {
        return false;
    };
    let is_colon = |t: &proc_macro2::TokenTree| matches!(t, proc_macro2::TokenTree::Punct(p) if p.as_char() == ':');
    if !is_colon(&trees[qualifier + 1]) || !is_colon(&trees[qualifier + 2]) {
        return false;
    }
    matches!(
        &trees[qualifier],
        proc_macro2::TokenTree::Ident(id) if EXEMPT_QUALIFIERS.iter().any(|q| *id == *q)
    )
}

/// 1-based `(line, enclosing-fn)` of every **non-test** `from_trusted` mention
/// whose enclosing function is not allowlisted. `Err` on a `syn` parse failure
/// (fail-loud). Pure given the source, so it is unit-tested directly.
///
/// The door must be the WHOLE enclosing path, so a *nested* fn shadowing
/// `deserialize_rendered_html` cannot borrow its exemption.
fn violations(source: &str) -> Result<Vec<(usize, String)>, String> {
    Ok(ident_gate::mentions(source, &TrustedDoor)?
        .into_iter()
        .filter(|m| !(m.top_level && ALLOWED_FNS.contains(&m.function.as_str())))
        .map(|m| (m.line, m.function))
        .collect())
}

/// The failure detail for every offending mention across the scanned files, or
/// `None` when every non-test `from_trusted` sits in an allowlisted function. A
/// per-file parse failure is surfaced (never swallowed). Pure given the
/// `(path, source)` pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        match violations(source) {
            Err(msg) => lines.push(format!("{path}: {msg}")),
            Ok(hits) => {
                for (ln, enclosing) in hits {
                    let where_ = if enclosing.is_empty() {
                        "at module scope".to_string()
                    } else {
                        format!("in fn `{enclosing}`")
                    };
                    lines.push(format!(
                        "{path}:{ln}: `RenderedHtml::from_trusted` {where_} is not an allowlisted \
                         trusted-rebuild door — a raw string minted here is emitted unescaped (XSS) \
                         (#398)"
                    ));
                }
            }
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.sort();
    lines.push(format!(
        "  recovery: `from_trusted` only *inherits* safety — it may reconstruct a value we \
         already sanitized and round-tripped through our own store or wire. If the HTML comes \
         from OUTSIDE jaunder (an ingested feed entry, a remote channel, any inbound producer), \
         it must go through `RenderedHtml::sanitize`, which *establishes* safety by scrubbing; \
         for a rendered post body that means `render()`. Only if this is a genuine round-trip \
         door, add its fn to ALLOWED_FNS in \
         xtask/src/steps/rendered_html_from_trusted_check.rs ({}).",
        ALLOWED_FNS.join(", ")
    ));
    Some(lines.join("\n"))
}

/// Scan every Rust file under each [`POLICED_ROOTS`] and push the result step. A
/// missing root is a hard failure, so a moved/renamed tree can never quietly
/// disable the guard.
pub fn run(result: &mut CommandResult) {
    ident_gate::run_scan(
        result,
        "rendered-html-from-trusted",
        POLICED_ROOTS,
        problems,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlisted_fn_is_clean() {
        let src = "\
fn deserialize_rendered_html(wire: String) -> RenderedHtml {
    RenderedHtml::from_trusted(wire)
}
";
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn map_reference_in_allowlisted_fn_is_clean() {
        // The `.map(RenderedHtml::from_trusted)` reference (not a direct call) must
        // still resolve to its enclosing fn and be allowed.
        let src = "\
fn deserialize_rendered_html(s: String) -> RenderedHtml {
    Some(s).map(RenderedHtml::from_trusted).unwrap()
}
";
        assert!(violations(src).unwrap().is_empty());
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

    #[test]
    fn content_type_door_is_exempt_in_a_non_allowlisted_fn() {
        // `ContentType::from_trusted` (#584) is a different, non-HTML door — its
        // qualifier is exempt, so it is not flagged even outside ALLOWED_FNS.
        let src = "\
fn detect(name: &str) -> ContentType {
    ContentType::from_trusted(name)
}
";
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn content_type_map_reference_is_exempt() {
        let src = "\
fn pick(name: Option<&str>) -> Option<ContentType> {
    name.map(ContentType::from_trusted)
}
";
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn a_from_trusted_on_an_unrelated_type_is_still_flagged() {
        // The exemption is keyed to the qualifier: only listed types are skipped.
        // Any other `Type::from_trusted` (or a bare/aliased one) stays guarded.
        let src = "\
fn sneaky(raw: String) -> Widget {
    Widget::from_trusted(raw)
}
";
        assert_eq!(violations(src).unwrap(), vec![(2, "sneaky".to_string())]);
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
    fn a_nested_fn_shadowing_an_allowed_name_is_still_flagged() {
        // A nested fn shadowing an ALLOWED_FNS name must not borrow the top-level
        // entry's exemption. Must use a name that IS allowlisted, or the test passes
        // vacuously — it would be flagged for the ordinary reason.
        let src = "\
fn outer(raw: String) -> RenderedHtml {
    fn deserialize_rendered_html(raw: String) -> RenderedHtml {
        RenderedHtml::from_trusted(raw)
    }
    deserialize_rendered_html(raw)
}
";
        assert_eq!(
            violations(src).unwrap(),
            vec![(3, "deserialize_rendered_html".to_string())]
        );
    }

    #[test]
    fn a_cfg_not_test_production_fn_is_scanned() {
        // `#[cfg(not(test))]` is production-only; a door there must be flagged, not
        // exempted as if it were test code.
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
        // Not inside any fn — no enclosing fn, so never allowlisted.
        let src = "static X: () = { RenderedHtml::from_trusted(\"<p>x</p>\"); };\n";
        let v = violations(src).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].1, "");
    }

    #[test]
    fn the_definition_site_has_no_path_mention() {
        // `fn from_trusted` defines the door; its body does not *reference* the
        // `from_trusted` path, so it is not a hit.
        let src = "\
impl RenderedHtml {
    pub fn from_trusted(html: impl Into<String>) -> Self {
        Self(html.into())
    }
}
";
        assert!(violations(src).unwrap().is_empty());
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
    fn an_exempt_qualifier_inside_a_macro_body_is_still_exempt() {
        // The macro walk applies the same qualifier rule as the AST walk: a
        // recognised non-HTML door (#584) is not the thing this gate guards.
        let src = r#"
            fn detect(name: &str) -> AnyView {
                view! { <div data-kind=ContentType::from_trusted(name)></div> }.into_any()
            }
        "#;
        assert!(violations(src).unwrap().is_empty());
    }

    #[test]
    fn a_bare_from_trusted_inside_a_macro_body_is_flagged() {
        // No qualifier at all (a `use`-imported or aliased door) — the positional
        // qualifier read finds nothing to exempt, so the leaf stays guarded.
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

    #[test]
    fn problems_reports_file_line_and_recovery() {
        let scanned = vec![(
            "web/src/x.rs".to_string(),
            "fn sneaky(raw: String) -> RenderedHtml { RenderedHtml::from_trusted(raw) }\n"
                .to_string(),
        )];
        let detail = problems(&scanned).expect("a problem");
        assert!(detail.contains("web/src/x.rs:1"));
        assert!(detail.contains("not an allowlisted trusted-rebuild door"));
        assert!(detail.contains("ALLOWED_FNS"));
    }

    #[test]
    fn problems_is_none_for_allowlisted_and_test_sites() {
        let scanned = vec![
            (
                "common/src/render.rs".to_string(),
                "fn deserialize_rendered_html(s: String) -> RenderedHtml { RenderedHtml::from_trusted(s) }\n"
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
