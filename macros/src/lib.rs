//! Workspace proc-macros: a target-agnostic, host-compiled build-time crate — the home
//! for the workspace's proc-macros — distinct from the `common`/`host`/`client` runtime
//! trio.

use proc_macro::TokenStream;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

mod id_newtype;
mod str_newtype;

/// Marks a **client-only reactive helper**: code that runs only in the browser (a
/// `server_resource` fetch, or an `Effect` that fires only client-side) and is exercised
/// by e2e, not host tests. It is an **identity** attribute — it expands to the annotated
/// item unchanged. Its sole purpose is to be a syntactic marker the coverage framework
/// (`xtask/src/coverage/exempt.rs`) recognizes and exempts, generalizing the `#[component]`
/// rule to non-component helpers (a macro-backed peer of the `cov:ignore` comment marker).
///
/// Interim until wasm-bindgen-test can cover these in a headless browser (Test-infra epic).
#[proc_macro_attribute]
pub fn client_only(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Derives the ADR-0063 **string-newtype trailer** for a `struct X(String)`: `Display`,
/// a serde bridge (deserialize routed through `FromStr`, so invalid input is rejected on
/// the wire), `AsRef`/`Borrow`/`Deref<str>`, `TryFrom<String>`, `From<Self> for String`,
/// and `PartialEq<str>`/`<&str>`. `FromStr` stays hand-written — it is the single
/// validating/normalizing chokepoint — as do the std `#[derive]`s.
///
/// `#[str_newtype(secret)]` selects the tight secret surface (redacting `Debug`,
/// `AsRef` + `TryFrom` only; no `Display`/serde/`Deref`/`Borrow`/owned-`String`/`PartialEq`).
/// `#[str_newtype(secret, serde)]` adds the validating serde bridge back onto that surface
/// for a secret that must cross the wire *inbound* (client→server) — still no `Display`/`Deref`:
///
/// ```
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret, serde)]
/// # struct Wire(String);
/// # impl FromStr for Wire { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Wire(s.to_owned())) } }
/// let w = Wire("x".to_owned());
/// assert_eq!(serde_json::to_string(&w).unwrap(), "\"x\""); // serde is back
/// let _back: Wire = serde_json::from_str("\"x\"").unwrap();
/// ```
///
/// Applying the derive to anything but a single-field tuple struct is a compile error:
///
/// ```compile_fail
/// use macros::StrNewtype;
/// #[derive(StrNewtype)]
/// struct NotATuple { s: String }
/// ```
///
/// A single-field tuple struct with a hand-written `FromStr` compiles:
///
/// ```
/// use macros::StrNewtype;
/// use std::str::FromStr;
/// #[derive(Clone, StrNewtype)]
/// struct Ok1(String);
/// impl FromStr for Ok1 {
///     type Err = std::convert::Infallible;
///     fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Ok1(s.to_owned())) }
/// }
/// ```
///
/// The **secret** surface omits `Display`, serde, owned-`String` extraction, value
/// `PartialEq`, and `Deref` coercion. The positive companion shows the identical fixture
/// compiles — and that `serde_json` resolves, so the serde `compile_fail` below fails for
/// the missing `Serialize`, not an unresolved crate. (Fixture lines are hidden with `#`.)
///
/// ```
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// let s = Sec("x".to_owned());
/// let _read: &str = s.as_ref();               // explicit borrowed read is allowed
/// let _ = serde_json::to_string(s.as_ref());  // serde_json resolves (a &str serializes)
/// ```
///
/// No `Display`:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _ = format!("{}", s);
/// ```
///
/// No serde:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _ = serde_json::to_string(&s);
/// ```
///
/// No owned-`String` extraction:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _ = String::from(s);
/// ```
///
/// No value `PartialEq`:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _ = s == "x";
/// ```
///
/// No `Deref` coercion to `&str`:
/// ```compile_fail
/// # use macros::StrNewtype;
/// # use std::str::FromStr;
/// # #[derive(Clone, StrNewtype)]
/// # #[str_newtype(secret)]
/// # struct Sec(String);
/// # impl FromStr for Sec { type Err = std::convert::Infallible; fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Sec(s.to_owned())) } }
/// # let s = Sec("x".to_owned());
/// let _: &str = &s;
/// ```
///
/// `#[str_newtype(infallible)]` selects the **infallible** trailer for a newtype whose
/// invariant never rejects: construction is a hand-written `From<String>` (the single
/// pure-wrap or normalizing chokepoint) rather than `FromStr`, so there is no
/// `TryFrom<String>`/`FromStr`. The derive also emits a `From<&str>` alias that routes
/// through that `From<String>` — so a literal constructs with one `.into()`, no
/// `.to_owned()` — and a `Deserialize` that routes wire input through it too, so it cannot
/// fail and normalizes identically. Exclusive with `secret`/`serde` (the infallible
/// trailer already includes the serde bridge):
///
/// ```
/// use macros::StrNewtype;
/// #[derive(Clone, StrNewtype)]
/// #[str_newtype(infallible)]
/// struct Inf(String);
/// impl From<String> for Inf {
///     fn from(s: String) -> Self { Inf(s) }
/// }
/// let v: Inf = "x".into();                                        // From<&str>, one hop
/// assert_eq!(serde_json::to_string(&v).unwrap(), "\"x\"");        // serde bridge
/// let _back: Inf = serde_json::from_str("\"x\"").unwrap();        // deserialize via From<String>
/// ```
#[proc_macro_derive(StrNewtype, attributes(str_newtype))]
pub fn str_newtype_derive(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    str_newtype::expand(&input).into()
}

/// Derives the ADR-0063 **numeric-ID trailer** for a `struct X(i64)`: `From<i64>`,
/// `From<Self> for i64`, `Display`, and a transparent-i64 serde bridge (wire form is a
/// bare integer). `Copy` and the other std traits stay in the user's `#[derive]` list.
///
/// Applying the derive to anything but a single-field tuple struct is a compile error:
///
/// ```compile_fail
/// use macros::IdNewtype;
/// #[derive(IdNewtype)]
/// struct NotATuple { n: i64 }
/// ```
///
/// A single-field tuple struct compiles:
///
/// ```
/// use macros::IdNewtype;
/// #[derive(Clone, Copy, IdNewtype)]
/// struct Id(i64);
/// ```
#[proc_macro_derive(IdNewtype)]
pub fn id_newtype_derive(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    id_newtype::expand(&input).into()
}

/// Validates that `input` is a **non-generic** single-field tuple struct (`struct X(T)`) —
/// the shape both newtype derives require — returning a spanned error (rendered as
/// `compile_error!`) otherwise. `macro_name`/`example` shape the diagnostic. Generics are
/// rejected here rather than silently mis-handled: the derives emit `impl … for #name`
/// with no `impl_generics`/`where_clause`, so a generic newtype would otherwise produce a
/// confusing "missing generics" error at the user's site instead of this clear one.
pub(crate) fn require_newtype_shape(
    input: &DeriveInput,
    macro_name: &str,
    example: &str,
) -> syn::Result<()> {
    let single_field_tuple = matches!(
        &input.data,
        Data::Struct(s) if matches!(&s.fields, Fields::Unnamed(f) if f.unnamed.len() == 1),
    );
    let non_generic = input.generics.params.is_empty() && input.generics.where_clause.is_none();
    if single_field_tuple && non_generic {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            input,
            format!(
                "{macro_name} requires a non-generic single-field tuple struct like `{example}`"
            ),
        ))
    }
}

// The derives' *error* paths (wrong shape, unknown `str_newtype` option) can only be
// reached through a compile error, which the `compile_fail` doctests exercise — but
// doctest compilation is invisible to coverage instrumentation. These unit tests drive
// the same branches at runtime by calling the codegen entry points directly with
// malformed input and asserting a `compile_error!` is emitted.
#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn require_newtype_shape_rejects_named_struct() {
        let input: DeriveInput = parse_quote! { struct X { a: String } };
        assert!(require_newtype_shape(&input, "StrNewtype", "struct X(String)").is_err());
    }

    #[test]
    fn require_newtype_shape_rejects_generic_struct() {
        let input: DeriveInput = parse_quote! { struct X<T>(T); };
        assert!(require_newtype_shape(&input, "StrNewtype", "struct X(String)").is_err());
    }

    #[test]
    fn require_newtype_shape_accepts_tuple_struct() {
        let input: DeriveInput = parse_quote! { struct X(String); };
        assert!(require_newtype_shape(&input, "StrNewtype", "struct X(String)").is_ok());
    }

    #[test]
    fn str_newtype_wrong_shape_emits_compile_error() {
        let input: DeriveInput = parse_quote! { struct X { a: String } };
        assert!(str_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_newtype_unknown_option_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(bogus)]
            struct X(String);
        };
        assert!(str_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_newtype_secret_selects_redacting_trailer() {
        // Drives `parse_opts`'s success path and the secret branch of `expand`: a redacting
        // Debug is emitted and the serde bridge is not.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("redacted"));
        assert!(!out.contains("Serialize"));
    }

    #[test]
    fn str_newtype_secret_serde_adds_the_serde_bridge_to_the_redacting_trailer() {
        // `secret, serde`: the redacting Debug AND the serde bridge, but not the full
        // trailer (no Display).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret, serde)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("redacted"));
        assert!(out.contains("Serialize"));
        assert!(!out.contains("Display"));
    }

    #[test]
    fn str_newtype_serde_without_secret_emits_compile_error() {
        // A bare `serde` is invalid — the default trailer already has the serde bridge.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(serde)]
            struct X(String);
        };
        assert!(str_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_newtype_infallible_emits_from_string_serde_and_omits_fallible_door() {
        // Infallible mode: Display/AsRef/Deref/Serialize/Deserialize present; the
        // fallible door (TryFrom / FromStr routing) is absent — the author writes
        // From<String> and Deserialize routes through it.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("Display"));
        assert!(out.contains("AsRef"));
        assert!(out.contains("Deref"));
        assert!(out.contains("Serialize"));
        assert!(out.contains("Deserialize"));
        // The fallible door (TryFrom / FromStr routing) is absent — the author writes
        // From<String> and Deserialize routes through it.
        assert!(!out.contains("TryFrom"));
        assert!(!out.contains("FromStr"));
    }

    #[test]
    fn str_newtype_infallible_with_secret_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible, secret)]
            struct X(String);
        };
        assert!(str_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_newtype_infallible_with_serde_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible, serde)]
            struct X(String);
        };
        assert!(str_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn id_newtype_wrong_shape_emits_compile_error() {
        let input: DeriveInput = parse_quote! { struct X { a: i64 } };
        assert!(id_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }
}
