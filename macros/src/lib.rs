//! Workspace proc-macros: a target-agnostic, host-compiled build-time crate — the home
//! for the workspace's proc-macros — distinct from the `common`/`host`/`client` runtime
//! trio.

use proc_macro::TokenStream;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

mod id_newtype;
mod num_newtype;
mod str_enum;
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

/// Derives the ADR-0063 **numeric-value trailer** for a `struct X(I)` over an integer `I`
/// with a declarative bound. Unlike `StrNewtype` (whose `FromStr` is hand-written) and
/// `IdNewtype` (which enforces no value invariant), a numeric bound is declarative, so this
/// derive *generates* the whole trailer from `#[num_newtype(...)]`: a self-contained error
/// type, `value()`, a validating `FromStr`, `Display`, an optional compile-checked `Default`,
/// and a validating transparent-integer serde bridge (out-of-range rejected on the wire).
/// The std `#[derive]`s (`Clone`/`Copy`/`Debug`/`PartialEq`/`Eq`/`Hash`/`Ord`) stay in the
/// user's list.
///
/// Options: `inner = <ty>` (**required**, the wrapped integer type; the tuple field must be
/// exactly this type), `min` / `max` (inclusive bounds, each optional — the check is emitted
/// only for a declared side), `default = <int>` (generates a `Default` guarded so an
/// out-of-range default is a compile error), `error = "…"` (overrides the generated
/// `Display` message), and `clamp` (a bare flag, requires both `min` and `max`: emits
/// `MIN`/`MAX` consts and an infallible `const fn clamped(inner) -> Self` coercing into range).
///
/// ```
/// use macros::NumNewtype;
/// use std::str::FromStr;
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, NumNewtype)]
/// #[num_newtype(inner = u32, min = 1, default = 20)]
/// struct MinItems(u32);
///
/// assert_eq!("7".parse::<MinItems>().unwrap().value(), 7);
/// assert!("0".parse::<MinItems>().is_err());          // below `min`
/// assert_eq!(MinItems::default().value(), 20);        // compile-checked default
/// assert_eq!(u32::from(MinItems::default()), 20);     // From<Self> for the inner
/// assert_eq!(serde_json::to_string(&MinItems::default()).unwrap(), "20"); // bare integer
/// assert!(serde_json::from_str::<MinItems>("0").is_err());                // wire rejection
/// ```
///
/// Applying the derive to anything but a single-field tuple struct is a compile error:
///
/// ```compile_fail
/// use macros::NumNewtype;
/// #[derive(NumNewtype)]
/// #[num_newtype(inner = u32)]
/// struct NotATuple { n: u32 }
/// ```
#[proc_macro_derive(NumNewtype, attributes(num_newtype))]
pub fn num_newtype_derive(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    num_newtype::expand(&input).into()
}

/// Derives the **string-enum trailer** for a unit-variant enum whose values are a fixed set
/// of lowercase wire/DB tokens — the closed-set analogue of the newtype derives. It generates
/// `as_str`, `Display`, `FromStr` **and** `TryFrom<&str>` (routed through a self-contained
/// `Invalid<Name>` error, no `thiserror`), and — under `#[str_enum(serde)]` — a string serde
/// bridge (serialize the token; deserialize an owned `String` via `FromStr`). Because the
/// valid set is known to the macro, it *generates* `FromStr` and the named error (unlike
/// `StrNewtype`, which routes through a hand-written `FromStr`). See the str-enum-trailer ADR.
///
/// Each variant's token defaults to the `snake_case` of its identifier (`InviteOnly` ->
/// `invite_only`); `#[str_enum(rename = "…")]` overrides it. `#[str_enum(error = "…")]`
/// (type-level) overrides the generated
/// `"must be one of: …"` message. `Default` (std `#[derive(Default)]` + `#[default]`),
/// `Copy`/`Hash`/… stay in the user's `#[derive]` list.
///
/// ```
/// use macros::StrEnum;
/// use std::str::FromStr;
/// #[derive(Clone, Copy, PartialEq, Eq, Debug, Default, StrEnum)]
/// #[str_enum(serde)]
/// enum Fmt { #[default] Markdown, Org, Html }
///
/// assert_eq!(Fmt::Org.as_str(), "org");
/// assert_eq!("html".parse::<Fmt>().unwrap(), Fmt::Html);
/// assert!("xml".parse::<Fmt>().is_err());
/// assert_eq!(Fmt::default(), Fmt::Markdown);
/// assert_eq!(serde_json::to_string(&Fmt::Org).unwrap(), "\"org\"");
/// let _back: Fmt = serde_json::from_str("\"org\"").unwrap();
/// ```
///
/// Applying the derive to anything but a unit-variant enum is a compile error:
///
/// ```compile_fail
/// use macros::StrEnum;
/// #[derive(StrEnum)]
/// struct NotAnEnum(String);
/// ```
#[proc_macro_derive(StrEnum, attributes(str_enum))]
pub fn str_enum_derive(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    str_enum::expand(&input).into()
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

    #[test]
    fn num_newtype_wrong_shape_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32)]
            struct X { a: u32 }
        };
        assert!(num_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn num_newtype_missing_inner_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(min = 1)]
            struct X(u32);
        };
        assert!(num_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn num_newtype_inner_type_mismatch_emits_compile_error() {
        // Field is `u32` but `inner = i64` — the declared inner disagrees with the field.
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = i64, min = 1)]
            struct X(u32);
        };
        assert!(num_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn num_newtype_unknown_option_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, bogus = 1)]
            struct X(u32);
        };
        assert!(num_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn num_newtype_min_max_default_emit_full_trailer() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, max = 100, default = 20)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("FromStr"));
        // The validating `TryFrom<inner>` door (the checked integer constructor) is emitted,
        // reusing the same bound guards as `FromStr`.
        assert!(out.contains("TryFrom"));
        assert!(out.contains("Default"));
        assert!(out.contains("Serialize"));
        assert!(out.contains("Deserialize"));
        // Both bound checks present — `v < min` and `v > max` (the `v` prefix is unique to
        // the generated checks; a bare `> ` also occurs in generics).
        assert!(out.contains("v < 1"));
        assert!(out.contains("v > 100"));
    }

    #[test]
    fn num_newtype_min_only_omits_max_check_and_default() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = usize, min = 1)]
            struct X(usize);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("v < 1"));
        // No `max` side (`v > `), no `default` impl.
        assert!(!out.contains("v > "));
        assert!(!out.contains("impl :: core :: default :: Default"));
    }

    #[test]
    fn num_newtype_error_message_overrides_generated() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, error = "must be a whole number of at least 1")]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(out.contains("must be a whole number of at least 1"));
    }

    #[test]
    fn num_newtype_max_only_emits_max_check_and_at_most_message() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, max = 100)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("v > 100"));
        assert!(!out.contains("v < ")); // no `min` side
        assert!(out.contains("at most 100"));
    }

    #[test]
    fn num_newtype_no_bounds_generates_valid_integer_message() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("a valid integer"));
        assert!(!out.contains("v < "));
        assert!(!out.contains("v > "));
    }

    /// True iff the emitted stream carries the three sqlx bridge impls.
    fn has_sqlx_bridge(out: &str) -> bool {
        out.contains("sqlx :: Type")
            && out.contains("sqlx :: Encode")
            && out.contains("sqlx :: Decode")
    }

    #[test]
    fn str_newtype_default_emits_sqlx_bridge() {
        // Default (non-secret) type: the validating sqlx bridge is on, feature-gated,
        // and routes Decode through FromStr.
        let input: DeriveInput = parse_quote! {
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(has_sqlx_bridge(&out));
        assert!(out.contains("cfg (feature = \"sqlx\")"));
        assert!(out.contains("from_str"));
    }

    #[test]
    fn str_newtype_no_sqlx_omits_the_bridge() {
        // `no_sqlx` opts a non-secret must-not-store type (RawToken) out of the bridge.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(no_sqlx)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(!has_sqlx_bridge(&out));
        // The rest of the default trailer is still there.
        assert!(out.contains("Serialize"));
    }

    #[test]
    fn str_newtype_secret_omits_the_bridge() {
        // A secret is bridge-less by default (must not be storable).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(!has_sqlx_bridge(&out));
    }

    #[test]
    fn str_newtype_secret_sqlx_readds_the_bridge() {
        // `secret, sqlx`: the redacting trailer plus the validating sqlx bridge
        // (InviteCode — a stored secret).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret, sqlx)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(out.contains("redacted"));
        assert!(has_sqlx_bridge(&out));
        assert!(out.contains("from_str"));
    }

    #[test]
    fn str_newtype_infallible_emits_the_infallible_sqlx_bridge() {
        // Infallible types are stored: the bridge is on by default and Decode wraps via
        // From<String> (no FromStr).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(has_sqlx_bridge(&out));
        assert!(!out.contains("from_str"));
    }

    #[test]
    fn str_newtype_infallible_no_sqlx_omits_the_bridge() {
        let input: DeriveInput = parse_quote! {
            #[str_newtype(infallible, no_sqlx)]
            struct X(String);
        };
        let out = str_newtype::expand(&input).to_string();
        assert!(!has_sqlx_bridge(&out));
    }

    #[test]
    fn str_newtype_no_sqlx_with_secret_emits_compile_error() {
        // A secret is already bridge-less — `no_sqlx` is redundant/invalid.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(secret, no_sqlx)]
            struct X(String);
        };
        assert!(str_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_newtype_no_sqlx_with_sqlx_emits_compile_error() {
        // `sqlx, no_sqlx` (no `secret`, so the `no_sqlx && secret` guard is skipped and
        // the `no_sqlx && sqlx` exclusivity arm fires).
        let input: DeriveInput = parse_quote! {
            #[str_newtype(sqlx, no_sqlx)]
            struct X(String);
        };
        assert!(str_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_newtype_bare_sqlx_without_secret_emits_compile_error() {
        // Bare `sqlx` is only meaningful on a secret; non-secret types get the bridge
        // by default.
        let input: DeriveInput = parse_quote! {
            #[str_newtype(sqlx)]
            struct X(String);
        };
        assert!(str_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn num_newtype_clamp_emits_bounds_and_clamped_constructor() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, max = 50, default = 50, clamp)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("const MIN"));
        assert!(out.contains("const MAX"));
        assert!(out.contains("fn clamped"));
    }

    #[test]
    fn num_newtype_clamp_without_both_bounds_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, clamp)]
            struct X(u32);
        };
        assert!(num_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn num_newtype_min_greater_than_max_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 50, max = 1)]
            struct X(u32);
        };
        assert!(num_newtype::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn num_newtype_without_clamp_omits_clamped_constructor() {
        let input: DeriveInput = parse_quote! {
            #[num_newtype(inner = u32, min = 1, max = 50)]
            struct X(u32);
        };
        let out = num_newtype::expand(&input).to_string();
        assert!(!out.contains("fn clamped"));
        assert!(!out.contains("const MAX"));
    }

    // StrEnum — positive branches (drive `expand`'s codegen at runtime; the `macros/tests`
    // integration enums expand at *compile time*, invisible to coverage).

    #[test]
    fn str_enum_serde_emits_bridge_and_lowercase_wire() {
        let input: DeriveInput = parse_quote! {
            #[str_enum(serde)]
            enum Fmt { Markdown, Org, Html }
        };
        let out = str_enum::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("fn as_str"));
        assert!(out.contains("FromStr"));
        assert!(out.contains("TryFrom"));
        assert!(out.contains("Serialize"));
        assert!(out.contains("Deserialize"));
        // Lowercased-identifier wire tokens.
        assert!(out.contains("\"markdown\""));
        assert!(out.contains("\"org\""));
        assert!(out.contains("\"html\""));
    }

    #[test]
    fn str_enum_without_serde_omits_bridge() {
        let input: DeriveInput = parse_quote! {
            enum Kind { Public, Subscribers, Named }
        };
        let out = str_enum::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("FromStr"));
        assert!(!out.contains("Serialize"));
    }

    #[test]
    fn str_enum_rename_overrides_lowercase() {
        let input: DeriveInput = parse_quote! {
            enum X { Bar, #[str_enum(rename = "zee")] Z }
        };
        let out = str_enum::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        assert!(out.contains("\"bar\"")); // default lowercase
        assert!(out.contains("\"zee\"")); // rename override
    }

    #[test]
    fn str_enum_multiword_variant_uses_snake_case() {
        let input: DeriveInput = parse_quote! {
            enum Policy { Open, InviteOnly, Closed }
        };
        let out = str_enum::expand(&input).to_string();
        assert!(!out.contains("compile_error"));
        // CamelCase -> snake_case, not the concatenated-lowercase "inviteonly".
        assert!(out.contains("\"invite_only\""));
        assert!(!out.contains("\"inviteonly\""));
        assert!(out.contains("\"open\""));
    }

    #[test]
    fn str_enum_auto_message_lists_variants() {
        let input: DeriveInput = parse_quote! {
            #[str_enum(serde)]
            enum Fmt { Markdown, Org, Html }
        };
        let out = str_enum::expand(&input).to_string();
        assert!(out.contains("must be one of: markdown, org, html"));
    }

    #[test]
    fn str_enum_error_override_replaces_auto_message() {
        let input: DeriveInput = parse_quote! {
            #[str_enum(error = "bad fmt")]
            enum Fmt { Markdown, Org }
        };
        let out = str_enum::expand(&input).to_string();
        assert!(out.contains("bad fmt"));
        assert!(!out.contains("must be one of"));
    }

    // StrEnum — error branches (each returns a spanned `compile_error!`).

    #[test]
    fn str_enum_on_struct_emits_compile_error() {
        let input: DeriveInput = parse_quote! { struct X(String); };
        assert!(str_enum::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_enum_generic_enum_emits_compile_error() {
        let input: DeriveInput = parse_quote! { enum X<T> { A(T) } };
        assert!(str_enum::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_enum_empty_enum_emits_compile_error() {
        let input: DeriveInput = parse_quote! { enum X {} };
        assert!(str_enum::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_enum_fielded_variant_emits_compile_error() {
        let input: DeriveInput = parse_quote! { enum X { A, B(u32) } };
        assert!(str_enum::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_enum_duplicate_wire_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            enum X { A, #[str_enum(rename = "a")] B }
        };
        assert!(str_enum::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_enum_unknown_type_option_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            #[str_enum(bogus)]
            enum X { A }
        };
        assert!(str_enum::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_enum_unknown_variant_option_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            enum X { #[str_enum(bogus = "x")] A }
        };
        assert!(str_enum::expand(&input)
            .to_string()
            .contains("compile_error"));
    }

    #[test]
    fn str_enum_empty_wire_token_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            enum X { #[str_enum(rename = "")] A }
        };
        assert!(str_enum::expand(&input)
            .to_string()
            .contains("compile_error"));
    }
}
