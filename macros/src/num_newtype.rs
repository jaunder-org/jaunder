//! Codegen for `#[derive(NumNewtype)]` — the ADR-0063 numeric-**value** trailer for a
//! `struct X(I)` over an integer `I`, with a declarative bound. Unlike `StrNewtype` (whose
//! `FromStr` is hand-written per type) and `IdNewtype` (which has no value invariant at
//! all), a numeric bound is declarative, so this derive *generates* the whole trailer from
//! `#[num_newtype(...)]` attributes: a validating `FromStr`, a `value()` accessor, `Display`,
//! an optional compile-checked `Default`, a validating transparent-integer serde bridge, an
//! optional `clamp` affordance (`MIN`/`MAX` + a coercing `clamped` constructor), and a
//! self-contained error type. The std `#[derive]`s stay in the user's list.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident, LitInt, LitStr, Type};

/// The parsed `#[num_newtype(...)]` options. `inner` is required; the bounds and `default`
/// are optional; `error` overrides the generated `Display` message.
struct Opts {
    inner: Type,
    min: Option<LitInt>,
    max: Option<LitInt>,
    default: Option<LitInt>,
    error: Option<LitStr>,
    /// Opt-in: emit `MIN`/`MAX` and a coercing `clamped` constructor. Requires both bounds.
    clamp: bool,
}

/// Expands `#[derive(NumNewtype)]` on a single-field tuple struct. On the wrong shape, a
/// missing/unknown option, a missing `inner`, or a tuple field whose type differs from
/// `inner`, it returns a spanned `compile_error!` instead of malformed impls.
pub(crate) fn expand(input: &DeriveInput) -> TokenStream {
    if let Err(e) = crate::require_newtype_shape(input, "NumNewtype", "struct X(u32)") {
        return e.to_compile_error();
    }
    let opts = match parse_opts(input) {
        Ok(o) => o,
        Err(e) => return e.to_compile_error(),
    };
    if let Err(e) = require_field_matches_inner(input, &opts.inner) {
        return e.to_compile_error();
    }
    // `clamp` needs both ends of the range to coerce into; a one-sided bound can't clamp.
    if opts.clamp && (opts.min.is_none() || opts.max.is_none()) {
        return syn::Error::new_spanned(
            &input.ident,
            "num_newtype `clamp` requires both `min` and `max`",
        )
        .to_compile_error();
    }
    // A `min > max` range is nonsensical: `FromStr`/serde would reject every value, and a
    // `clamped` door (if opted in) would coerce into an empty range and could return an
    // out-of-range value — so reject it at compile time and keep `clamped`'s invariant true.
    let bogus_range = opts
        .min
        .as_ref()
        .and_then(|m| m.base10_parse::<i128>().ok())
        .zip(
            opts.max
                .as_ref()
                .and_then(|m| m.base10_parse::<i128>().ok()),
        )
        .is_some_and(|(lo, hi)| lo > hi);
    if bogus_range {
        return syn::Error::new_spanned(&input.ident, "num_newtype `min` must not exceed `max`")
            .to_compile_error();
    }

    let name = &input.ident;
    let err_name = quote::format_ident!("Invalid{}", name);
    let error_ty = error_type(&err_name, &opts.message(name));
    let accessor = accessor(name, &opts.inner);
    let from_str = from_str_impl(name, &err_name, &opts);
    let display = display_impl(name);
    let default_impl = default_impl(name, &opts);
    let serde = serde_impl(name, &err_name, &opts);
    let clamped = clamped_impl(name, &opts);

    quote! {
        #error_ty
        #accessor
        #from_str
        #display
        #default_impl
        #serde
        #clamped
    }
}

/// The self-contained error type: a hand-written `Display` + `Error` (no `thiserror`), so
/// any adopter crate needs no extra dependency.
fn error_type(err_name: &Ident, message: &TokenStream) -> TokenStream {
    quote! {
        #[doc = "Error returned when a value is out of the declared numeric bounds."]
        #[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy, ::core::cmp::PartialEq, ::core::cmp::Eq)]
        pub struct #err_name;

        #[automatically_derived]
        impl ::core::fmt::Display for #err_name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(#message)
            }
        }

        #[automatically_derived]
        impl ::std::error::Error for #err_name {}
    }
}

/// `impl X { pub fn value(self) -> I }` plus `From<X> for I` — the inner accessor and its
/// idiomatic conversion (inners here are `Copy`). Only the extraction direction is emitted;
/// a `From<I> for X` would be an unchecked constructor that bypasses the bound.
fn accessor(name: &Ident, inner: &Type) -> TokenStream {
    quote! {
        impl #name {
            #[doc = "The inner value (within the declared bounds)."]
            #[must_use]
            pub fn value(self) -> #inner {
                self.0
            }
        }

        // Idiomatic extraction, mirroring `IdNewtype`'s `From<Self> for i64`, so a caller can
        // write `#inner::from(x)` / `x.into()` where that reads better than `x.value()`.
        #[automatically_derived]
        impl ::core::convert::From<#name> for #inner {
            fn from(v: #name) -> Self {
                v.0
            }
        }
    }
}

/// The single validating chokepoint: trim, parse the inner integer, then apply the declared
/// bound(s). The inverse of `Display`.
fn from_str_impl(name: &Ident, err_name: &Ident, opts: &Opts) -> TokenStream {
    let inner = &opts.inner;
    let min = opts.min.as_ref().map(|m| {
        quote! { if v < #m { return ::core::result::Result::Err(#err_name); } }
    });
    let max = opts.max.as_ref().map(|m| {
        quote! { if v > #m { return ::core::result::Result::Err(#err_name); } }
    });
    quote! {
        #[automatically_derived]
        impl ::core::str::FromStr for #name {
            type Err = #err_name;
            fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {
                let v: #inner = s.trim().parse::<#inner>().map_err(|_| #err_name)?;
                #min
                #max
                ::core::result::Result::Ok(#name(v))
            }
        }
    }
}

/// `Display` delegates to the inner integer's `Display`.
fn display_impl(name: &Ident) -> TokenStream {
    quote! {
        #[automatically_derived]
        impl ::core::fmt::Display for #name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }
    }
}

/// `Default` (only when `default` is declared), guarded by a compile-time assertion that the
/// default satisfies the declared bounds. `d` is typed `#inner`, so the bound literals infer
/// to `#inner` (no i32-inferred overflow, no signed/unsigned mismatch).
fn default_impl(name: &Ident, opts: &Opts) -> TokenStream {
    let Some(d) = opts.default.as_ref() else {
        return quote! {};
    };
    let inner = &opts.inner;
    let lower = opts
        .min
        .as_ref()
        .map_or_else(|| quote! { true }, |m| quote! { d >= #m });
    let upper = opts
        .max
        .as_ref()
        .map_or_else(|| quote! { true }, |m| quote! { d <= #m });
    quote! {
        const _: () = {
            let d: #inner = #d;
            assert!(#lower && #upper, "NumNewtype `default` is outside the declared bounds");
        };

        #[automatically_derived]
        impl ::core::default::Default for #name {
            fn default() -> Self {
                #name(#d)
            }
        }
    }
}

/// The opt-in `clamp` affordance: `MIN`/`MAX` associated consts and an infallible
/// `const fn clamped` that coerces its argument into `MIN..=MAX`. `Ord::clamp` isn't `const`,
/// so the bound is hand-rolled with `<`/`>` (const-evaluable on integer inners). Emitted only
/// under the `clamp` flag, which `expand` has already proven carries both bounds with
/// `min <= max` — so `clamped` can never yield an out-of-range value and does not weaken the
/// newtype's invariant.
fn clamped_impl(name: &Ident, opts: &Opts) -> TokenStream {
    if !opts.clamp {
        return quote! {};
    }
    let (Some(min), Some(max)) = (opts.min.as_ref(), opts.max.as_ref()) else {
        // cov:ignore-start unreachable: `expand` rejects `clamp` without both bounds
        return quote! {};
        // cov:ignore-stop
    };
    let inner = &opts.inner;
    quote! {
        impl #name {
            #[doc = "Inclusive lower bound of the declared range."]
            pub const MIN: #inner = #min;
            #[doc = "Inclusive upper bound of the declared range."]
            pub const MAX: #inner = #max;

            #[doc = "Coerce `v` into `MIN..=MAX`; infallible (the result is always in range)."]
            #[must_use]
            pub const fn clamped(v: #inner) -> Self {
                let v = if v < Self::MIN {
                    Self::MIN
                } else if v > Self::MAX {
                    Self::MAX
                } else {
                    v
                };
                Self(v)
            }
        }
    }
}

/// Transparent-integer serde: the wire form is a bare integer, so a DTO field adopts the
/// type with no serialized-shape change. Deserialize re-runs the bound check, so an
/// out-of-range value is rejected on the wire (mapped to a serde custom error).
fn serde_impl(name: &Ident, err_name: &Ident, opts: &Opts) -> TokenStream {
    let inner = &opts.inner;
    let min = opts.min.as_ref().map(|m| {
        quote! { if v < #m { return ::core::result::Result::Err(::serde::de::Error::custom(#err_name)); } }
    });
    let max = opts.max.as_ref().map(|m| {
        quote! { if v > #m { return ::core::result::Result::Err(::serde::de::Error::custom(#err_name)); } }
    });
    quote! {
        #[automatically_derived]
        impl ::serde::Serialize for #name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::core::result::Result<S::Ok, S::Error> {
                ::serde::Serialize::serialize(&self.0, serializer)
            }
        }

        #[automatically_derived]
        impl<'de> ::serde::Deserialize<'de> for #name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::core::result::Result<Self, D::Error> {
                let v = <#inner as ::serde::Deserialize>::deserialize(deserializer)?;
                #min
                #max
                ::core::result::Result::Ok(#name(v))
            }
        }
    }
}

impl Opts {
    /// The `Display` message: the explicit `error = "..."`, else one generated from the
    /// declared bounds and the type name.
    fn message(&self, name: &Ident) -> TokenStream {
        if let Some(lit) = &self.error {
            return quote! { #lit };
        }
        let bound = match (&self.min, &self.max) {
            (Some(min), Some(max)) => {
                format!("an integer between {} and {}", digits(min), digits(max))
            }
            (Some(min), None) => format!("an integer of at least {}", digits(min)),
            (None, Some(max)) => format!("an integer of at most {}", digits(max)),
            (None, None) => "a valid integer".to_owned(),
        };
        let msg = format!("{name} must be {bound}");
        quote! { #msg }
    }
}

/// A `LitInt`'s digits without any type suffix, for the generated message.
fn digits(l: &LitInt) -> &str {
    l.base10_digits()
}

/// Confirms the single tuple field's type is token-identical to the declared `inner`.
fn require_field_matches_inner(input: &DeriveInput, inner: &Type) -> syn::Result<()> {
    let field_ty = match &input.data {
        syn::Data::Struct(s) => match &s.fields {
            syn::Fields::Unnamed(f) => &f.unnamed[0].ty,
            // Unreachable: `require_newtype_shape` already proved a single unnamed field.
            _ => unreachable!("shape guard ensures a single-field tuple struct"),
        },
        _ => unreachable!("shape guard ensures a struct"),
    };
    if quote!(#field_ty).to_string() == quote!(#inner).to_string() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            field_ty,
            format!(
                "NumNewtype field type must match `#[num_newtype(inner = {})]`",
                quote!(#inner)
            ),
        ))
    }
}

/// Reads `#[num_newtype(inner = <ty>, min = <int>, max = <int>, default = <int>, error = "...", clamp)]`.
/// `inner` is required; `clamp` is a bare flag (requires both bounds, validated in `expand`).
/// A missing `inner`, an unknown key, or a malformed value is a spanned error rendered as
/// `compile_error!`.
fn parse_opts(input: &DeriveInput) -> syn::Result<Opts> {
    let mut inner: Option<Type> = None;
    let mut min: Option<LitInt> = None;
    let mut max: Option<LitInt> = None;
    let mut default: Option<LitInt> = None;
    let mut error: Option<LitStr> = None;
    let mut clamp = false;

    let mut saw_attr = false;
    for attr in &input.attrs {
        if attr.path().is_ident("num_newtype") {
            saw_attr = true;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("inner") {
                    inner = Some(meta.value()?.parse()?);
                    Ok(())
                } else if meta.path.is_ident("min") {
                    min = Some(meta.value()?.parse()?);
                    Ok(())
                } else if meta.path.is_ident("max") {
                    max = Some(meta.value()?.parse()?);
                    Ok(())
                } else if meta.path.is_ident("default") {
                    default = Some(meta.value()?.parse()?);
                    Ok(())
                } else if meta.path.is_ident("error") {
                    error = Some(meta.value()?.parse()?);
                    Ok(())
                } else if meta.path.is_ident("clamp") {
                    // A bare flag — no `= value`.
                    clamp = true;
                    Ok(())
                } else {
                    Err(meta.error(
                        "unknown `num_newtype` option (expected `inner`, `min`, `max`, `default`, `error`, or `clamp`)",
                    ))
                }
            })?;
        } // cov:ignore `?`-fall-through closing brace; executed by the unit tests but llvm-cov leaves the gap region unmarked (mirrors str_newtype)
    }

    let inner = inner.ok_or_else(|| {
        let span_src: &dyn quote::ToTokens = if saw_attr { &input.ident } else { input };
        syn::Error::new_spanned(
            span_src,
            "NumNewtype requires `#[num_newtype(inner = <integer type>)]`",
        )
    })?;

    Ok(Opts {
        inner,
        min,
        max,
        default,
        error,
        clamp,
    })
}
