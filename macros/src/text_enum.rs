//! Codegen for `#[text_enum(…)]` — the closed-string-enum convention (#746).
//!
//! An **attribute** rather than a derive, for one reason: a derive cannot add attributes
//! to its item. The convention needs `#[strum(parse_err_ty = …, parse_err_fn = …)]` to
//! name a parse fn this macro generates, and only an attribute macro can write that line
//! for the author instead of making them guess a generated ident.
//!
//! `strum` still does all the real work — the token mapping, `Display`, `FromStr`,
//! `VariantArray`, `EnumMessage`. This macro writes the derives the author would have
//! written, plus the pieces strum has no opinion about: the named parse error, the default
//! serde bridge (or `no_serde` for a separately derived representation), and (opt-in) the
//! sqlx bridge.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{DeriveInput, Expr, ExprLit, Lit, Meta, Path, Token};

/// The strum derives this macro writes for the author. Every adopting enum needs exactly
/// these, which is why they are injected rather than repeated eight times.
const UNIFORM_DERIVES: [&str; 4] = ["AsRefStr", "Display", "EnumString", "IntoStaticStr"];

/// The parsed `#[text_enum(…)]` arguments.
struct Opts {
    /// `sqlx` — emit the storage bridge. Off by default: most of these enums are
    /// wire-only, so "this one is stored" is worth declaring.
    sqlx: bool,
    /// `no_serde` — leave serialization to explicit derives when the enum's existing wire
    /// representation is intentionally different from its strum token grammar.
    no_serde: bool,
    /// `error = <Ident>` — the generated parse error's name. Stated rather than derived
    /// from the enum's name because it is public API: `host`'s `validation_from!`
    /// registers these across a crate boundary, so the name must be greppable here.
    error: syn::Ident,
    /// `message = "<literal>"` — the error's `Display` text.
    message: syn::LitStr,
}

/// Expands `#[text_enum(…)]`. On any error it emits the diagnostic **and** the original
/// item, so the user sees one clear error rather than a cascade of "cannot find type".
pub(crate) fn expand(attr: TokenStream, item: &TokenStream) -> TokenStream {
    let input: DeriveInput = match syn::parse2(item.clone()) {
        Ok(i) => i,
        Err(e) => return with_item(&e.to_compile_error(), item),
    };
    if let Err(e) = crate::require_enum_shape(&input, "text_enum", "enum X { A, B }") {
        return with_item(&e.to_compile_error(), item);
    }
    let opts = match parse_opts(attr) {
        Ok(o) => o,
        Err(e) => return with_item(&e.to_compile_error(), item),
    };
    if let Err(e) = reject_const_into_str(&input, &opts) {
        return with_item(&e.to_compile_error(), item);
    }

    let name = &input.ident;
    let parse_fn = format_ident!("__{}_parse_err", snake_case(name));
    let injected = injected_derives(&input);
    let error_ty = error_type(&opts.error, &opts.message, name);
    let serde = if opts.no_serde {
        quote! {}
    } else {
        serde_impls(name)
    };
    let sqlx = if opts.sqlx {
        sqlx_bridge(name)
    } else {
        quote! {}
    };
    let error = &opts.error;

    quote! {
        #injected
        #[strum(parse_err_ty = #error, parse_err_fn = #parse_fn)]
        #input

        #error_ty

        fn #parse_fn(_: &str) -> #error {
            #error
        }

        #serde
        #sqlx
    }
}

/// Pairs a diagnostic with the untouched item.
fn with_item(diagnostic: &TokenStream, item: &TokenStream) -> TokenStream {
    quote! { #diagnostic #item }
}

/// A `#[derive(::strum::…)]` naming whichever of [`UNIFORM_DERIVES`] the author has not
/// already written.
///
/// The paths are **fully qualified**, so an adopting crate must depend on `strum` under
/// that name — an unqualified `#[derive(AsRefStr)]` would only resolve if the author
/// happened to import it.
///
/// Suppression can only see attributes **below** `#[text_enum]`; anything above it has
/// already been expanded and stripped by the time this macro runs. That is why the
/// attribute must come first — a uniform derive written above it is invisible here and
/// collides as a duplicate impl.
fn injected_derives(input: &DeriveInput) -> TokenStream {
    let already: Vec<String> = input
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("derive"))
        .filter_map(|a| {
            a.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated)
                .ok()
        })
        .flatten()
        .filter_map(|p| strum_derive_name(&p))
        .collect();

    let missing = UNIFORM_DERIVES
        .iter()
        .filter(|d| !already.iter().any(|a| a == *d))
        .map(|d| {
            let ident = format_ident!("{}", d);
            quote! { ::strum::#ident }
        })
        .collect::<Vec<_>>();

    if missing.is_empty() {
        quote! {}
    } else {
        quote! { #[derive(#(#missing),*)] }
    }
}

/// The derive's name **if it could plausibly be strum's** — a bare `Display`, or a path
/// ending `strum::Display`. `None` for anything else.
///
/// Matching the last segment alone would be wrong: an author's
/// `#[derive(derive_more::Display)]` would silently suppress `::strum::Display`, and the
/// enum would quietly lose the token `Display` this macro exists to guarantee. Declining
/// to suppress instead lets the real conflict surface as a duplicate-impl error, which
/// is loud and points at the two derives that actually clash.
///
/// A bare `Display` still suppresses, because `use strum::Display` makes that spelling
/// legitimate and nothing here can tell the two apart.
fn strum_derive_name(path: &Path) -> Option<String> {
    let last = path.segments.last()?.ident.to_string();
    let qualifier = path
        .segments
        .len()
        .checked_sub(2)
        .map(|i| &path.segments[i]);
    match qualifier {
        None => Some(last),
        Some(q) if q.ident == "strum" => Some(last),
        Some(_) => None,
    }
}

/// The named parse error: a unit struct with a hand-written `Display` + `Error`.
///
/// Deliberately **not** `thiserror` — `num_newtype`'s error does the same, so an adopting
/// crate needs no extra dependency beyond the `strum` the injected derives already
/// require. The unit-struct shape is load-bearing: `host`'s `validation_from!` registers
/// these by name and its assertions construct them as bare unit expressions.
fn error_type(error: &syn::Ident, message: &syn::LitStr, enum_name: &syn::Ident) -> TokenStream {
    let doc = format!("Parse error for [`{enum_name}`]'s string token.");
    let message = quote! { #message };
    crate::public_unit_error_type(error, &doc, &message)
}

/// The serde bridge as direct impls: serialize the `&'static str` token (no allocation,
/// no clone), and deserialize an owned `String` routed through `FromStr`.
///
/// This is the same owned-`String`-through-`FromStr` path `#[serde(try_from = "String")]`
/// took, which is what keeps these types decodable as bare `serde_qs` form values.
fn serde_impls(name: &syn::Ident) -> TokenStream {
    quote! {
        #[automatically_derived]
        impl ::serde::Serialize for #name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::core::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(
                    <&#name as ::core::convert::Into<&'static str>>::into(self),
                )
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

/// The sqlx bridge for a stored token.
///
/// `Type` reports `String` — **not** `str` — because the generic storage impls bind on
/// `String: Type<DB>` and sqlx's blanket `Type for &T` runs the wrong way to bridge the
/// difference. `Encode` and `Decode` both borrow, so neither side allocates.
///
/// **The decode error echoes the offending token.** The generated parse error is a unit
/// struct that can only name the valid tokens, and a `ColumnDecode` labels the column, not
/// the value — so without this the report of a corrupt row is "must be one of …" with no
/// way to learn what the row actually holds. A `text_enum`'s domain is a closed set of
/// operator-facing tokens, never a secret, which is why the echo is safe here and is
/// deliberately *not* done in the `StrNewtype` bridge (that one also carries secrets).
fn sqlx_bridge(name: &syn::Ident) -> TokenStream {
    // Empty generics: `require_enum_shape` rejects a generic enum.
    let generics = syn::Generics::default();
    crate::sqlx_bridge::bridge(&crate::sqlx_bridge::BridgeSpec {
        name,
        generics: &generics,
        type_inner: quote! { ::std::string::String },
        encode_inner: quote! { &'q str },
        // `AsRef<str>` would tie the borrow to `&self`; the token is `&'static str`, which
        // coerces to the buffer's `'q` at the annotated local the bridge emits.
        to_inner: quote! { &<&#name as ::core::convert::Into<&'static str>>::into(self) },
        decode_inner: quote! { &'r str },
        convert: quote! {
            <#name as ::core::str::FromStr>::from_str(v).map_err(
                |e| -> ::sqlx::error::BoxDynError {
                    ::std::format!("{e}; stored value: {v:?}").into()
                },
            )
        },
        // `type_inner` is `String`, which has `PgHasArrayType`, so a slice binds as
        // `TEXT[]` (#891).
        pg_array: true,
    })
}

/// `PostFormat` -> `post_format`, for building the generated parse fn's name.
fn snake_case(ident: &syn::Ident) -> String {
    let s = ident.to_string();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `#[strum(const_into_str)]` suppresses the `From<&X> for &'static str` that the
/// generated serde and sqlx bridges read the token through. It is compatible with
/// `no_serde` when the sqlx bridge is also absent, because then only strum consumes the
/// static token conversion.
fn reject_const_into_str(input: &DeriveInput, opts: &Opts) -> syn::Result<()> {
    if opts.no_serde && !opts.sqlx {
        return Ok(());
    }

    for attr in &input.attrs {
        if !attr.path().is_ident("strum") {
            continue;
        }
        let mut found = false;
        // `parse_nested_meta` errors on strum's own `key = value` forms, which are not
        // ours to validate — only the flag matters here, so ignore anything else.
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("const_into_str") {
                found = true;
            }
            Ok(())
        });
        if found {
            return Err(syn::Error::new_spanned(
                attr,
                "`#[strum(const_into_str)]` is incompatible with `text_enum`: it suppresses \
                 the `From<&Self> for &'static str` that the serde and sqlx bridges read \
                 the token through",
            ));
        }
    }
    Ok(())
}

/// Reads `sqlx`, `no_serde`, `error = <Ident>`, and `message = "<literal>"`. `error` and
/// `message` are mandatory and come as a pair; anything else is a spanned error, so a typo
/// fails loudly rather than silently dropping a bridge.
fn parse_opts(attr: TokenStream) -> syn::Result<Opts> {
    let span = proc_macro2::Span::call_site();
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr)?;
    let mut sqlx = false;
    let mut no_serde = false;
    let mut error: Option<syn::Ident> = None;
    let mut message: Option<syn::LitStr> = None;

    for meta in &metas {
        match meta {
            Meta::Path(p) if p.is_ident("sqlx") => sqlx = true,
            Meta::Path(p) if p.is_ident("no_serde") => no_serde = true,
            Meta::NameValue(nv) if nv.path.is_ident("error") => {
                let Expr::Path(p) = &nv.value else {
                    return Err(syn::Error::new_spanned(
                        &nv.value,
                        "`error` must be a bare identifier, e.g. `error = InvalidPostFormat`",
                    ));
                };
                error = Some(p.path.require_ident()?.clone());
            }
            Meta::NameValue(nv) if nv.path.is_ident("message") => {
                let Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) = &nv.value
                else {
                    return Err(syn::Error::new_spanned(
                        &nv.value,
                        "`message` must be a string literal",
                    ));
                };
                message = Some(s.clone());
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "unknown `text_enum` option (expected `sqlx`, `no_serde`, \
                     `error = <Ident>`, or `message = \"…\"`)",
                ));
            }
        }
    }

    match (error, message) {
        (Some(error), Some(message)) => Ok(Opts {
            sqlx,
            no_serde,
            error,
            message,
        }),
        _ => Err(syn::Error::new(
            span,
            "`text_enum` requires both `error = <Ident>` and `message = \"…\"`; the named \
             parse error is public API and its text is the client-facing message",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlx_bridge::tests::{norm, norm_s};

    /// Parse `attr` and `item` from source text and run the expansion.
    fn expand_str(attr: &str, item: &str) -> TokenStream {
        expand(
            attr.parse().expect("attr must lex"),
            &item.parse().expect("item must lex"),
        )
    }

    #[test]
    fn missing_or_unpaired_options_are_spanned_errors() {
        for attr in [
            "",
            "error = InvalidX",
            r#"message = "bad""#,
            r#"error = InvalidX, message = "b", bogus"#,
        ] {
            let out = expand_str(attr, "pub enum X { A }").to_string();
            assert!(
                out.contains("compile_error"),
                "attr {attr:?} must be rejected"
            );
        }
    }

    #[test]
    fn an_item_that_is_not_a_type_definition_is_rejected() {
        let out = expand_str(r#"error = InvalidX, message = "b""#, "fn f() {}").to_string();
        assert!(out.contains("compile_error"));
    }

    #[test]
    fn error_must_be_an_identifier_not_a_literal() {
        let out =
            expand_str(r#"error = "InvalidX", message = "b""#, "pub enum X { A }").to_string();
        assert!(out.contains("compile_error"));
        assert!(out.contains("bare identifier"));
    }

    #[test]
    fn message_must_be_a_string_literal() {
        let out =
            expand_str("error = InvalidX, message = SomeIdent", "pub enum X { A }").to_string();
        assert!(out.contains("compile_error"));
        assert!(out.contains("string literal"));
    }

    #[test]
    fn injects_nothing_when_the_author_wrote_all_four() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "b""#,
            "#[derive(::strum::AsRefStr, ::strum::Display, ::strum::EnumString, \
             ::strum::IntoStaticStr)] pub enum X { A }",
        ));
        for d in [
            "::strum::AsRefStr",
            "::strum::Display",
            "::strum::EnumString",
            "::strum::IntoStaticStr",
        ] {
            assert_eq!(
                out.matches(&norm_s(d)).count(),
                1,
                "{d} must not be doubled"
            );
        }
    }

    #[test]
    fn the_parse_fn_name_is_snake_cased_from_a_multi_word_enum() {
        let out = norm(&expand_str(
            r#"error = InvalidPostFormat, message = "b""#,
            "pub enum PostFormat { Markdown }",
        ));
        assert!(out.contains("parse_err_fn=__post_format_parse_err"));
    }

    #[test]
    fn const_into_str_is_rejected() {
        let out = expand_str(
            r#"error = InvalidX, message = "b""#,
            r"#[strum(const_into_str)] pub enum X { A }",
        )
        .to_string();
        assert!(out.contains("compile_error"));
        assert!(out.contains("const_into_str"));
    }

    #[test]
    fn wrong_shape_is_a_spanned_error_naming_the_macro() {
        for item in ["pub struct S(String);", "pub enum X { A(u8) }"] {
            let out = expand_str(r#"error = InvalidX, message = "b""#, item).to_string();
            assert!(out.contains("compile_error"));
            assert!(out.contains("text_enum"));
        }
    }

    #[test]
    fn injects_the_four_uniform_derives_path_qualified() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "bad x""#,
            "pub enum X { A, B }",
        ));
        for d in [
            "::strum::AsRefStr",
            "::strum::Display",
            "::strum::EnumString",
            "::strum::IntoStaticStr",
        ] {
            assert!(
                out.contains(&norm_s(d)),
                "{d} must be injected, path-qualified"
            );
        }
    }

    #[test]
    fn injects_the_strum_parse_err_pair_naming_the_declared_error() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "bad x""#,
            "pub enum X { A }",
        ));
        assert!(out.contains("parse_err_ty=InvalidX"));
        assert!(out.contains("parse_err_fn=__x_parse_err"));
    }

    #[test]
    fn generates_a_unit_error_matching_the_num_newtype_precedent() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "bad x""#,
            "pub enum X { A }",
        ));
        assert!(
            out.contains("pubstructInvalidX;"),
            "must be a bare unit struct"
        );
        assert!(out.contains(&norm_s(
            "#[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy, \
             ::core::cmp::PartialEq, ::core::cmp::Eq)]"
        )));
        assert!(out.contains("::core::fmt::DisplayforInvalidX"));
        assert!(out.contains("f.write_str(\"badx\")"));
        assert!(out.contains("::std::error::ErrorforInvalidX"));
        assert!(out.contains("fn__x_parse_err(_:&str)->InvalidX"));
        assert!(out.contains("InvalidX}"));
        assert!(
            !out.contains("thiserror"),
            "must not require a thiserror dependency"
        );
        assert!(
            out.contains("\"badx\""),
            "norm strips the space inside the literal"
        );
    }

    #[test]
    fn preserves_author_attributes_and_derives() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "bad x""#,
            r#"#[derive(Clone, Copy, ::strum::VariantArray)]
               #[strum(serialize_all = "snake_case")]
               pub enum X { A }"#,
        ));
        assert!(out.contains("::strum::VariantArray"));
        assert!(out.contains("serialize_all=\"snake_case\""));
    }

    #[test]
    fn a_same_named_derive_from_another_crate_does_not_suppress_strums() {
        // `derive_more::Display` is not strum's; suppressing on the last segment alone
        // would silently drop the token `Display` this macro exists to guarantee.
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "b""#,
            "#[derive(derive_more::Display)] pub enum X { A }",
        ));
        assert!(
            out.contains(&norm_s("::strum::Display")),
            "strum's Display must still be injected"
        );
    }

    #[test]
    fn a_strum_qualified_derive_does_suppress() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "b""#,
            "#[derive(strum::Display)] pub enum X { A }",
        ));
        assert!(!out.contains(&norm_s("::strum::Display")));
    }

    #[test]
    fn does_not_duplicate_a_uniform_derive_written_below_the_attribute() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "bad x""#,
            "#[derive(::strum::Display)] pub enum X { A }",
        ));
        assert_eq!(
            out.matches("::strum::Display").count(),
            1,
            "a derive the author already wrote must not be injected again"
        );
    }

    #[test]
    fn serialize_writes_the_static_token_without_allocating() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "b""#,
            "pub enum X { A }",
        ));
        assert!(out.contains("serializer.serialize_str"));
        assert!(out.contains("<&Xas::core::convert::Into<&'staticstr>>::into(self)"));
        assert!(!out.contains("to_owned"));
        assert!(!out.contains("clone()"));
    }

    #[test]
    fn deserialize_routes_an_owned_string_through_from_str() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "b""#,
            "pub enum X { A }",
        ));
        assert!(
            out.contains(
                "<::std::string::Stringas::serde::Deserialize>::deserialize(deserializer)?"
            )
        );
        assert!(out.contains("::from_str(&s).map_err(::serde::de::Error::custom)"));
    }

    #[test]
    fn sqlx_flag_emits_the_bridge_with_the_three_declared_inners() {
        let out = norm(&expand_str(
            r#"sqlx, error = InvalidX, message = "b""#,
            "pub enum X { A }",
        ));
        // Type delegates to String, NOT str — storage binds `String: Type<DB>`.
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(!out.contains("<stras::sqlx::Type"));
        // Encode borrows a 'static token; the `&&` is a reference to the `&'q str` local.
        assert!(
            out.contains("letinner:&&'qstr=&<&Xas::core::convert::Into<&'staticstr>>::into(self);")
        );
        assert!(out.contains("<&'qstras::sqlx::Encode<'q,DB>>::encode_by_ref(inner,buf)"));
        assert!(!out.contains("to_owned"));
        // Decode borrows.
        assert!(out.contains("<&'rstras::sqlx::Decode<'r,DB>>::decode(value)?"));
        assert!(out.contains("::from_str(v).map_err"));
    }

    #[test]
    fn the_decode_error_echoes_the_offending_token() {
        let out = norm(&expand_str(
            r#"sqlx, error = InvalidX, message = "b""#,
            "pub enum X { A }",
        ));
        // `norm` strips spaces inside literals too, so match the stripped form.
        assert!(
            out.contains(norm_s(r#"::std::format!("{e}; stored value: {v:?}")"#).as_str()),
            "a corrupt row must report what it holds, not just the valid tokens"
        );
    }

    #[test]
    fn without_the_sqlx_flag_no_bridge_is_emitted() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "b""#,
            "pub enum X { A }",
        ));
        assert!(!out.contains("::sqlx::"));
    }

    #[test]
    fn no_serde_omits_only_the_serde_bridge() {
        let out = norm(&expand_str(
            r#"no_serde, error = InvalidX, message = "b""#,
            "pub enum X { A }",
        ));
        assert!(
            !out.contains("::serde::"),
            "`no_serde` must suppress the generated serde impls"
        );
        for token in [
            "::strum::AsRefStr",
            "::strum::Display",
            "::strum::EnumString",
            "::strum::IntoStaticStr",
            "parse_err_ty=InvalidX",
            "fn__x_parse_err(_:&str)->InvalidX",
        ] {
            assert!(
                out.contains(&norm_s(token)),
                "`no_serde` must retain {token}"
            );
        }
    }

    #[test]
    fn no_serde_can_coexist_with_the_sqlx_bridge() {
        let out = norm(&expand_str(
            r#"no_serde, sqlx, error = InvalidX, message = "b""#,
            "pub enum X { A }",
        ));
        assert!(!out.contains("::serde::"));
        assert!(out.contains("::sqlx::"));
    }

    #[test]
    fn no_serde_without_sqlx_allows_const_into_str() {
        let out = expand_str(
            r#"no_serde, error = InvalidX, message = "b""#,
            r"#[strum(const_into_str)] pub enum X { A }",
        )
        .to_string();
        assert!(!out.contains("compile_error"));
    }

    #[test]
    fn default_options_still_emit_the_serde_bridge() {
        let out = norm(&expand_str(
            r#"error = InvalidX, message = "b""#,
            "pub enum X { A }",
        ));
        assert!(
            out.contains("::serde::Serialize"),
            "serde remains the default macro surface"
        );
        assert!(out.contains("::serde::Deserialize"));
    }
}
