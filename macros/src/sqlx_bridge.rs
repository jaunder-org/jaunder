//! The shared sqlx storage bridge (ADR-0071), emitted by every derive and attribute
//! macro in this crate that makes a type a first-class column.
//!
//! The three impls are pure delegation, but *what* they delegate to is no longer one
//! type: `Type`, `Encode`, and `Decode` each take their own inner. That split is what
//! lets a `FromStr`-based decode borrow `&'r str` while its `Type` still reports
//! `String` (see [`BridgeSpec`]). Callers differ only in the fields they fill:
//!
//! | Caller                    | Type     | Encode    | Decode    | `Decode` conversion             |
//! | ------------------------- | -------- | --------- | --------- | ------------------------------- |
//! | `StrNewtype` (validating) | `String` | `String`  | `&'r str` | validating `FromStr`            |
//! | `StrNewtype` (infallible) | `String` | `String`  | `String`  | `From<String>` (moves)          |
//! | `IdNewtype`               | `i64`    | `i64`     | `i64`     | infallible wrap                 |
//! | `NumNewtype`              | declared | declared  | declared  | bound-checking `TryFrom<inner>` |
//! | `SqlxBridge`              | field ty | field ty  | field ty  | infallible wrap (moves)         |
//! | `#[text_enum(sqlx)]`      | `String` | `&'q str` | `&'r str` | validating `FromStr`            |
//!
//! The rule is per-conversion, not per-family: a decode that *borrows to parse and
//! drops* takes `&'r str`; a decode that *moves the decoded value into* the new value
//! keeps `String`, where borrowing would add an allocation rather than remove one.
//!
//! All items are wrapped in `#[cfg(feature = "sqlx")]` so the proc-macro crate never
//! depends on sqlx and the wasm build never sees them.

use proc_macro2::TokenStream;
use quote::quote;

/// What [`bridge`] needs to emit the three impls for one type.
///
/// `to_inner` must evaluate to `&#encode_inner` and may use `self`; `convert` may use
/// the bound local **`v`** (of type `decode_inner`, already decoded) and must evaluate
/// to `Result<Self, ::sqlx::error::BoxDynError>`.
pub(crate) struct BridgeSpec<'a> {
    pub(crate) name: &'a syn::Ident,
    pub(crate) type_inner: TokenStream,
    pub(crate) encode_inner: TokenStream,
    pub(crate) to_inner: TokenStream,
    pub(crate) decode_inner: TokenStream,
    pub(crate) convert: TokenStream,
}

/// The three sqlx bridge impls for `spec.name`.
pub(crate) fn bridge(spec: &BridgeSpec<'_>) -> TokenStream {
    let BridgeSpec {
        name,
        type_inner,
        encode_inner,
        to_inner,
        decode_inner,
        convert,
    } = spec;
    quote! {
        #[cfg(feature = "sqlx")]
        const _: () = {
            #[automatically_derived]
            impl<DB: ::sqlx::Database> ::sqlx::Type<DB> for #name
            where
                #type_inner: ::sqlx::Type<DB>,
            {
                fn type_info() -> <DB as ::sqlx::Database>::TypeInfo {
                    <#type_inner as ::sqlx::Type<DB>>::type_info()
                }
                fn compatible(ty: &<DB as ::sqlx::Database>::TypeInfo) -> bool {
                    <#type_inner as ::sqlx::Type<DB>>::compatible(ty)
                }
            }

            #[automatically_derived]
            impl<'q, DB: ::sqlx::Database> ::sqlx::Encode<'q, DB> for #name
            where
                #encode_inner: ::sqlx::Encode<'q, DB>,
            {
                fn encode_by_ref(
                    &self,
                    buf: &mut <DB as ::sqlx::Database>::ArgumentBuffer<'q>,
                ) -> ::core::result::Result<::sqlx::encode::IsNull, ::sqlx::error::BoxDynError>
                {
                    // The annotated local is load-bearing for the enum caller: it coerces
                    // a `&'static str` token to the buffer's `'q`, and (being an extending
                    // borrow of an rvalue) keeps the temporary alive for the block.
                    let inner: &#encode_inner = #to_inner;
                    <#encode_inner as ::sqlx::Encode<'q, DB>>::encode_by_ref(inner, buf)
                }
                fn size_hint(&self) -> usize {
                    let inner: &#encode_inner = #to_inner;
                    <#encode_inner as ::sqlx::Encode<'q, DB>>::size_hint(inner)
                }
            }

            #[automatically_derived]
            impl<'r, DB: ::sqlx::Database> ::sqlx::Decode<'r, DB> for #name
            where
                #decode_inner: ::sqlx::Decode<'r, DB>,
            {
                fn decode(
                    value: <DB as ::sqlx::Database>::ValueRef<'r>,
                ) -> ::core::result::Result<Self, ::sqlx::error::BoxDynError> {
                    let v = <#decode_inner as ::sqlx::Decode<'r, DB>>::decode(value)?;
                    #convert
                }
            }
        };
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use quote::format_ident;

    /// Whitespace-stripped rendering, so assertions survive `TokenStream`'s spacing.
    /// Note: this also strips spaces *inside* string literals.
    pub(crate) fn norm(t: &TokenStream) -> String {
        t.to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    fn spec_for(name: &syn::Ident) -> BridgeSpec<'_> {
        BridgeSpec {
            name,
            type_inner: quote! { ::std::string::String },
            encode_inner: quote! { ::std::string::String },
            to_inner: quote! { &self.0 },
            decode_inner: quote! { ::std::string::String },
            convert: quote! { ::core::result::Result::Ok(#name(v)) },
        }
    }

    #[test]
    fn type_impl_delegates_to_type_inner() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::compatible(ty)"));
    }

    #[test]
    fn encode_binds_an_annotated_local_and_keeps_size_hint() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(out.contains("letinner:&::std::string::String=&self.0;"));
        assert!(out.contains("::encode_by_ref(inner,buf)"));
        assert!(
            out.contains("fnsize_hint(&self)->usize"),
            "size_hint must be emitted"
        );
        assert!(out.contains("::size_hint(inner)"));
    }

    #[test]
    fn decode_delegates_to_decode_inner_then_converts() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(
            out.contains("letv=<::std::string::Stringas::sqlx::Decode<'r,DB>>::decode(value)?;")
        );
        assert!(out.contains("::core::result::Result::Ok(X(v))"));
    }

    #[test]
    fn the_three_inners_are_independent() {
        let n = format_ident!("X");
        let out = norm(&bridge(&BridgeSpec {
            name: &n,
            type_inner: quote! { ::std::string::String },
            encode_inner: quote! { &'q str },
            to_inner: quote! { &"tok" },
            decode_inner: quote! { &'r str },
            convert: quote! { ::core::result::Result::Ok(X) },
        }));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("<&'qstras::sqlx::Encode<'q,DB>>::encode_by_ref(inner,buf)"));
        assert!(out.contains("<&'rstras::sqlx::Decode<'r,DB>>::decode(value)?"));
    }

    #[test]
    fn output_is_feature_gated_and_marked_derived() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(out.contains("#[cfg(feature=\"sqlx\")]"));
        assert_eq!(out.matches("#[automatically_derived]").count(), 3);
    }
}
