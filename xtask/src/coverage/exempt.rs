//! Structural coverage exemption: parse a Rust source file with `syn` and return
//! the 1-based line numbers that are exempt from coverage. Two constructs are
//! recognized:
//!
//! - the body of a `#[component]` function — Leptos component bodies render only
//!   in the browser (never natively exercised by the host test suite), so
//!   measuring them is noise; and
//! - a literal `unreachable!(<message>)` invocation — a provably-dead line whose
//!   exemption is *self-enforcing*: reaching it panics ⇒ the test fails ⇒
//!   `cargo llvm-cov` exits non-zero ⇒ no report. A message **argument** is
//!   required (recognition is `!mac.tokens.is_empty()`), so a bare
//!   `unreachable!()` stays measured — mirroring the spirit of `crap:allow`'s
//!   required reason. (The token check does not inspect the message text, so a
//!   deliberately-empty `unreachable!("")` is still exempted; this degenerate
//!   form is not worth the fragile format-arg parsing to reject.)
//!
//! Recognition is deliberately **fail-closed**: an unparseable file (or an
//! unrecognized form — `std::unreachable!`, aliases, macro-generated invocations)
//! yields *no* exemption, leaving those lines measured so the gate can still FAIL.
//! A missed exemption is safe (over-measures); a false exemption would silently
//! drop coverage, so we never risk it.
//!
//! There is deliberately no standalone `view!` rule: a `view!` inside a component
//! is already covered by the fn-body span; a `view!` elsewhere (e.g.
//! `web/src/lib.rs`) must stay measured.

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
    /// A literal `unreachable!(<non-empty message>)` invocation is dropped from
    /// the executable set — self-enforcing (reaching it panics ⇒ the test fails ⇒
    /// `cargo llvm-cov` exits non-zero ⇒ no report), message-required (bare
    /// `unreachable!()` stays measured, forcing an explicit reason), and
    /// fail-closed (`std::unreachable!`, aliases, and macro-generated forms are
    /// not `is_ident("unreachable")` → they stay measured).
    ///
    /// NB: no standalone `view!` rule — a `view!` inside a component is already
    /// inside `f.block.span()`; `view!` elsewhere stays measured (see module docs).
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("unreachable") && !mac.tokens.is_empty() {
            add_span(self.out, mac.span()); // path + bang + delimiters
            add_span(self.out, mac.tokens.span()); // the (possibly multi-line) message
        }
        syn::visit::visit_macro(self, mac);
    }
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
    fn exempts_unreachable_with_message() {
        let src = "\
fn pick(n: u8) -> u8 {
    match n {
        0 => 1,
        _ => unreachable!(\"caller guarantees n == 0\"),
    }
}
";
        let ex = exempt_lines(src).unwrap();
        // The `unreachable!(\"...\")` line (line 4) must be exempt.
        assert!(ex.contains(&4), "unreachable! with message exempt: {ex:?}");
    }

    #[test]
    fn does_not_exempt_bare_unreachable() {
        let src = "\
fn pick(n: u8) -> u8 {
    match n {
        0 => 1,
        _ => unreachable!(),
    }
}
";
        let ex = exempt_lines(src).unwrap();
        // Message-required: bare unreachable!() stays measured.
        assert!(
            !ex.contains(&4),
            "bare unreachable!() must stay measured: {ex:?}"
        );
    }

    #[test]
    fn does_not_exempt_panic_or_todo() {
        let src = "\
fn a() { panic!(\"boom\"); }
fn b() { todo!(); }
fn c() { unimplemented!(\"later\"); }
";
        let ex = exempt_lines(src).unwrap();
        // panic!/todo!/unimplemented! are NOT unreachable! — stay measured.
        assert!(
            ex.is_empty(),
            "panic!/todo!/unimplemented! stay measured: {ex:?}"
        );
    }

    #[test]
    fn exempts_multiline_unreachable_message_span() {
        let src = "\
fn pick(n: u8) -> u8 {
    match n {
        0 => 1,
        _ => unreachable!(
            \"caller guarantees n == 0 for this arm\",
        ),
    }
}
";
        let ex = exempt_lines(src).unwrap();
        // Every line of the multi-line invocation (4..=6) must be exempt.
        assert!(ex.contains(&4), "macro-open line exempt: {ex:?}");
        assert!(ex.contains(&5), "message line exempt: {ex:?}");
        assert!(ex.contains(&6), "macro-close line exempt: {ex:?}");
    }

    #[test]
    fn does_not_exempt_std_unreachable() {
        let src = "\
fn pick(n: u8) -> u8 {
    match n {
        0 => 1,
        _ => std::unreachable!(\"path-qualified\"),
    }
}
";
        let ex = exempt_lines(src).unwrap();
        // Fail-closed boundary: only the single-segment literal matches.
        assert!(
            !ex.contains(&4),
            "std::unreachable! must stay measured: {ex:?}"
        );
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
