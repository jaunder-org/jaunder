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

    if matches!(opts.kind, Kind::Secret) {
        let trailer = secret_trailer(name);
        // A secret re-opens serde only when it must cross the wire *inbound*
        // (`secret, serde`); its inbound-only role is enforced by an xtask gate.
        let serde = if opts.serde {
            serde_impls(name)
        } else {
            quote! {}
        };
        // A secret is bridge-less by default (a secret must not be storable);
        // `secret, sqlx` re-adds the validating sqlx bridge to one that genuinely
        // is stored (`InviteCode`).
        let sqlx = if opts.sqlx {
            sqlx_impls(name)
        } else {
            quote! {}
        };
        return quote! {
            #trailer
            #serde
            #sqlx
        };
    }

    if matches!(opts.kind, Kind::Infallible) {
        let trailer = infallible_trailer(name);
        // The infallible sqlx bridge is on by default (infallible types are stored);
        // `no_sqlx` opts out.
        let sqlx = if opts.no_sqlx {
            quote! {}
        } else {
            sqlx_impls_infallible(name)
        };
        return quote! {
            #trailer
            #sqlx
        };
    }

    let serde = serde_impls(name);
    // The validating sqlx bridge is on by default for a non-secret type; `no_sqlx`
    // opts a must-not-store type out (`RawToken`).
    let sqlx = if opts.no_sqlx {
        quote! {}
    } else {
        sqlx_impls(name)
    };
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
        #sqlx
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

/// The **validating sqlx bridge**: generic `Type`/`Encode`/`Decode` impls that make the
/// newtype a first-class TEXT column, delegating to the inner `String` (mirroring what
/// `#[derive(sqlx::Type)] #[sqlx(transparent)]` expands to). `Decode` decodes a `String`
/// then routes it through `<#name as FromStr>::from_str`, so a corrupted/migrated column
/// is rejected rather than silently admitted; the `?` folds the `FromStr::Err` (all our
/// newtype errors derive `thiserror::Error`) into `sqlx::error::BoxDynError`. All items
/// are stripped when the `sqlx` feature is off — the proc-macro crate never depends on sqlx.
fn sqlx_impls(name: &syn::Ident) -> proc_macro2::TokenStream {
    let convert = quote! {
        ::core::result::Result::Ok(<#name as ::core::str::FromStr>::from_str(&s)?)
    };
    sqlx_impls_inner(name, &convert)
}

/// The **infallible sqlx bridge**: as `sqlx_impls`, but `Decode` wraps the decoded
/// `String` via the type's infallible `From<String>` (no validation to run).
fn sqlx_impls_infallible(name: &syn::Ident) -> proc_macro2::TokenStream {
    let convert = quote! {
        ::core::result::Result::Ok(
            <#name as ::core::convert::From<::std::string::String>>::from(s),
        )
    };
    sqlx_impls_inner(name, &convert)
}

/// Shared body of the two sqlx bridges: identical `Type`/`Encode` delegation to the inner
/// `String`, parameterized only by how `Decode` turns the decoded `String` into `Self`
/// (`convert` names a bound local `s: String` and yields `Result<Self, BoxDynError>`).
fn sqlx_impls_inner(
    name: &syn::Ident,
    convert: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    quote! {
        #[cfg(feature = "sqlx")]
        const _: () = {
            #[automatically_derived]
            impl<DB: ::sqlx::Database> ::sqlx::Type<DB> for #name
            where
                ::std::string::String: ::sqlx::Type<DB>,
            {
                fn type_info() -> <DB as ::sqlx::Database>::TypeInfo {
                    <::std::string::String as ::sqlx::Type<DB>>::type_info()
                }
                fn compatible(ty: &<DB as ::sqlx::Database>::TypeInfo) -> bool {
                    <::std::string::String as ::sqlx::Type<DB>>::compatible(ty)
                }
            }

            #[automatically_derived]
            impl<'q, DB: ::sqlx::Database> ::sqlx::Encode<'q, DB> for #name
            where
                ::std::string::String: ::sqlx::Encode<'q, DB>,
            {
                fn encode_by_ref(
                    &self,
                    buf: &mut <DB as ::sqlx::Database>::ArgumentBuffer<'q>,
                ) -> ::core::result::Result<::sqlx::encode::IsNull, ::sqlx::error::BoxDynError>
                {
                    <::std::string::String as ::sqlx::Encode<'q, DB>>::encode_by_ref(&self.0, buf)
                }
                fn size_hint(&self) -> usize {
                    <::std::string::String as ::sqlx::Encode<'q, DB>>::size_hint(&self.0)
                }
            }

            #[automatically_derived]
            impl<'r, DB: ::sqlx::Database> ::sqlx::Decode<'r, DB> for #name
            where
                ::std::string::String: ::sqlx::Decode<'r, DB>,
            {
                fn decode(
                    value: <DB as ::sqlx::Database>::ValueRef<'r>,
                ) -> ::core::result::Result<Self, ::sqlx::error::BoxDynError> {
                    let s = <::std::string::String as ::sqlx::Decode<'r, DB>>::decode(value)?;
                    #convert
                }
            }
        };
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
