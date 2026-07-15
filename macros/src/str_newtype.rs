//! Codegen for `#[derive(StrNewtype)]` — the ADR-0063 string-newtype trailer for a
//! `struct X(String)`. The derive owns the whole trailer except `FromStr` (the one
//! per-type validating/normalizing chokepoint) and the std `#[derive]`s.

use quote::quote;
use syn::DeriveInput;

/// The `#[str_newtype(...)]` options: the tight `secret` surface, whether a
/// secret re-opens the serde bridge (`secret, serde`) for an inbound wire value,
/// and the `infallible` mode (construction via a hand-written `From<String>`
/// instead of `FromStr`, for a newtype whose invariant never rejects).
struct Opts {
    secret: bool,
    serde: bool,
    infallible: bool,
}

/// Expands `#[derive(StrNewtype)]` on a single-field tuple struct. On the wrong shape
/// (or an unknown/invalid `str_newtype` option) it returns a spanned `compile_error!`
/// instead of malformed impls. `#[str_newtype(secret)]` selects the tight secret
/// surface; `#[str_newtype(secret, serde)]` adds the serde bridge back to it.
pub(crate) fn expand(input: &DeriveInput) -> proc_macro2::TokenStream {
    if let Err(e) = crate::require_newtype_shape(input, "StrNewtype", "struct X(String)") {
        return e.to_compile_error();
    }
    let opts = match parse_opts(input) {
        Ok(o) => o,
        Err(e) => return e.to_compile_error(),
    };
    let name = &input.ident;

    if opts.secret {
        let trailer = secret_trailer(name);
        // A secret re-opens serde only when it must cross the wire *inbound*
        // (`secret, serde`); its inbound-only role is enforced by an xtask gate.
        let serde = if opts.serde {
            serde_impls(name)
        } else {
            quote! {}
        };
        return quote! {
            #trailer
            #serde
        };
    }

    if opts.infallible {
        return infallible_trailer(name);
    }

    let serde = serde_impls(name);
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

        #serde
    }
}

/// The serde bridge as direct impls (not `#[serde(try_from/into)]`): serialize borrows
/// instead of cloning into a String, and deserialize routes through `FromStr` so invalid
/// input is rejected on the wire. Shared by the default trailer and the `secret, serde`
/// variant.
fn serde_impls(name: &syn::Ident) -> proc_macro2::TokenStream {
    quote! {
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

/// The **infallible trailer**: the full default trailer *minus* the fallible door.
/// Construction never rejects, so there is no `FromStr`/`TryFrom<String>` — instead the
/// type author hand-writes the one `From<String>` chokepoint (pure-wrap or normalizing),
/// and the derived `Deserialize` routes a deserialized `String` through *that*, so wire
/// values are normalized identically to in-process construction. Emits `Display`,
/// `AsRef`/`Borrow`/`Deref<str>`, `From<Self> for String`, `PartialEq<str>`/`<&str>`, and
/// the infallible serde bridge; deliberately omits `TryFrom<String>` (which would collide
/// with the hand-written `From<String>` via the std blanket `impl<T, U: Into<T>> TryFrom<U>`).
fn infallible_trailer(name: &syn::Ident) -> proc_macro2::TokenStream {
    let serde = serde_impls_infallible(name);
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

        #serde
    }
}

/// The infallible serde bridge: serialize borrows (as the default); deserialize a `String`
/// and route it through the type's own `From<String>` (never `FromStr`), so it cannot fail
/// and normalizes wire input identically to construction.
fn serde_impls_infallible(name: &syn::Ident) -> proc_macro2::TokenStream {
    quote! {
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
                ::core::result::Result::Ok(<#name as ::core::convert::From<::std::string::String>>::from(s))
            }
        }
    }
}

/// The **tight secret surface** (ADR-0063 secret exception, as amended by #403): a
/// redacting `Debug`, explicit borrowed access via `AsRef<str>`, and construction via
/// `TryFrom<String>` — and deliberately *none* of `Display`, `Deref`, `Borrow`,
/// `From<Self> for String`, or `PartialEq`, so a secret cannot leak or be value-compared.
/// `#[str_newtype(secret, serde)]` layers the serde bridge back on for an inbound value.
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

/// Reads `#[str_newtype(secret)]` / `#[str_newtype(secret, serde)]`. Errors on any other
/// option so a typo fails loudly rather than silently un-redacting, and on a bare
/// `serde` (the default trailer already has the serde bridge — `serde` is only meaningful
/// as a re-opener for a `secret`).
fn parse_opts(input: &DeriveInput) -> syn::Result<Opts> {
    let mut secret = false;
    let mut serde = false;
    let mut infallible = false;
    for attr in &input.attrs {
        if attr.path().is_ident("str_newtype") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("secret") {
                    secret = true;
                    Ok(())
                } else if meta.path.is_ident("serde") {
                    serde = true;
                    Ok(())
                } else if meta.path.is_ident("infallible") {
                    infallible = true;
                    Ok(())
                } else {
                    Err(meta.error(
                        "unknown `str_newtype` option (expected `secret`, `serde`, or `infallible`)",
                    ))
                }
            })?;
        } // cov:ignore `?`-fall-through closing brace; executed by the secret unit tests but llvm-cov leaves the gap region unmarked
    }
    // Checked before the `serde`-needs-`secret` guard so `infallible, serde` reports the
    // exclusivity error rather than falling through to it.
    if infallible && (secret || serde) {
        return Err(syn::Error::new_spanned(
            input,
            "`str_newtype(infallible)` is exclusive with `secret`/`serde` (infallible mode already includes the serde bridge)",
        ));
    }
    if serde && !secret {
        return Err(syn::Error::new_spanned(
            input,
            "`str_newtype(serde)` is only valid with `secret`; the default trailer already includes the serde bridge",
        ));
    }
    Ok(Opts {
        secret,
        serde,
        infallible,
    })
}
