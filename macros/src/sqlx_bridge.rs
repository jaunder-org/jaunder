//! The shared sqlx storage bridge (ADR-0071), emitted by all three newtype derives.
//!
//! `Type`/`Encode` are pure delegation to the newtype's inner type and are therefore
//! **identical** across the three families — only the inner type differs
//! (`String` for `StrNewtype`, `i64` for `IdNewtype`, the declared `inner` for
//! `NumNewtype`). The families differ solely in how `Decode` turns the decoded inner
//! value back into `Self`, which is the one thing [`bridge`] takes as a parameter:
//!
//! | Derive       | Inner              | `Decode` conversion                           |
//! | ------------ | ------------------ | --------------------------------------------- |
//! | `StrNewtype` | `String`           | validating `FromStr`, or `From<String>`       |
//! | `IdNewtype`  | `i64`              | infallible wrap (no invariant to re-run)      |
//! | `NumNewtype` | declared `inner`   | bound-checking `TryFrom<inner>`               |
//!
//! All items are wrapped in `#[cfg(feature = "sqlx")]` so the proc-macro crate never
//! depends on sqlx and the wasm build never sees them.

use proc_macro2::TokenStream;
use quote::quote;

/// The three sqlx bridge impls for `name`, delegating to `inner`.
///
/// `convert` is the tail of `Decode::decode`: it may use the bound local **`v`** (of type
/// `inner`, already decoded) and must evaluate to
/// `Result<Self, ::sqlx::error::BoxDynError>`.
pub(crate) fn bridge(name: &syn::Ident, inner: &TokenStream, convert: &TokenStream) -> TokenStream {
    quote! {
        #[cfg(feature = "sqlx")]
        const _: () = {
            #[automatically_derived]
            impl<DB: ::sqlx::Database> ::sqlx::Type<DB> for #name
            where
                #inner: ::sqlx::Type<DB>,
            {
                fn type_info() -> <DB as ::sqlx::Database>::TypeInfo {
                    <#inner as ::sqlx::Type<DB>>::type_info()
                }
                fn compatible(ty: &<DB as ::sqlx::Database>::TypeInfo) -> bool {
                    <#inner as ::sqlx::Type<DB>>::compatible(ty)
                }
            }

            #[automatically_derived]
            impl<'q, DB: ::sqlx::Database> ::sqlx::Encode<'q, DB> for #name
            where
                #inner: ::sqlx::Encode<'q, DB>,
            {
                fn encode_by_ref(
                    &self,
                    buf: &mut <DB as ::sqlx::Database>::ArgumentBuffer<'q>,
                ) -> ::core::result::Result<::sqlx::encode::IsNull, ::sqlx::error::BoxDynError>
                {
                    <#inner as ::sqlx::Encode<'q, DB>>::encode_by_ref(&self.0, buf)
                }
                fn size_hint(&self) -> usize {
                    <#inner as ::sqlx::Encode<'q, DB>>::size_hint(&self.0)
                }
            }

            #[automatically_derived]
            impl<'r, DB: ::sqlx::Database> ::sqlx::Decode<'r, DB> for #name
            where
                #inner: ::sqlx::Decode<'r, DB>,
            {
                fn decode(
                    value: <DB as ::sqlx::Database>::ValueRef<'r>,
                ) -> ::core::result::Result<Self, ::sqlx::error::BoxDynError> {
                    let v = <#inner as ::sqlx::Decode<'r, DB>>::decode(value)?;
                    #convert
                }
            }
        };
    }
}
