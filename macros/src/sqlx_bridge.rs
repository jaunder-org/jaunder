//! The shared sqlx storage bridge (ADR-0071), emitted by every derive and attribute
//! macro in this crate that makes a type a first-class column.
//!
//! Three of the four impls are pure delegation, but *what* they delegate to is no
//! longer one type: `Type`, `Encode`, and `Decode` each take their own inner. That
//! split is what lets a `FromStr`-based decode borrow `&'r str` while its `Type` still
//! reports `String` (see [`BridgeSpec`]). The fourth, `PgHasArrayType`, delegates to
//! the `Type` inner and is **opt-in** — see the `pg_array` column. Callers differ only
//! in the fields they fill:
//!
//! | Caller                    | Type     | Encode    | Decode    | `Decode` conversion             | `pg_array` |
//! | ------------------------- | -------- | --------- | --------- | ------------------------------- | ---------- |
//! | `StrNewtype` (validating) | `String` | `String`  | `&'r str` | validating `FromStr`            | yes        |
//! | `IdNewtype`               | `i64`    | `i64`     | `i64`     | infallible wrap                 | yes        |
//! | `NumNewtype`              | declared | declared  | declared  | bound-checking `TryFrom<inner>` | **no**     |
//! | `SqlxBridge`              | field ty | field ty  | field ty  | infallible wrap (moves)         | **no**     |
//! | `#[text_enum(sqlx)]`      | `String` | `&'q str` | `&'r str` | validating `FromStr`            | yes        |
//!
//! `NumNewtype` and `SqlxBridge` are off because their inner is caller-declared, and
//! sqlx implements `PgHasArrayType` for `i32`/`i64`/`String` only. The array impl is
//! concrete rather than generic over `DB`, so an inner without it is `E0277` **at the
//! impl** — not a deferred bound. Turning `NumNewtype` on stops `common` compiling on
//! its `u32`/`usize` newtypes (#891).
//!
//! The rule is per-conversion: a decode that borrows to parse and drops takes `&'r str`;
//! a decode that moves the decoded value into a new value keeps the owned inner, where
//! borrowing would add an allocation rather than remove one.
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
///
/// `generics` is the *user's* generics, threaded through all three impls (#875). Each impl
/// introduces a parameter of its own, so the emitted header is the user's list with `DB`
/// pushed on and `'q`/`'r` prepended; every caller but `StrNewtype` passes an empty one.
pub(crate) struct BridgeSpec<'a> {
    pub(crate) name: &'a syn::Ident,
    pub(crate) generics: &'a syn::Generics,
    pub(crate) type_inner: TokenStream,
    pub(crate) encode_inner: TokenStream,
    pub(crate) to_inner: TokenStream,
    pub(crate) decode_inner: TokenStream,
    pub(crate) convert: TokenStream,
    /// Emit `PgHasArrayType`, so a slice of this newtype binds as a Postgres array.
    ///
    /// Opt-in per caller rather than universal, and **deliberately not a `where`
    /// clause**: this impl is concrete (unlike the three above, which are generic over
    /// `DB` and defer their bounds to use sites), so `where #type_inner:
    /// PgHasArrayType` would be a *trivial bound* that rustc discharges at the
    /// definition — `E0277` at the impl, not an unusable impl. sqlx implements
    /// `PgHasArrayType` for `i32`, `i64` and `String` only, so `NumNewtype` (whose
    /// inners include `u32`/`usize`) must stay off or `common` stops compiling (#891).
    pub(crate) pg_array: bool,
}

/// Merge the user's generics with the parameters and predicate required by one `SQLx` impl.
///
/// A lifetime is inserted before the user's parameters because Rust requires lifetimes
/// to precede type parameters; `DB` is always appended after them.
fn sqlx_impl_generics(
    user: &syn::Generics,
    leading_lifetime: Option<syn::LifetimeParam>,
    predicate: syn::WherePredicate,
) -> syn::Generics {
    let mut merged = user.clone();
    if let Some(lifetime) = leading_lifetime {
        merged
            .params
            .insert(0, syn::GenericParam::Lifetime(lifetime));
    }
    merged.params.push(syn::parse_quote!(DB: ::sqlx::Database));
    merged.make_where_clause().predicates.push(predicate);
    merged
}

/// The sqlx bridge impls for `spec.name`: `Type`, `Encode`, `Decode`, and — when
/// `spec.pg_array` is set — `PgHasArrayType`.
pub(crate) fn bridge(spec: &BridgeSpec<'_>) -> TokenStream {
    let BridgeSpec {
        name,
        generics,
        type_inner,
        encode_inner,
        to_inner,
        decode_inner,
        convert,
        pg_array,
    } = spec;
    // The bound each impl needs on its inner is a `where` predicate rather than a bound on
    // the parameter, because the inner is a *type*, not a parameter — and folding it into
    // the merged copy is what lets the user's own `where` clause survive alongside it.
    // `ty_generics` comes from the user's ORIGINAL generics: the merged copies carry each
    // impl's own parameters, and splitting one of those would spell the self type
    // `X<T, DB>` — a bug the `sqlx` feature being off in this crate would otherwise hide.
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Postgres-only, and concrete rather than generic over `DB` — so it is emitted
    // only for callers whose `type_inner` is known to implement `PgHasArrayType`
    // (#891). `::sqlx::postgres` is always in scope here: the workspace pins sqlx with
    // both the `sqlite` and `postgres` features, so the enclosing `feature = "sqlx"`
    // gate is the only one needed.
    //
    // Unlike the three below it introduces no parameter of its own, so it takes the
    // user's generics unmerged (#875) — empty for every current `pg_array` caller, and
    // correct rather than accidentally so if a generic newtype ever opts in.
    let pg_array_impl = if *pg_array {
        quote! {
            #[automatically_derived]
            impl #impl_generics ::sqlx::postgres::PgHasArrayType for #name #ty_generics #where_clause {
                fn array_type_info() -> ::sqlx::postgres::PgTypeInfo {
                    <#type_inner as ::sqlx::postgres::PgHasArrayType>::array_type_info()
                }
                fn array_compatible(ty: &::sqlx::postgres::PgTypeInfo) -> bool {
                    <#type_inner as ::sqlx::postgres::PgHasArrayType>::array_compatible(ty)
                }
            }
        }
    } else {
        TokenStream::new()
    };

    let type_g = sqlx_impl_generics(
        generics,
        None,
        syn::parse_quote!(#type_inner: ::sqlx::Type<DB>),
    );
    let (type_impl_generics, _, type_where) = type_g.split_for_impl();

    let encode_g = sqlx_impl_generics(
        generics,
        Some(syn::parse_quote!('q)),
        syn::parse_quote!(#encode_inner: ::sqlx::Encode<'q, DB>),
    );
    let (encode_impl_generics, _, encode_where) = encode_g.split_for_impl();

    let decode_g = sqlx_impl_generics(
        generics,
        Some(syn::parse_quote!('r)),
        syn::parse_quote!(#decode_inner: ::sqlx::Decode<'r, DB>),
    );
    let (decode_impl_generics, _, decode_where) = decode_g.split_for_impl();

    quote! {
        #[cfg(feature = "sqlx")]
        const _: () = {
            #[automatically_derived]
            impl #type_impl_generics ::sqlx::Type<DB> for #name #ty_generics
            #type_where
            {
                fn type_info() -> <DB as ::sqlx::Database>::TypeInfo {
                    <#type_inner as ::sqlx::Type<DB>>::type_info()
                }
                fn compatible(ty: &<DB as ::sqlx::Database>::TypeInfo) -> bool {
                    <#type_inner as ::sqlx::Type<DB>>::compatible(ty)
                }
            }

            #[automatically_derived]
            impl #encode_impl_generics ::sqlx::Encode<'q, DB> for #name #ty_generics
            #encode_where
            {
                fn encode_by_ref(
                    &self,
                    buf: &mut <DB as ::sqlx::Database>::ArgumentBuffer,
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
            impl #decode_impl_generics ::sqlx::Decode<'r, DB> for #name #ty_generics
            #decode_where
            {
                fn decode(
                    value: <DB as ::sqlx::Database>::ValueRef<'r>,
                ) -> ::core::result::Result<Self, ::sqlx::error::BoxDynError> {
                    let v = <#decode_inner as ::sqlx::Decode<'r, DB>>::decode(value)?;
                    #convert
                }
            }

            #pg_array_impl
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
        norm_s(&t.to_string())
    }

    /// [`norm`] for a needle written in source form.
    pub(crate) fn norm_s(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn spec_for<'a>(name: &'a syn::Ident, generics: &'a syn::Generics) -> BridgeSpec<'a> {
        BridgeSpec {
            name,
            generics,
            type_inner: quote! { ::std::string::String },
            encode_inner: quote! { ::std::string::String },
            to_inner: quote! { &self.0 },
            decode_inner: quote! { ::std::string::String },
            convert: quote! { ::core::result::Result::Ok(#name(v)) },
            pg_array: false,
        }
    }

    #[test]
    fn type_impl_delegates_to_type_inner() {
        let n = format_ident!("X");
        let g = syn::Generics::default();
        let out = norm(&bridge(&spec_for(&n, &g)));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::compatible(ty)"));
    }

    #[test]
    fn encode_binds_an_annotated_local_and_keeps_size_hint() {
        let n = format_ident!("X");
        let g = syn::Generics::default();
        let out = norm(&bridge(&spec_for(&n, &g)));
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
        let g = syn::Generics::default();
        let out = norm(&bridge(&spec_for(&n, &g)));
        assert!(
            out.contains("letv=<::std::string::Stringas::sqlx::Decode<'r,DB>>::decode(value)?;")
        );
        assert!(out.contains("::core::result::Result::Ok(X(v))"));
    }

    #[test]
    fn the_three_inners_are_independent() {
        let n = format_ident!("X");
        let g = syn::Generics::default();
        let out = norm(&bridge(&BridgeSpec {
            name: &n,
            generics: &g,
            type_inner: quote! { ::std::string::String },
            encode_inner: quote! { &'q str },
            to_inner: quote! { &"tok" },
            decode_inner: quote! { &'r str },
            convert: quote! { ::core::result::Result::Ok(X) },
            pg_array: false,
        }));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("<&'qstras::sqlx::Encode<'q,DB>>::encode_by_ref(inner,buf)"));
        assert!(out.contains("<&'rstras::sqlx::Decode<'r,DB>>::decode(value)?"));
    }

    /// The user's generics reach all three impl headers, and each impl's own parameter is
    /// merged in at a position Rust accepts: `DB` pushed on the end, the lifetimes
    /// prepended (a lifetime after a type parameter is a syntax error, #875).
    #[test]
    fn the_users_generics_thread_through_all_three_impls() {
        let n = format_ident!("X");
        let g: syn::Generics = syn::parse_quote!(<T: Role>);
        let out = norm(&bridge(&spec_for(&n, &g)));
        assert!(
            out.contains("impl<T:Role,DB:::sqlx::Database>::sqlx::Type<DB>forX<T>"),
            "{out}"
        );
        assert!(out.contains("impl<'q,T:Role,DB:::sqlx::Database>::sqlx::Encode<'q,DB>forX<T>"));
        assert!(out.contains("impl<'r,T:Role,DB:::sqlx::Database>::sqlx::Decode<'r,DB>forX<T>"));
    }

    /// The user's own `where` clause survives the merge — the bridge's inner predicate is
    /// added to it rather than replacing it.
    #[test]
    fn a_users_where_clause_survives_the_merge() {
        let n = format_ident!("X");
        // `syn::Generics`'s own parser reads only the `<…>` list, so the `where` clause has
        // to come off a whole item.
        let item: syn::ItemStruct = syn::parse_quote!(
            struct X<T>(String)
            where
                T: Role;
        );
        let g = item.generics;
        let out = norm(&bridge(&spec_for(&n, &g)));
        assert_eq!(out.matches("T:Role,").count(), 3, "one per impl: {out}");
        assert_eq!(
            out.matches("::std::string::String:::sqlx::Type<DB>")
                .count(),
            1,
            "the bridge's own predicate is still there: {out}"
        );
    }

    #[test]
    fn output_is_feature_gated_and_marked_derived() {
        let n = format_ident!("X");
        let g = syn::Generics::default();
        let out = norm(&bridge(&spec_for(&n, &g)));
        assert!(out.contains("#[cfg(feature=\"sqlx\")]"));
        assert_eq!(out.matches("#[automatically_derived]").count(), 3);

        // The array impl is the fourth, and only when opted in — asserted both ways
        // rather than bumping the count, since half the callers never emit it (#891).
        let with_array = norm(&bridge(&BridgeSpec {
            pg_array: true,
            ..spec_for(&n, &g)
        }));
        assert_eq!(with_array.matches("#[automatically_derived]").count(), 4);
    }

    #[test]
    fn pg_array_impl_delegates_to_type_inner_when_enabled() {
        let n = format_ident!("X");
        let g = syn::Generics::default();
        let out = norm(&bridge(&BridgeSpec {
            pg_array: true,
            ..spec_for(&n, &g)
        }));
        assert!(out.contains("impl::sqlx::postgres::PgHasArrayTypeforX"));
        assert!(out.contains(
            "<::std::string::Stringas::sqlx::postgres::PgHasArrayType>::array_type_info()"
        ));
        assert!(out.contains(
            "<::std::string::Stringas::sqlx::postgres::PgHasArrayType>::array_compatible(ty)"
        ));
    }

    #[test]
    fn pg_array_impl_is_absent_when_disabled() {
        let n = format_ident!("X");
        let g = syn::Generics::default();
        let out = norm(&bridge(&spec_for(&n, &g)));
        assert!(
            !out.contains("PgHasArrayType"),
            "NumNewtype-style callers must not get the impl: their inner may be u32/usize, \
             and the concrete impl would be E0277 at the definition"
        );
    }
}
