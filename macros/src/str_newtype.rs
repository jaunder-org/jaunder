//! Codegen for `#[derive(StrNewtype)]` — the ADR-0063 string-newtype trailer for a
//! `struct X(String)` or, since #875, a phantom-tagged `struct X<T: Bound>(String,
//! PhantomData<fn() -> T>)`, whose generics are threaded through every emitted impl.
//! The derive owns the whole trailer except `FromStr` (the one per-type
//! validating/normalizing chokepoint) and the std `#[derive]`s — except ordering, which
//! it emits (#761) unless `#[str_newtype(no_ord)]` suppresses it.

use quote::quote;
use syn::DeriveInput;

/// How the sqlx storage bridge is selected — the `sqlx`/`no_sqlx` flags collapsed into one
/// field, so "both at once" is unrepresentable in [`Opts`] rather than merely rejected.
enum SqlxMode {
    /// No explicit flag — the default-on-except-secret policy applies.
    Default,
    /// `#[str_newtype(sqlx)]`: re-add the bridge to a secret that genuinely is stored
    /// (`InviteCode`).
    Forced,
    /// `#[str_newtype(no_sqlx)]`: opt a non-secret must-not-store type out (`RawToken`).
    Off,
}

/// The `#[str_newtype(...)]` options: whether the type is a secret, whether a secret
/// re-opens the serde bridge (`secret, serde`) for an inbound wire value, and the sqlx
/// bridge control. The sqlx bridge (feature-gated `Type`/`Encode`/`Decode`) is **on by
/// default** for every non-secret type and dropped for a `secret` one; [`SqlxMode`] carries
/// the two overrides.
struct Opts {
    secret: bool,
    serde: bool,
    sqlx: SqlxMode,
    /// Whether the author opted *out* of the ordering half of the trailer (#761).
    ord: bool,
}

/// Expands `#[derive(StrNewtype)]` on a single-field tuple struct. On the wrong shape
/// (or an unknown/invalid `str_newtype` option) it returns a spanned `compile_error!`
/// instead of malformed impls. `#[str_newtype(secret)]` selects the tight secret
/// surface; `#[str_newtype(secret, serde)]` adds the serde bridge back to it.
pub(crate) fn expand(input: &DeriveInput) -> proc_macro2::TokenStream {
    if let Err(e) = crate::require_newtype_shape(
        input,
        crate::NewtypeShape::PhantomTagged,
        "StrNewtype",
        "struct X(String)",
    ) {
        return e.to_compile_error();
    }
    let opts = match parse_opts(input) {
        Ok(o) => o,
        Err(e) => return e.to_compile_error(),
    };
    let name = &input.ident;
    // The user's generics, threaded through every emitted impl (#875). Nothing the derive
    // emits ever constructs `Self`, so the phantom marker field never has to be named:
    // `TryFrom`, `Deserialize`, and the sqlx `Decode` all route through the author's
    // `FromStr`, which is where the `PhantomData` is supplied.
    let generics = &input.generics;

    // The sqlx storage bridge is selected once from the secret/default policy.
    let sqlx = sqlx_bridge(&opts, name, generics);
    // The ordering half (#761), suppressed by `no_ord` and never emitted for a secret.
    let ord = if opts.ord {
        crate::ord_impls(name, generics)
    } else {
        quote! {}
    };
    if opts.secret {
        // The tight secret surface; `secret, serde` re-opens the serde bridge for an
        // inbound wire value (its inbound-only role is enforced by an xtask gate).
        let trailer = secret_trailer(name, generics);
        let serde = if opts.serde {
            serde_impls(name, generics)
        } else {
            quote! {}
        };
        quote! {
            #trailer
            #serde
            #sqlx
        }
    } else {
        // The full ergonomic trailer plus the validating serde and sqlx bridges.
        let trailer = default_trailer(name, generics);
        let serde = serde_impls(name, generics);
        quote! {
            #trailer
            #serde
            #ord
            #sqlx
        }
    }
}

/// The sqlx storage bridge for `name`, per the default-on-except-secret policy: a
/// non-secret type gets it unless `no_sqlx` opts out (`RawToken`); a `secret` gets it
/// only with an explicit `sqlx` (a stored secret, `InviteCode`). Every bridge validates
/// through `FromStr`.
fn sqlx_bridge(
    opts: &Opts,
    name: &syn::Ident,
    generics: &syn::Generics,
) -> proc_macro2::TokenStream {
    if opts.secret {
        if matches!(opts.sqlx, SqlxMode::Forced) {
            sqlx_impls(name, generics)
        } else {
            quote! {}
        }
    } else if matches!(opts.sqlx, SqlxMode::Off) {
        quote! {}
    } else {
        sqlx_impls(name, generics)
    }
}

/// The full **ergonomic default trailer**: `Display`, `AsRef`/`Borrow`/`Deref<str>`,
/// `TryFrom<String>` (the fallible door, via `FromStr`), `From<Self> for String`, and
/// `PartialEq<str>`/`<&str>`. The serde and sqlx bridges are appended by [`expand`].
fn default_trailer(name: &syn::Ident, generics: &syn::Generics) -> proc_macro2::TokenStream {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        #[automatically_derived]
        impl #impl_generics ::core::fmt::Display for #name #ty_generics #where_clause {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(&self.0)
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::convert::AsRef<str> for #name #ty_generics #where_clause {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::borrow::Borrow<str> for #name #ty_generics #where_clause {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::ops::Deref for #name #ty_generics #where_clause {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::convert::TryFrom<::std::string::String>
            for #name #ty_generics #where_clause
        {
            type Error = <#name #ty_generics as ::core::str::FromStr>::Err;
            fn try_from(s: ::std::string::String) -> ::core::result::Result<Self, Self::Error> {
                <#name #ty_generics as ::core::str::FromStr>::from_str(&s)
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::convert::From<#name #ty_generics>
            for ::std::string::String #where_clause
        {
            fn from(v: #name #ty_generics) -> Self {
                v.0
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::cmp::PartialEq<str> for #name #ty_generics #where_clause {
            fn eq(&self, other: &str) -> bool {
                self.0 == *other
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::cmp::PartialEq<&str> for #name #ty_generics #where_clause {
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
fn serde_impls(name: &syn::Ident, generics: &syn::Generics) -> proc_macro2::TokenStream {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    // `Deserialize` introduces `'de`, so its header is the user's generics with that
    // lifetime merged in; `ty_generics`/`where_clause` still come from the original.
    let de = crate::with_leading_param(generics, syn::parse_quote!('de));
    let (de_impl_generics, _, _) = de.split_for_impl();
    quote! {
        #[automatically_derived]
        impl #impl_generics ::serde::Serialize for #name #ty_generics #where_clause {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::core::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        #[automatically_derived]
        impl #de_impl_generics ::serde::Deserialize<'de> for #name #ty_generics #where_clause {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::core::result::Result<Self, D::Error> {
                let s = <::std::string::String as ::serde::Deserialize>::deserialize(deserializer)?;
                <#name #ty_generics as ::core::str::FromStr>::from_str(&s)
                    .map_err(::serde::de::Error::custom)
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
fn sqlx_impls(name: &syn::Ident, generics: &syn::Generics) -> proc_macro2::TokenStream {
    let (_, ty_generics, _) = generics.split_for_impl();
    crate::sqlx_bridge::bridge(&crate::sqlx_bridge::BridgeSpec {
        name,
        generics,
        type_inner: sqlx_inner(),
        encode_inner: sqlx_inner(),
        to_inner: quote! { &self.0 },
        // Borrowed: `FromStr` parses from a `&str` and builds its own `String`, so
        // decoding an owned one here would allocate it only to drop it (#746 D3).
        decode_inner: quote! { &'r str },
        convert: quote! {
            ::core::result::Result::Ok(<#name #ty_generics as ::core::str::FromStr>::from_str(v)?)
        },
        // `String: PgHasArrayType`, so a slice binds as `TEXT[]` (#891).
        pg_array: true,
    })
}

/// The **tight secret surface** (ADR-0063 secret exception, as amended by #403): a
/// redacting `Debug`, explicit borrowed access via `AsRef<str>`, and construction via
/// `TryFrom<String>` — and deliberately *none* of `Display`, `Deref`, `Borrow`,
/// `From<Self> for String`, or `PartialEq`, so a secret cannot leak or be value-compared.
/// `#[str_newtype(secret, serde)]` layers the serde bridge back on for an inbound value.
fn secret_trailer(name: &syn::Ident, generics: &syn::Generics) -> proc_macro2::TokenStream {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        #[automatically_derived]
        impl #impl_generics ::core::fmt::Debug for #name #ty_generics #where_clause {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(concat!(stringify!(#name), "([redacted])"))
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::convert::AsRef<str> for #name #ty_generics #where_clause {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        #[automatically_derived]
        impl #impl_generics ::core::convert::TryFrom<::std::string::String>
            for #name #ty_generics #where_clause
        {
            type Error = <#name #ty_generics as ::core::str::FromStr>::Err;
            fn try_from(s: ::std::string::String) -> ::core::result::Result<Self, Self::Error> {
                <#name #ty_generics as ::core::str::FromStr>::from_str(&s)
            }
        }
    }
}

/// Reads the five `#[str_newtype(...)]` options — `secret`, `serde`, `sqlx`, `no_sqlx`,
/// and `no_ord`. Errors on any other so a typo fails loudly rather than silently
/// un-redacting, and on combinations that are contradictory or redundant.
fn parse_opts(input: &DeriveInput) -> syn::Result<Opts> {
    let mut secret = false;
    let mut serde = false;
    let mut sqlx = false;
    let mut no_sqlx = false;
    let mut no_ord = false;
    for attr in &input.attrs {
        if attr.path().is_ident("str_newtype") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("secret") {
                    secret = true;
                    Ok(())
                } else if meta.path.is_ident("serde") {
                    serde = true;
                    Ok(())
                } else if meta.path.is_ident("sqlx") {
                    sqlx = true;
                    Ok(())
                } else if meta.path.is_ident("no_sqlx") {
                    no_sqlx = true;
                    Ok(())
                } else if meta.path.is_ident("no_ord") {
                    no_ord = true;
                    Ok(())
                } else {
                    Err(meta.error(
                        "unknown `str_newtype` option (expected `secret`, `serde`, `sqlx`, `no_sqlx`, or `no_ord`)",
                    ))
                }
            })?;
        } // cov:ignore `?`-fall-through closing brace; executed by the secret unit tests but llvm-cov leaves the gap region unmarked
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
    if no_ord && secret {
        return Err(syn::Error::new_spanned(
            input,
            "a `secret` newtype is already unordered; `no_ord` is redundant/invalid",
        ));
    }
    if sqlx && !secret {
        return Err(syn::Error::new_spanned(
            input,
            "bare `sqlx` is only valid with `secret`; non-secret newtypes get the bridge by default — use `no_sqlx` to opt out",
        ));
    }
    let sqlx = if sqlx {
        SqlxMode::Forced
    } else if no_sqlx {
        SqlxMode::Off
    } else {
        SqlxMode::Default
    };
    Ok(Opts {
        secret,
        serde,
        sqlx,
        ord: !no_ord,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlx_bridge::tests::norm;

    #[test]
    fn validating_bridge_decodes_a_borrowed_str_without_allocating() {
        let n = quote::format_ident!("Slug");
        let out = norm(&sqlx_impls(&n, &syn::Generics::default()));
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
        let out = norm(&sqlx_impls(&n, &syn::Generics::default()));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("letinner:&::std::string::String=&self.0;"));
    }
}
