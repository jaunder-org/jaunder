//! Codegen for `#[derive(StrNewtype)]` — the ADR-0063 string-newtype trailer for a
//! `struct X(String)`. The derive owns the whole trailer except `FromStr` (the one
//! per-type validating/normalizing chokepoint) and the std `#[derive]`s.

use quote::quote;
use syn::DeriveInput;

/// Expands `#[derive(StrNewtype)]` on a single-field tuple struct. On the wrong shape
/// (or an unknown `str_newtype` option) it returns a spanned `compile_error!` instead of
/// malformed impls. `#[str_newtype(secret)]` selects the tight secret surface.
pub(crate) fn expand(input: &DeriveInput) -> proc_macro2::TokenStream {
    if let Err(e) = crate::require_newtype_shape(input, "StrNewtype", "struct X(String)") {
        return e.to_compile_error();
    }
    let secret = match is_secret(input) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error(),
    };
    let name = &input.ident;

    if secret {
        return secret_trailer(name);
    }

    quote! {
        #[automatically_derived]
        impl ::core::fmt::Display for #name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(&self.0)
            }
        }

        #[automatically_derived]
        impl ::core::convert::AsRef<str> for #name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        #[automatically_derived]
        impl ::core::borrow::Borrow<str> for #name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        #[automatically_derived]
        impl ::core::ops::Deref for #name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        #[automatically_derived]
        impl ::core::convert::TryFrom<::std::string::String> for #name {
            type Error = <#name as ::core::str::FromStr>::Err;
            fn try_from(s: ::std::string::String) -> ::core::result::Result<Self, Self::Error> {
                <#name as ::core::str::FromStr>::from_str(&s)
            }
        }

        #[automatically_derived]
        impl ::core::convert::From<#name> for ::std::string::String {
            fn from(v: #name) -> Self {
                v.0
            }
        }

        #[automatically_derived]
        impl ::core::cmp::PartialEq<str> for #name {
            fn eq(&self, other: &str) -> bool {
                self.0 == *other
            }
        }

        #[automatically_derived]
        impl ::core::cmp::PartialEq<&str> for #name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == **other
            }
        }

        // serde bridge as direct impls (not `#[serde(try_from/into)]`): serialize borrows
        // instead of cloning into a String, and deserialize routes through `FromStr` so
        // invalid input is rejected on the wire.
        #[automatically_derived]
        impl ::serde::Serialize for #name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::core::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        #[automatically_derived]
        impl<'de> ::serde::Deserialize<'de> for #name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::core::result::Result<Self, D::Error> {
                let s = <::std::string::String as ::serde::Deserialize>::deserialize(deserializer)?;
                <#name as ::core::str::FromStr>::from_str(&s).map_err(::serde::de::Error::custom)
            }
        }
    }
}

/// The **tight secret surface** (ADR-0063 secret exception, as amended by #403): a
/// redacting `Debug`, explicit borrowed access via `AsRef<str>`, and construction via
/// `TryFrom<String>` — and deliberately *none* of `Display`, serde, `Deref`, `Borrow`,
/// `From<Self> for String`, or `PartialEq`, so a secret cannot leak or be value-compared.
fn secret_trailer(name: &syn::Ident) -> proc_macro2::TokenStream {
    quote! {
        #[automatically_derived]
        impl ::core::fmt::Debug for #name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(concat!(stringify!(#name), "([redacted])"))
            }
        }

        #[automatically_derived]
        impl ::core::convert::AsRef<str> for #name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        #[automatically_derived]
        impl ::core::convert::TryFrom<::std::string::String> for #name {
            type Error = <#name as ::core::str::FromStr>::Err;
            fn try_from(s: ::std::string::String) -> ::core::result::Result<Self, Self::Error> {
                <#name as ::core::str::FromStr>::from_str(&s)
            }
        }
    }
}

/// Reads `#[str_newtype(secret)]`. Returns `true` when present, errors on any other
/// `str_newtype(...)` option so a typo fails loudly rather than silently un-redacting.
fn is_secret(input: &DeriveInput) -> syn::Result<bool> {
    let mut secret = false;
    for attr in &input.attrs {
        if attr.path().is_ident("str_newtype") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("secret") {
                    secret = true;
                    Ok(())
                } else {
                    Err(meta.error("unknown `str_newtype` option (expected `secret`)"))
                }
            })?;
        } // cov:ignore `?`-fall-through closing brace; executed by the secret unit test but llvm-cov leaves the gap region unmarked
    }
    Ok(secret)
}
