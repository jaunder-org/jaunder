//! Codegen for `#[derive(StrEnum)]` — the string-**enum** trailer, the closed-set analogue
//! of the ADR-0063 newtype trailers. For a unit-variant enum whose values are a fixed set of
//! lowercase wire/DB tokens, it generates `as_str`, `Display`, `FromStr` **and**
//! `TryFrom<&str>` routed through a self-contained `Invalid<Name>` error, and — under
//! `#[str_enum(serde)]` — a string serde bridge (serialize `as_str`; deserialize an owned
//! `String` via `FromStr`, so the `serde_qs` form transport works). The wire token defaults to
//! the `snake_case` of the variant identifier, per-variant overridable with
//! `#[str_enum(rename = "…")]`.
//! `Default`/`Copy`/`Hash`/… stay in the user's `#[derive]` list. Modelled on `num_newtype`
//! (self-contained error, no `thiserror`).

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Ident, LitStr};

/// Parsed type-level `#[str_enum(...)]` options. `serde` opts into the serde bridge; `error`
/// overrides the generated `Display` message.
struct Opts {
    serde: bool,
    error: Option<LitStr>,
}

/// A variant paired with its resolved wire literal.
struct WireVariant {
    ident: Ident,
    wire: String,
}

/// Expands `#[derive(StrEnum)]`. On the wrong shape (not a non-generic unit-variant enum, or
/// empty), an unknown option, or two variants resolving to the same wire literal, returns a
/// spanned `compile_error!` instead of malformed impls.
pub(crate) fn expand(input: &DeriveInput) -> TokenStream {
    let variants = match collect_variants(input) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error(),
    };
    let opts = match parse_opts(input) {
        Ok(o) => o,
        Err(e) => return e.to_compile_error(),
    };

    let name = &input.ident;
    let err_name = quote::format_ident!("Invalid{}", name);
    let idents: Vec<&Ident> = variants.iter().map(|v| &v.ident).collect();
    let wires: Vec<&str> = variants.iter().map(|v| v.wire.as_str()).collect();

    let error_ty = error_type(&err_name, &opts.message(&wires));
    let as_str = as_str_impl(name, &idents, &wires);
    let display = display_impl(name);
    let from_str = from_str_impl(name, &err_name, &idents, &wires);
    let try_from = try_from_impl(name, &err_name);
    let serde = if opts.serde {
        serde_impl(name)
    } else {
        quote! {}
    };

    quote! {
        #error_ty
        #as_str
        #display
        #from_str
        #try_from
        #serde
    }
}

/// Validates the non-generic unit-variant-enum shape and resolves each variant's wire literal
/// (the `#[str_enum(rename = "…")]` override, else the `snake_case` identifier). Rejects a
/// non-enum, a generic enum, an empty enum, a fielded variant, an unknown variant-level
/// `str_enum` key, or two variants that resolve to the same wire literal.
fn collect_variants(input: &DeriveInput) -> syn::Result<Vec<WireVariant>> {
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            input,
            "StrEnum requires a unit-variant enum like `enum X { A, B }`",
        ));
    };
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            input,
            "StrEnum does not support generic enums",
        ));
    }
    if data.variants.is_empty() {
        return Err(syn::Error::new_spanned(
            input,
            "StrEnum requires at least one variant",
        ));
    }

    let mut variants: Vec<WireVariant> = Vec::new();
    for v in &data.variants {
        if !matches!(v.fields, syn::Fields::Unit) {
            return Err(syn::Error::new_spanned(
                v,
                "StrEnum requires unit variants (no fields)",
            ));
        }
        let wire = variant_wire(v)?;
        if wire.is_empty() {
            return Err(syn::Error::new_spanned(
                v,
                "StrEnum wire token must not be empty (an empty `#[str_enum(rename = \"\")]`)",
            ));
        }
        if let Some(dup) = variants.iter().find(|e| e.wire == wire) {
            return Err(syn::Error::new_spanned(
                v,
                format!(
                    "StrEnum wire string {:?} is shared by `{}` and `{}`",
                    wire, dup.ident, v.ident
                ),
            ));
        }
        variants.push(WireVariant {
            ident: v.ident.clone(),
            wire,
        });
    }
    Ok(variants)
}

/// The variant's wire literal: `#[str_enum(rename = "…")]` if present, else the `snake_case`
/// identifier. Non-`str_enum` attributes (e.g. std `#[default]`) are ignored; an unknown
/// variant-level `str_enum` key is a spanned error.
fn variant_wire(v: &syn::Variant) -> syn::Result<String> {
    let mut rename: Option<String> = None;
    for attr in &v.attrs {
        if attr.path().is_ident("str_enum") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename") {
                    rename = Some(meta.value()?.parse::<LitStr>()?.value());
                    Ok(())
                } else {
                    Err(meta.error("unknown variant `str_enum` option (expected `rename`)"))
                }
            })?;
        } // cov:ignore `?`-fall-through closing brace (mirrors num_newtype)
    }
    Ok(rename.unwrap_or_else(|| to_snake_case(&v.ident.to_string())))
}

/// Converts a `CamelCase` variant identifier to its `snake_case` wire token: lowercase
/// everything and insert `_` before each uppercase letter that isn't first (`InviteOnly` ->
/// `invite_only`, `Open` -> `open`; a single-word identifier is just lowercased). Consecutive
/// capitals snake-case one underscore per letter (`HTMLPage` -> `h_t_m_l_page`); override an
/// acronym with `#[str_enum(rename = "…")]`.
fn to_snake_case(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len() + 4);
    for (i, ch) in ident.char_indices() {
        if i != 0 && ch.is_uppercase() {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

/// The self-contained error type: a hand-written `Display` + `Error` (no `thiserror`), so any
/// adopter crate needs no extra dependency. `Debug`/`PartialEq` let callers `assert_eq!` on a
/// `TryFrom`/`FromStr` result.
fn error_type(err_name: &Ident, message: &TokenStream) -> TokenStream {
    quote! {
        #[doc = "Error returned when a string matches no variant of the enum."]
        #[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy, ::core::cmp::PartialEq, ::core::cmp::Eq)]
        pub struct #err_name;

        #[automatically_derived]
        impl ::core::fmt::Display for #err_name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(#message)
            }
        }

        #[automatically_derived]
        impl ::std::error::Error for #err_name {}
    }
}

/// `impl X { pub fn as_str(&self) -> &'static str }` — the variant→token map, the single
/// source of truth the other impls route through.
fn as_str_impl(name: &Ident, idents: &[&Ident], wires: &[&str]) -> TokenStream {
    quote! {
        impl #name {
            #[doc = "The wire/DB token for this variant."]
            #[must_use]
            pub fn as_str(&self) -> &'static str {
                match self {
                    #(Self::#idents => #wires,)*
                }
            }
        }
    }
}

/// `Display` writes the variant's token via `as_str`.
fn display_impl(name: &Ident) -> TokenStream {
    quote! {
        #[automatically_derived]
        impl ::core::fmt::Display for #name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    }
}

/// The single validating chokepoint: match the token back to a variant, else the error.
fn from_str_impl(name: &Ident, err_name: &Ident, idents: &[&Ident], wires: &[&str]) -> TokenStream {
    quote! {
        #[automatically_derived]
        impl ::core::str::FromStr for #name {
            type Err = #err_name;
            fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {
                match s {
                    #(#wires => ::core::result::Result::Ok(Self::#idents),)*
                    _ => ::core::result::Result::Err(#err_name),
                }
            }
        }
    }
}

/// `TryFrom<&str>` routes through `FromStr` (no `From<&str>` is emitted, so there is no
/// blanket-impl conflict with `impl<T, U: Into<T>> TryFrom<U>`).
fn try_from_impl(name: &Ident, err_name: &Ident) -> TokenStream {
    quote! {
        #[automatically_derived]
        impl ::core::convert::TryFrom<&str> for #name {
            type Error = #err_name;
            fn try_from(s: &str) -> ::core::result::Result<Self, Self::Error> {
                <Self as ::core::str::FromStr>::from_str(s)
            }
        }
    }
}

/// The string serde bridge: serialize the token; deserialize an **owned** `String` (so
/// `serde_qs` form decoding works) and route it through `FromStr`, mapping failure to a serde
/// custom error carrying the generated `Invalid<Name>` message.
fn serde_impl(name: &Ident) -> TokenStream {
    quote! {
        #[automatically_derived]
        impl ::serde::Serialize for #name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::core::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        #[automatically_derived]
        impl<'de> ::serde::Deserialize<'de> for #name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::core::result::Result<Self, D::Error> {
                let s = <::std::string::String as ::serde::Deserialize>::deserialize(deserializer)?;
                <Self as ::core::str::FromStr>::from_str(&s).map_err(::serde::de::Error::custom)
            }
        }
    }
}

/// Reads type-level `#[str_enum(serde, error = "…")]`. `serde` is a bare flag; `error`
/// overrides the generated message. An unknown key is a spanned error.
fn parse_opts(input: &DeriveInput) -> syn::Result<Opts> {
    let mut serde = false;
    let mut error: Option<LitStr> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("str_enum") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("serde") {
                    serde = true;
                    Ok(())
                } else if meta.path.is_ident("error") {
                    error = Some(meta.value()?.parse()?);
                    Ok(())
                } else {
                    Err(meta.error("unknown `str_enum` option (expected `serde` or `error`)"))
                }
            })?;
        } // cov:ignore `?`-fall-through closing brace (mirrors num_newtype)
    }
    Ok(Opts { serde, error })
}

impl Opts {
    /// The `Display` message: the explicit `error = "..."`, else `must be one of: a, b, c`.
    fn message(&self, wires: &[&str]) -> TokenStream {
        if let Some(lit) = &self.error {
            return quote! { #lit };
        }
        let msg = format!("must be one of: {}", wires.join(", "));
        quote! { #msg }
    }
}
