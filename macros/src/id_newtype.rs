//! Codegen for `#[derive(IdNewtype)]` — the ADR-0063 numeric-ID trailer for a
//! `struct X(i64)`: `From<i64>`, `From<Self> for i64`, `Display`, `FromStr` (delegating to
//! `i64`'s parse), and a transparent-i64 serde bridge. `Copy` and the other std traits stay
//! in the user's `#[derive]` list.

use quote::quote;
use syn::DeriveInput;

/// Expands `#[derive(IdNewtype)]` on a single-field tuple struct. On the wrong shape it
/// returns a spanned `compile_error!` instead of malformed impls.
pub(crate) fn expand(input: &DeriveInput) -> proc_macro2::TokenStream {
    if let Err(e) = crate::require_newtype_shape(input, "IdNewtype", "struct X(i64)") {
        return e.to_compile_error();
    }
    let name = &input.ident;

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
    }
}
