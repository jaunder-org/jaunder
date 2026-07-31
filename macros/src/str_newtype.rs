//! Codegen for `#[derive(StrNewtype)]` — the ADR-0063 string-newtype trailer for a
//! `struct X(String)`. The derive owns the whole trailer except `FromStr` (the one
//! per-type validating/normalizing chokepoint) and the std `#[derive]`s.

use quote::quote;
use syn::DeriveInput;

/// Which trailer the derive emits — the three are mutually exclusive (grouped into an
/// enum rather than parallel bools so an invalid combination is unrepresentable and the
/// `Opts` bool count stays in bounds).
enum Kind {
    /// The full default trailer (`Display`/`Deref`/serde/`TryFrom`/`FromStr` routing).
    Default,
    /// The tight `secret` surface (redacting `Debug`, `AsRef` + `TryFrom` only).
    Secret,
    /// The infallible trailer (construction via a hand-written `From<String>`).
    Infallible,
}

/// The `#[str_newtype(...)]` options: the trailer `kind`, whether a secret re-opens the
/// serde bridge (`secret, serde`) for an inbound wire value, and the sqlx bridge controls.
/// The sqlx bridge (feature-gated `Type`/`Encode`/`Decode`) is **on by default** for every
/// non-secret type, dropped for a `secret` one; `sqlx` re-adds it to a secret that genuinely
/// is stored (`InviteCode`) and `no_sqlx` opts a non-secret must-not-store type out
/// (`RawToken`).
struct Opts {
    kind: Kind,
    serde: bool,
    sqlx: bool,
    no_sqlx: bool,
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

    // The sqlx storage bridge is a per-kind decision (default-on except secret),
    // computed once here; each trailer is a sibling helper, so the three arms stay
    // short and parallel.
    let sqlx = sqlx_bridge(&opts, name);
    match opts.kind {
        // The tight secret surface; `secret, serde` re-opens the serde bridge for an
        // inbound wire value (its inbound-only role is enforced by an xtask gate).
        Kind::Secret => {
            let trailer = secret_trailer(name);
            let serde = if opts.serde {
                serde_impls(name)
            } else {
                quote! {}
            };
            quote! {
                #trailer
                #serde
                #sqlx
            }
        }
        // Construction never rejects (a hand-written `From<String>` chokepoint), so the
        // trailer omits `FromStr`/`TryFrom` and the bridges route through `From<String>`.
        Kind::Infallible => {
            let trailer = infallible_trailer(name);
            quote! {
                #trailer
                #sqlx
            }
        }
        // The full ergonomic trailer plus the validating serde and sqlx bridges.
        Kind::Default => {
            let trailer = default_trailer(name);
            let serde = serde_impls(name);
            quote! {
                #trailer
                #serde
                #sqlx
            }
        }
    }
}

/// The sqlx storage bridge for `name`, per the default-on-except-secret policy: a
/// non-secret type gets it unless `no_sqlx` opts out (`RawToken`); a `secret` gets it
/// only with an explicit `sqlx` (a stored secret, `InviteCode`). `Infallible` types
/// decode via `From<String>`; the rest validate via `FromStr`. The `opts` guarantees
/// from `parse_opts` (no bare `sqlx` off a secret, no `no_sqlx` on a secret) keep the
/// arms consistent.
fn sqlx_bridge(opts: &Opts, name: &syn::Ident) -> proc_macro2::TokenStream {
    match opts.kind {
        Kind::Secret if opts.sqlx => sqlx_impls(name),
        Kind::Secret => quote! {},
        _ if opts.no_sqlx => quote! {},
        Kind::Infallible => sqlx_impls_infallible(name),
        Kind::Default => sqlx_impls(name),
    }
}

/// The full **ergonomic default trailer**: `Display`, `AsRef`/`Borrow`/`Deref<str>`,
/// `TryFrom<String>` (the fallible door, via `FromStr`), `From<Self> for String`, and
/// `PartialEq<str>`/`<&str>`. The serde and sqlx bridges are appended by [`expand`], so
/// this mirrors [`secret_trailer`]/[`infallible_trailer`] as one of the three trailers.
fn default_trailer(name: &syn::Ident) -> proc_macro2::TokenStream {
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

        // A borrowed-source alias for the owned `From<String>` chokepoint: routes through
        // it (so any normalization still happens in one place) and lets a `&str`/literal
        // construct the newtype with a single `.into()` / `X::from("…")`, no `.to_owned()`.
        #[automatically_derived]
        impl ::core::convert::From<&str> for #name {
            fn from(s: &str) -> Self {
                <#name as ::core::convert::From<::std::string::String>>::from(
                    ::std::string::String::from(s),
                )
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

/// The inner type every string newtype's sqlx bridge delegates to.
fn sqlx_inner() -> proc_macro2::TokenStream {
    quote! { ::std::string::String }
}

/// The **validating sqlx bridge**: makes the newtype a first-class TEXT column via the
/// shared [`crate::sqlx_bridge`] delegation, with a `Decode` that routes the decoded
/// `String` through `<#name as FromStr>::from_str`, so a corrupted/migrated column is
/// rejected rather than silently admitted; the `?` folds the `FromStr::Err` (all our
/// newtype errors derive `thiserror::Error`) into `sqlx::error::BoxDynError`.
fn sqlx_impls(name: &syn::Ident) -> proc_macro2::TokenStream {
    crate::sqlx_bridge::bridge(&crate::sqlx_bridge::BridgeSpec {
        name,
        type_inner: sqlx_inner(),
        encode_inner: sqlx_inner(),
        to_inner: quote! { &self.0 },
        // Borrowed: `FromStr` parses from a `&str` and builds its own `String`, so
        // decoding an owned one here would allocate it only to drop it (#746 D3).
        decode_inner: quote! { &'r str },
        convert: quote! {
            ::core::result::Result::Ok(<#name as ::core::str::FromStr>::from_str(v)?)
        },
    })
}

/// The **infallible sqlx bridge**: as `sqlx_impls`, but `Decode` wraps the decoded
/// `String` via the type's infallible `From<String>` (no validation to run).
fn sqlx_impls_infallible(name: &syn::Ident) -> proc_macro2::TokenStream {
    crate::sqlx_bridge::bridge(&crate::sqlx_bridge::BridgeSpec {
        name,
        type_inner: sqlx_inner(),
        encode_inner: sqlx_inner(),
        to_inner: quote! { &self.0 },
        // Stays `String`: the `From<String>` chokepoint takes the decoded value by
        // value, so borrowing here would add an allocation, not remove one.
        decode_inner: sqlx_inner(),
        convert: quote! {
            ::core::result::Result::Ok(
                <#name as ::core::convert::From<::std::string::String>>::from(v),
            )
        },
    })
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
    let mut sqlx = false;
    let mut no_sqlx = false;
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
                } else if meta.path.is_ident("sqlx") {
                    sqlx = true;
                    Ok(())
                } else if meta.path.is_ident("no_sqlx") {
                    no_sqlx = true;
                    Ok(())
                } else {
                    Err(meta.error(
                        "unknown `str_newtype` option (expected `secret`, `serde`, `infallible`, `sqlx`, or `no_sqlx`)",
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
    if no_sqlx && secret {
        return Err(syn::Error::new_spanned(
            input,
            "a `secret` newtype is already bridge-less; `no_sqlx` is redundant/invalid",
        ));
    }
    if no_sqlx && sqlx {
        return Err(syn::Error::new_spanned(
            input,
            "`no_sqlx` is exclusive with `sqlx`",
        ));
    }
    if sqlx && !secret {
        return Err(syn::Error::new_spanned(
            input,
            "bare `sqlx` is only valid with `secret`; non-secret newtypes get the bridge by default — use `no_sqlx` to opt out",
        ));
    }
    let kind = if secret {
        Kind::Secret
    } else if infallible {
        Kind::Infallible
    } else {
        Kind::Default
    };
    Ok(Opts {
        kind,
        serde,
        sqlx,
        no_sqlx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlx_bridge::tests::norm;

    #[test]
    fn validating_bridge_decodes_a_borrowed_str_without_allocating() {
        let n = quote::format_ident!("Slug");
        let out = norm(&sqlx_impls(&n));
        assert!(out.contains("<&'rstras::sqlx::Decode<'r,DB>>::decode(value)?"));
        assert!(
            out.contains("::from_str(v)?"),
            "must parse the borrowed str directly"
        );
        assert!(
            !out.contains("::from_str(&v)"),
            "the &v form re-borrows an owned String"
        );
        assert!(!out.contains("to_owned"));
    }

    #[test]
    fn validating_bridge_keeps_string_for_type_and_encode() {
        let n = quote::format_ident!("Slug");
        let out = norm(&sqlx_impls(&n));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("letinner:&::std::string::String=&self.0;"));
    }

    #[test]
    fn infallible_bridge_is_untouched_on_all_three_inners() {
        // `PostBody`'s `From<String>` MOVES the value, so borrowing here would ADD an
        // allocation rather than remove one. Standing guard for the #758 boundary.
        let n = quote::format_ident!("PostBody");
        let out = norm(&sqlx_impls_infallible(&n));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("letinner:&::std::string::String=&self.0;"));
        assert!(out.contains("<::std::string::Stringas::sqlx::Decode<'r,DB>>::decode(value)?"));
        assert!(!out.contains("&'rstras::sqlx::Decode"));
    }
}
