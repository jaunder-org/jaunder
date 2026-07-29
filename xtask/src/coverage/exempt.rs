//! Structural coverage exemption: parse a Rust source file with `syn` and return
//! the 1-based line numbers that are exempt from coverage. One construct is
//! recognized:
//!
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
//! **Retired in #520: the `#[component]` / `#[client_only]` structural exemption**
//! (ADR-0050 Decision 1). It is not merely unused — it is unnecessary. Every
//! `#[component]` now lives in a wasm-only `component.rs` behind
//! `#[cfg(target_arch = "wasm32")] mod component;` (ADR-0070), so component lines
//! never enter the host denominator at all: not-compiled beats measured-but-exempt.
//! `macros::client_only` is deleted. A test below pins the retirement, so an
//! accidental reintroduction of the attribute rule fails loudly.

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
    /// A literal `unreachable!(<non-empty message>)` invocation is dropped from
    /// the executable set — self-enforcing (reaching it panics ⇒ the test fails ⇒
    /// `cargo llvm-cov` exits non-zero ⇒ no report), message-required (bare
    /// `unreachable!()` stays measured, forcing an explicit reason), and
    /// fail-closed (`std::unreachable!`, aliases, and macro-generated forms are
    /// not `is_ident("unreachable")` → they stay measured).
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("unreachable") && !mac.tokens.is_empty() {
            add_span(self.out, mac.span()); // path + bang + delimiters
            add_span(self.out, mac.tokens.span()); // the (possibly multi-line) message
        }
        syn::visit::visit_macro(self, mac);
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
    fn does_not_exempt_component_or_client_only_marked_fns() {
        // Pins the #520 retirement (ADR-0050 Decision 1). Components are wasm-only
        // (ADR-0070), so their lines never reach the host denominator and need no
        // exemption; `macros::client_only` no longer exists. Reintroducing either
        // attribute rule must fail here rather than silently discarding coverage.
        let src = "\
#[component]
fn Thing() -> impl IntoView {
    let x = 1;
    view! { <p>{x}</p> }
}

#[client_only]
fn helper() -> u32 {
    7
}
";
        let ex = exempt_lines(src).unwrap();
        assert!(
            ex.is_empty(),
            "neither #[component] nor #[client_only] may exempt anything: {ex:?}"
        );
    }
}
