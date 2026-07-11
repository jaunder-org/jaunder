//! Structural coverage exemption: parse a Rust source file with `syn` and return
//! the 1-based line numbers that are exempt from coverage. Three constructs are
//! recognized:
//!
//! - a `#[component]` function, signature AND body — Leptos component bodies
//!   render only in the browser, and the `#[component]` macro generates a prop
//!   struct/builder from the parameter list whose code is attributed to the
//!   signature lines and is likewise never exercised host-side;
//! - a `#[client_only]` function OR method, signature AND body — client-only
//!   reactive helpers that aren't components (e.g. `web::reactive::Invalidator`'s
//!   `resource`/`action`, a `server_resource` fetch or a browser-only gating
//!   `Effect`), exercised by e2e, not host tests. It generalizes the `#[component]`
//!   rule to non-component helpers and to `impl` methods (`ImplItemFn`), via the
//!   `macros::client_only` identity attribute; and
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
        exempt_marked_fn(self.out, &f.attrs, f.sig.span(), f.block.span());
        syn::visit::visit_item_fn(self, f);
    }

    /// The client-only `web::reactive::Invalidator` helpers are `#[client_only]` METHODS
    /// (`ImplItemFn`), not free fns — this arm reaches them. (Components are always free
    /// functions, so `#[component]` never lands here; the shared helper keeps both rules
    /// byte-identical regardless.)
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        exempt_marked_fn(self.out, &f.attrs, f.sig.span(), f.block.span());
        syn::visit::visit_impl_item_fn(self, f);
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

/// Matches `#[component]`/`#[component(...)]` OR `#[client_only]`/`#[client_only(...)]`
/// — path-anchored, not a substring scan, so `#[my::component_thing]` /
/// `#[my::client_only_thing]` do not falsely match.
fn has_exempt_attr(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().is_ident("component") || a.path().is_ident("client_only"))
}

/// Exempt a whole marked fn/method — its signature span AND body span — when it carries
/// `#[component]` or `#[client_only]`. Shared by the free-fn (`ItemFn`) and method
/// (`ImplItemFn`) arms so the two rules stay byte-identical.
///
/// The SIGNATURE span is exempted, not just the body: for `#[component]` the macro
/// generates a prop struct/builder from the parameter list whose code is attributed back to
/// the signature lines and is likewise never exercised host-side — body-only exemption
/// forced hand-marking the prop list, a `cov:ignore` on a function declaration, exactly the
/// wrong shape (#245). `#[client_only]` gets the same treatment: its body is browser-only
/// and its signature lines are equally un-exercised host-side.
fn exempt_marked_fn(
    out: &mut BTreeSet<u32>,
    attrs: &[syn::Attribute],
    sig: proc_macro2::Span,
    block: proc_macro2::Span,
) {
    if has_exempt_attr(attrs) {
        add_span(out, sig); // signature + any macro-generated prop code
        add_span(out, block); // body
    }
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
    fn exempts_component_signature_prop_list() {
        // The `#[component]` macro generates prop struct/builder code attributed to
        // the multi-line parameter list; those signature lines must be exempt too,
        // not just the body — else the prop list needs hand-marking (#245).
        let src = "\
#[component]
pub fn Widget(
    label: String,
    count: u32,
) -> impl IntoView {
    view! { <span>{label}</span> }
}
";
        let ex = exempt_lines(src).unwrap();
        // Line 2 `pub fn Widget(`, 3 `label`, 4 `count` are signature lines.
        assert!(ex.contains(&2), "fn signature line exempt: {ex:?}");
        assert!(ex.contains(&3), "prop line exempt: {ex:?}");
        assert!(ex.contains(&4), "prop line exempt: {ex:?}");
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

    #[test]
    fn exempts_client_only_method() {
        // Client-only helpers are METHODS (ImplItemFn), not free fns — the visitor must
        // reach them via visit_impl_item_fn, exempting signature + body like #[component].
        let src = "\
struct S;
impl S {
    #[client_only]
    fn helper(&self) -> u32 {
        let x = 1;
        x
    }
}
";
        let ex = exempt_lines(src).unwrap();
        // Line 4 is the `fn helper(&self) -> u32 {` SIGNATURE line; the body braces span
        // 4..=7 with interior statements on 5, 6. All must be exempt — asserting the
        // signature line pins that the method arm exempts `sig.span()`, not just the body.
        assert!(
            ex.contains(&4),
            "client_only method signature exempt: {ex:?}"
        );
        assert!(ex.contains(&5), "client_only method body exempt: {ex:?}");
        assert!(ex.contains(&6), "client_only method body exempt: {ex:?}");
    }

    #[test]
    fn exempts_client_only_free_fn() {
        let src = "\
#[client_only]
fn helper() -> u32 {
    let x = 1;
    x
}
";
        let ex = exempt_lines(src).unwrap();
        assert!(ex.contains(&3), "client_only free-fn body exempt: {ex:?}");
        assert!(ex.contains(&4), "client_only free-fn body exempt: {ex:?}");
    }

    #[test]
    fn does_not_exempt_unmarked_method() {
        let src = "\
struct S;
impl S {
    fn helper(&self) -> u32 {
        let x = 1;
        x
    }
}
";
        let ex = exempt_lines(src).unwrap();
        assert!(ex.is_empty(), "unmarked method stays measured: {ex:?}");
    }

    #[test]
    fn does_not_exempt_non_ident_client_only_path() {
        // Path-anchored (is_ident), matching #[component] recognition: a multi-segment
        // path must NOT match, so the bare-ident marker is the only recognized form.
        let src = "\
#[foo::client_only]
fn helper() -> u32 {
    let x = 1;
    x
}
";
        let ex = exempt_lines(src).unwrap();
        assert!(ex.is_empty(), "#[foo::client_only] must not match: {ex:?}");
    }
}
