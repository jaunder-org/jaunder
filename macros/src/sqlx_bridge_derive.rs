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
    let text = match wants_text(&input.attrs) {
        Ok(text) => text,
        Err(e) => return e.to_compile_error(),
    };
    if text {
        // A value living in a TEXT column must report `String` as its `Type`, not the
        // field's type — decoding alone would leave the column integer-shaped. So all
        // three inners move together, and the decode borrows to parse and drops (the
        // per-conversion rule in `sqlx_bridge`'s module doc).
        return crate::sqlx_bridge::bridge(&crate::sqlx_bridge::BridgeSpec {
            name,
            type_inner: quote! { String },
            encode_inner: quote! { String },
            to_inner: quote! { &self.0.to_string() },
            decode_inner: quote! { &'r str },
            // Parses into `Self`, **not** into the inner type: `parse::<#inner>().map(Self)`
            // would wrap the raw scalar directly and so bypass the newtype's own validating
            // `FromStr`, letting a corrupt row reconstitute a value the type forbids (a
            // stored `"0"` decoding to a `SmtpPort(0)` whose `FromStr` rejects zero). The
            // charter forbids this derive from *emitting* a constructor; calling the one the
            // author wrote is what the `StrNewtype` and `text_enum` bridges already do. Spelt
            // `parse::<#name>` rather than a `FromStr` path so no token here reads as an
            // emitted impl — see `text_option_still_emits_no_constructor`.
            //
            // The error echoes the offending text. The inner type's own parse error
            // describes a grammar ("invalid digit found in string") and names nothing —
            // and a `ColumnDecode` labels the column, not the value — so without this a
            // corrupt row is reported in terms an operator cannot act on. A value stored
            // as text in a config column is operator-facing by construction, never a
            // secret (secrets are `StrNewtype`s, whose bridge does not do this).
            convert: quote! {
                v.parse::<#name>()
                    .map_err(|e| -> ::sqlx::error::BoxDynError {
                        // `.into()`, not `From::from` — the charter test forbids the bare
                        // token `From` anywhere in this derive's output, and an error
                        // conversion must not be the thing that erodes that guard.
                        ::std::format!("{e}; stored value: {v:?}").into()
                    })
            },
        });
    }
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

/// Whether `#[sqlx_bridge(text)]` is present.
///
/// The only option, and a bare flag rather than `decode_inner = <ty>`: what a caller
/// needs to say is "this value is stored as text", which moves `Type`, `Encode` and
/// `Decode` together. Naming one of the three would describe the mechanism and
/// under-specify the intent.
fn wants_text(attrs: &[syn::Attribute]) -> syn::Result<bool> {
    let mut text = false;
    for attr in attrs.iter().filter(|a| a.path().is_ident("sqlx_bridge")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("text") {
                text = true;
                Ok(())
            } else {
                Err(meta.error("unknown `sqlx_bridge` option (expected `text`)"))
            }
        })?;
    }
    Ok(text)
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
        // Bare `From` is in the list deliberately: it subsumes `FromStr`/`TryFrom` and
        // catches any inbound constructor spelling, which is the whole point of this
        // derive existing separately from `StrNewtype`.
        for forbidden in [
            "From",
            "FromStr",
            "TryFrom",
            "Deserialize",
            "Serialize",
            "Display",
            "Deref",
        ] {
            assert!(!out.contains(forbidden), "{forbidden} must not be emitted");
        }
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

    /// `#[sqlx_bridge(text)]` moves all three inners to the text forms, because a
    /// value stored in a `TEXT` column must report `String` as its `Type` — decoding
    /// alone is not enough (#687: `site_config.value` is `TEXT NOT NULL`).
    #[test]
    fn text_option_makes_the_column_text_and_parses_on_decode() {
        let input: DeriveInput = syn::parse_quote! {
            #[sqlx_bridge(text)]
            pub struct SmtpPort(u16);
        };
        let out = norm(&expand(&input));
        assert!(
            out.contains("<Stringas::sqlx::Type<DB>>::type_info()"),
            "the column must be TEXT, not the field's integer type: {out}"
        );
        assert!(
            out.contains("letinner:&String=&self.0.to_string();"),
            "encode must render the value as text: {out}"
        );
        assert!(
            out.contains("<&'rstras::sqlx::Decode<'r,DB>>::decode(value)?"),
            "decode borrows to parse and drops: {out}"
        );
        assert!(
            out.contains("v.parse::<SmtpPort>()"),
            "convert must route through the newtype's validating FromStr, not the raw \
             inner type — parsing into `u16` and wrapping would let a corrupt row \
             reconstitute a value the type forbids: {out}"
        );
        assert!(
            !out.contains("map(Self)"),
            "a bare `map(Self)` is the bypass this guards against: {out}"
        );
        // `norm` strips spaces inside literals too, so match the stripped form.
        assert!(
            out.contains(
                crate::sqlx_bridge::tests::norm_s(r#"::std::format!("{e}; stored value: {v:?}")"#)
                    .as_str()
            ),
            "a corrupt row must report what it holds, not just the parser's grammar: {out}"
        );
    }

    /// The charter (see `emits_only_the_three_bridge_impls`) holds under the option:
    /// a bridge must not leak an inbound constructor, so `SmtpPort` still needs a
    /// hand-written `FromStr`.
    #[test]
    fn text_option_still_emits_no_constructor() {
        let input: DeriveInput = syn::parse_quote! {
            #[sqlx_bridge(text)]
            pub struct SmtpPort(u16);
        };
        let out = norm(&expand(&input));
        // Same list as `emits_only_the_three_bridge_impls`, bare `From` included. Dropping
        // `From` here to accommodate an error conversion would quietly retire the strongest
        // anti-constructor assertion for the one mode that most needs it.
        for forbidden in [
            "From",
            "FromStr",
            "TryFrom",
            "Deserialize",
            "Serialize",
            "Display",
            "Deref",
        ] {
            assert!(
                !out.contains(forbidden),
                "{forbidden} must not be emitted: {out}"
            );
        }
    }

    /// Opt-in: without the attribute the emitted tokens are exactly what they were.
    #[test]
    fn without_the_option_every_inner_is_still_the_field_type() {
        let input: DeriveInput = syn::parse_quote! { pub struct IntPort(u16); };
        let out = norm(&expand(&input));
        assert!(out.contains("<u16as::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("letinner:&u16=&self.0;"));
        assert!(out.contains("::core::result::Result::Ok(Self(v))"));
        assert!(
            !out.contains("parse::<"),
            "no parse step without the option: {out}"
        );
    }

    #[test]
    fn an_unknown_option_is_a_spanned_error() {
        let input: DeriveInput = syn::parse_quote! {
            #[sqlx_bridge(nonsense)]
            pub struct X(u16);
        };
        let out = expand(&input).to_string();
        assert!(out.contains("compile_error"), "{out}");
        assert!(
            out.contains("sqlx_bridge"),
            "the message must name the attribute: {out}"
        );
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
