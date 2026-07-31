//! Codegen for `#[derive(SqlxBridge)]` — the sqlx storage bridge and **nothing else**,
//! for a `struct X(Inner)` whose constructor is not the derive's business.
//!
//! This exists because the shared bridge ([`crate::sqlx_bridge`]) was otherwise reachable
//! only through a derive that also emits a trailer. `RenderedHtml` needs the bridge and
//! must never gain an inbound constructor from a raw `String`, so a `bridge_only` *mode*
//! on `StrNewtype` would put that invariant one editing mistake away from a trailer arm.
//! A separate derive cannot leak a constructor: the codegen is not in it.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

/// Expands `#[derive(SqlxBridge)]`. On the wrong shape it returns a spanned
/// `compile_error!` instead of malformed impls.
pub(crate) fn expand(input: &DeriveInput) -> TokenStream {
    if let Err(e) = crate::require_newtype_shape(input, "SqlxBridge", "struct X(Inner)") {
        return e.to_compile_error();
    }
    // `require_newtype_shape` has already established the single-unnamed-field shape.
    let Data::Struct(data) = &input.data else {
        unreachable!("shape guard admits only a tuple struct")
    };
    let Fields::Unnamed(fields) = &data.fields else {
        unreachable!("shape guard admits only unnamed fields")
    };
    let inner = &fields.unnamed[0].ty;
    let name = &input.ident;
    crate::sqlx_bridge::bridge(&crate::sqlx_bridge::BridgeSpec {
        name,
        type_inner: quote! { #inner },
        encode_inner: quote! { #inner },
        to_inner: quote! { &self.0 },
        // The decoded value is *moved* into the newtype, so an owned inner is already
        // optimal here — borrowing would add an allocation, not remove one.
        decode_inner: quote! { #inner },
        convert: quote! { ::core::result::Result::Ok(Self(v)) },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlx_bridge::tests::norm;

    #[test]
    fn emits_only_the_three_bridge_impls() {
        let input: DeriveInput = syn::parse_quote! { pub struct RenderedHtml(String); };
        let out = norm(&expand(&input));
        assert!(out.contains("::sqlx::Type<DB>forRenderedHtml"));
        assert!(out.contains("::sqlx::Encode<'q,DB>forRenderedHtml"));
        assert!(out.contains("::sqlx::Decode<'r,DB>forRenderedHtml"));
        for forbidden in [
            "FromStr",
            "TryFrom",
            "Deserialize",
            "Serialize",
            "Display",
            "Deref",
        ] {
            assert!(!out.contains(forbidden), "{forbidden} must not be emitted");
        }
        assert!(!out.contains("From<::std::string::String>forRenderedHtml"));
    }

    #[test]
    fn all_three_inners_are_the_field_type_and_decode_moves() {
        let input: DeriveInput = syn::parse_quote! { pub struct RenderedHtml(String); };
        let out = norm(&expand(&input));
        assert!(out.contains("<Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("letinner:&String=&self.0;"));
        assert!(out.contains("<Stringas::sqlx::Decode<'r,DB>>::decode(value)?"));
        assert!(out.contains("::core::result::Result::Ok(Self(v))"));
    }

    #[test]
    fn wrong_shape_is_a_spanned_error_naming_the_derive() {
        let input: DeriveInput = syn::parse_quote! { pub enum E { A } };
        let out = expand(&input).to_string();
        assert!(out.contains("compile_error"));
        assert!(
            out.contains("SqlxBridge"),
            "the message must name the derive"
        );
    }
}
