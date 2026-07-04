//! Structural coverage exemption: parse a Rust source file with `syn` and return
//! the 1-based line numbers that are exempt from coverage because they sit inside
//! the body of a `#[component]` function. Leptos component bodies render only in
//! the browser (never natively exercised by the host test suite), so measuring
//! them is noise — but the recognition is deliberately **fail-closed**: an
//! unparseable file (or an unrecognized `#[component]` form) yields *no*
//! exemptions, leaving those lines measured so the gate can still FAIL. A missed
//! exemption is safe (over-measures); a false exemption would silently drop
//! coverage, so we never risk it.
//!
//! `#[component]`-ONLY: there is deliberately no standalone `view!` rule. A
//! `view!` inside a component is already covered by the fn-body span; a `view!`
//! elsewhere (e.g. `web/src/lib.rs`) must stay measured.

use std::collections::BTreeSet;

use syn::spanned::Spanned;

/// 1-based line numbers structurally exempt from coverage in `src`.
///
/// Returns `Err` if the file cannot be parsed — the caller treats a parse
/// failure as "nothing exempt" (fail-closed: lines stay measured → the gate can
/// FAIL, never silently exempt).
pub fn exempt_lines(src: &str) -> syn::Result<BTreeSet<u32>> {
    let file = syn::parse_file(src)?;
    let mut out = BTreeSet::new();
    let mut v = ExemptVisitor { out: &mut out };
    syn::visit::visit_file(&mut v, &file);
    Ok(out)
}

struct ExemptVisitor<'a> {
    out: &'a mut BTreeSet<u32>,
}

impl<'ast> syn::visit::Visit<'ast> for ExemptVisitor<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if has_component_attr(&f.attrs) {
            add_span(self.out, f.block.span()); // whole body exempt
        }
        syn::visit::visit_item_fn(self, f);
    }
    // NB: no `visit_macro` / standalone `view!` rule — a `view!` inside a
    // component is already inside `f.block.span()`; `view!` elsewhere stays
    // measured (see module docs).
}

/// Matches `#[component]` AND `#[component(...)]` — path-anchored, not a substring
/// scan, so an attribute like `#[my::component_thing]` does not falsely match.
fn has_component_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("component"))
}

/// Insert every 1-based line the span covers (inclusive) into `out`.
fn add_span(out: &mut BTreeSet<u32>, s: proc_macro2::Span) {
    for l in s.start().line..=s.end().line {
        out.insert(l as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exempts_plain_component_body() {
        let src = "\
#[component]
fn Foo() -> impl IntoView {
    let x = 1;
    x
}
";
        let ex = exempt_lines(src).unwrap();
        // Body braces span lines 2..=5; the interior statements must be exempt.
        assert!(ex.contains(&3), "let-line exempt: {ex:?}");
        assert!(ex.contains(&4), "expr-line exempt: {ex:?}");
    }

    #[test]
    fn exempts_component_with_args() {
        let src = "\
#[component(transparent)]
fn Bar() -> impl IntoView {
    let y = 2;
    y
}
";
        let ex = exempt_lines(src).unwrap();
        assert!(
            ex.contains(&3),
            "#[component(transparent)] body exempt: {ex:?}"
        );
    }

    #[test]
    fn exempts_view_inside_component() {
        let src = "\
#[component]
fn Baz() -> impl IntoView {
    view! { <div>hi</div> }
}
";
        let ex = exempt_lines(src).unwrap();
        // The `view!` line is inside the component body span → exempt.
        assert!(ex.contains(&3), "view! inside component exempt: {ex:?}");
    }

    #[test]
    fn does_not_exempt_view_in_plain_fn() {
        let src = "\
fn render() -> impl IntoView {
    view! { <div>hi</div> }
}
";
        let ex = exempt_lines(src).unwrap();
        // A standalone view! in a non-component fn must stay measured.
        assert!(
            ex.is_empty(),
            "no #[component] → nothing exempt, incl. the view! line: {ex:?}"
        );
    }

    #[test]
    fn does_not_exempt_server_fn() {
        let src = "\
#[server]
async fn save() -> Result<(), ServerFnError> {
    Ok(())
}
";
        let ex = exempt_lines(src).unwrap();
        // #[server] is not #[component]; its body stays measured.
        assert!(ex.is_empty(), "#[server] body must stay measured: {ex:?}");
    }

    #[test]
    fn does_not_exempt_plain_fn() {
        let src = "\
fn add(a: i32, b: i32) -> i32 {
    let s = a + b;
    s
}
";
        let ex = exempt_lines(src).unwrap();
        assert!(ex.is_empty(), "plain fn body stays measured: {ex:?}");
    }

    #[test]
    fn component_body_maps_to_correct_line_range_not_zero() {
        // CANARY: proves proc-macro2 `span-locations` is enabled. Without that
        // feature, `Span::start()/end()` return line 0 and this fixture would map
        // to {0}, not the real body lines.
        let src = "\
#[component]
fn Foo() -> impl IntoView {
    let x = 1;
    x
}
";
        let ex = exempt_lines(src).unwrap();
        // Line 1 = `#[component]`; the block `{`..`}` spans lines 2..=5.
        assert_eq!(
            ex,
            BTreeSet::from([2, 3, 4, 5]),
            "component body must map to its ACTUAL 2..=5 line range, not 0: {ex:?}"
        );
        assert!(!ex.contains(&0), "span-locations disabled → line 0 leaked");
    }

    #[test]
    fn parse_error_yields_empty() {
        // Unparseable source → Err; the caller treats Err as "nothing exempt"
        // (fail-closed), so the offending file's lines stay measured.
        let src = "fn broken( {{{ this is not valid rust";
        assert!(
            exempt_lines(src).is_err(),
            "an unparseable file must return Err (fail-closed)"
        );
    }
}
