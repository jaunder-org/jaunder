//! Codegen for `#[derive(IdNewtype)]` — the ADR-0063 numeric-ID trailer for a
//! `struct X(i64)`: `From<i64>`, `From<Self> for i64`, `Display`, `FromStr` (delegating to
//! `i64`'s parse), a transparent-i64 serde bridge, and the feature-gated sqlx storage bridge
//! (ADR-0071). `Copy` and the other std traits stay in the user's `#[derive]` list.
//!
//! Unlike `StrNewtype`, the sqlx bridge here is **unconditional** — there is no
//! `no_sqlx`/`sqlx` control, because every id is stored. Add one only if a non-stored id
//! ever appears.

use quote::quote;
use syn::DeriveInput;

/// Expands `#[derive(IdNewtype)]` on a single-field tuple struct. On the wrong shape it
/// returns a spanned `compile_error!` instead of malformed impls.
pub(crate) fn expand(input: &DeriveInput) -> proc_macro2::TokenStream {
    if let Err(e) = crate::require_newtype_shape(input, "IdNewtype", "struct X(i64)") {
        return e.to_compile_error();
    }
    let name = &input.ident;
    let sqlx = sqlx_impls(name);

    quote! {
        #[automatically_derived]
        impl ::core::convert::From<i64> for #name {
            fn from(v: i64) -> Self {
                #name(v)
            }
        }

        #[automatically_derived]
        impl ::core::convert::From<#name> for i64 {
            fn from(v: #name) -> Self {
                v.0
            }
        }

        #[automatically_derived]
        impl ::core::fmt::Display for #name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }

        // `FromStr` delegates to `i64`'s parse, then wraps — so `"42".parse::<#name>()` works
        // at the few sites that carry an id as a string (e.g. a Leptos route param, whose
        // `ParamsMap` yields `String`). Unlike a string newtype's `FromStr`, it enforces no
        // invariant beyond "is an integer" (an id has no value invariant, only the
        // transposition guarantee); it is the inverse of `Display`, not a validating chokepoint.
        #[automatically_derived]
        impl ::core::str::FromStr for #name {
            type Err = ::core::num::ParseIntError;
            fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {
                ::core::result::Result::Ok(#name(<i64 as ::core::str::FromStr>::from_str(s)?))
            }
        }

        // Transparent-i64 serde: the wire form is a bare integer (`42`), so DTO fields can
        // adopt the type without changing any serialized shape. Deserialize is an infallible
        // wrap — an id has no value invariant, only the transposition guarantee.
        #[automatically_derived]
        impl ::serde::Serialize for #name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::core::result::Result<S::Ok, S::Error> {
                serializer.serialize_i64(self.0)
            }
        }

        #[automatically_derived]
        impl<'de> ::serde::Deserialize<'de> for #name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::core::result::Result<Self, D::Error> {
                ::core::result::Result::Ok(#name(
                    <i64 as ::serde::Deserialize>::deserialize(deserializer)?,
                ))
            }
        }

        #sqlx
    }
}

/// The sqlx storage bridge (ADR-0071) for an id: the shared [`crate::sqlx_bridge`]
/// delegation to the inner `i64`, so an id binds and decodes as a bare integer column on
/// every backend.
///
/// `Decode` is an **infallible wrap** — an id has no value invariant, only the
/// transposition guarantee (ADR-0063 §2) — so unlike the string bridge it does not route
/// through a validating `FromStr`, and unlike `NumNewtype`'s it does not re-run a bound.
fn sqlx_impls(name: &syn::Ident) -> proc_macro2::TokenStream {
    let convert = quote! {
        ::core::result::Result::Ok(#name(v))
    };
    crate::sqlx_bridge::bridge(name, &quote! { i64 }, &convert)
}
