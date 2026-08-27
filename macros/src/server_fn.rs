//! The derivation behind `#[macros::server]` (#714).
//!
//! A bare `#[server]` fn would carry three hand-maintained literals that all
//! restate what the source already says: the wire `endpoint`, the ADR-0011 span
//! `name`, and the `boundary!` label. This module computes all three from the one
//! thing that cannot drift — the fn's location and identifier.
//!
//! **Everything here takes the file path as a parameter.** `proc_macro`'s
//! `Span::call_site().file()` panics outside a live macro expansion, so any logic
//! that called it would be unreachable from `cargo test`. Keeping the derivation
//! pure and letting the thin shell in [`crate`] supply the path is what makes the
//! error paths testable at all — and this crate is coverage-measured, so
//! "untestable" would mean "unshippable".

use proc_macro2::TokenTree;
use quote::quote;
use syn::parse::{Parse, ParseStream, Parser};
use syn::{Expr, Token};

/// Where a `#[server]` fn must live, relative to the repository root. The vertical
/// is the single path segment between this marker and `api.rs`.
const WEB_SRC: &str = "web/src/";

/// The file a vertical's `#[server]` fns must live in.
const API_FILE: &str = "api.rs";

/// What `#[macros::server]` derives for one fn.
pub(crate) struct Derived {
    /// The wire endpoint, e.g. `/audiences/rename`.
    pub endpoint: String,
    /// The ADR-0011 span name, e.g. `web.audiences.rename`.
    pub span_name: String,
    /// Arguments forwarded verbatim to `#[server]` (only `input = …`).
    pub server_args: Vec<syn::Meta>,
    /// Declaration-only `fields(...)` arguments forwarded to the generated span.
    pub instrument_args: Vec<syn::Meta>,
    /// Parameters explicitly omitted by source `skip(...)`.
    pub skipped: Vec<syn::Ident>,
    /// Whether source `skip_all` suppresses every generated parameter field.
    pub skip_all: bool,
}

/// The vertical owning `file`, or an error naming the placement rule.
///
/// Matched **relative to the [`WEB_SRC`] marker** rather than from the start of the
/// path, so a `--remap-path-prefix` build (Nix) derives the same vertical as a host
/// build. Mirrors `xtask/src/web_server_fns.rs`'s `vertical_of`, which keys the
/// gates on the same rule.
fn vertical_of(file: &str, ident: &syn::Ident) -> Result<String, syn::Error> {
    let Some((_, relative)) = file.split_once(WEB_SRC) else {
        return Err(syn::Error::new(
            ident.span(),
            format!(
                "#[macros::server] is only for `{WEB_SRC}<vertical>/{API_FILE}`; this file ({file}) is not under `{WEB_SRC}`"
            ),
        ));
    };
    let segments: Vec<&str> = relative.split('/').collect();
    match segments.as_slice() {
        [vertical, file_name] if *file_name == API_FILE => Ok((*vertical).to_string()),
        // A fn directly under `web/src` has no vertical, so there is no honest name
        // to derive — the same hard error ADR-0082 already requires of the gates.
        [_] => Err(syn::Error::new(
            ident.span(),
            format!(
                "#[macros::server] fn directly under `{WEB_SRC}` has no vertical directory; move it to `{WEB_SRC}<vertical>/{API_FILE}`"
            ),
        )),
        // The case that would make `(vertical, ident)` lossy: two same-named fns in
        // one vertical's submodules derive one endpoint and one span name, and the
        // compiler cannot catch it (a glob re-export lets one shadow the other, so
        // the pair compiles and the loser 404s — #358).
        _ => Err(syn::Error::new(
            ident.span(),
            format!(
                "#[macros::server] fns live in `{WEB_SRC}<vertical>/{API_FILE}`, never a submodule; `{file}` would make the derived endpoint ambiguous with another vertical member"
            ),
        )),
    }
}

/// Route one attribute argument to `#[server]` or `#[tracing::instrument]`.
///
/// Default-deny: only the keys actually used across the tree are accepted, so an
/// argument this macro does not model cannot silently reach either attribute.
/// `endpoint` and `name` are refused because they are the macro's to derive —
/// accepting them would reintroduce exactly the drift #714 removes. `fields(…)`
/// is accepted only for empty declarations (`field = tracing::field::Empty`):
/// values recorded later still go through the span owner's code, and the macro
/// never accepts expressions that bypass the type-owned `TraceField` projection.
struct EmptyFieldDeclaration;

impl Parse for EmptyFieldDeclaration {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut saw_ident = false;
        while !input.peek(Token![=]) {
            match input.parse::<TokenTree>()? {
                TokenTree::Ident(_) => saw_ident = true,
                TokenTree::Punct(punct) if punct.as_char() == '.' => {}
                token => {
                    return Err(syn::Error::new_spanned(
                        token,
                        "`fields` names may contain only identifiers and dots",
                    ));
                }
            }
        }
        if !saw_ident {
            return Err(input.error("`fields` declarations need a field name"));
        }
        input.parse::<Token![=]>()?;
        let value: Expr = input.parse()?;
        let Expr::Path(value) = value else {
            return Err(syn::Error::new_spanned(
                value,
                "`fields` accepts only `field = tracing::field::Empty` declarations",
            ));
        };
        if value
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            != ["tracing", "field", "Empty"]
        {
            return Err(syn::Error::new_spanned(
                value,
                "`fields` accepts only `field = tracing::field::Empty` declarations",
            ));
        }
        Ok(Self)
    }
}

fn validate_empty_fields(arg: &syn::Meta) -> Result<(), syn::Error> {
    let syn::Meta::List(list) = arg else {
        return Err(syn::Error::new_spanned(
            arg,
            "`fields` must be a list of empty declarations",
        ));
    };

    let fields = syn::punctuated::Punctuated::<EmptyFieldDeclaration, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .map_err(|error| syn::Error::new_spanned(list, error))?;
    if fields.is_empty() {
        return Err(syn::Error::new_spanned(
            list,
            "#[macros::server] accepts only empty tracing fields; declare with \
             `field = tracing::field::Empty` and record bounded values in the body",
        ));
    }
    Ok(())
}

fn route(arg: &syn::Meta, derived: &mut Derived) -> Result<(), syn::Error> {
    let named = |key: &str| arg.path().is_ident(key);

    if named("endpoint") || named("name") {
        return Err(syn::Error::new_spanned(
            arg,
            format!(
                "#[macros::server] derives `{}` from the file path and fn ident; remove it",
                if named("endpoint") {
                    "endpoint"
                } else {
                    "name"
                }
            ),
        ));
    }
    if named("fields") {
        validate_empty_fields(arg)?;
        derived.instrument_args.push(arg.clone());
        return Ok(());
    }
    if named("input") {
        derived.server_args.push(arg.clone());
        return Ok(());
    }
    if named("skip") {
        let syn::Meta::List(list) = arg else {
            return Err(syn::Error::new_spanned(
                arg,
                "`skip` requires `skip(name, …)`",
            ));
        };
        let skip_idents = syn::punctuated::Punctuated::<syn::Ident, Token![,]>::parse_terminated
            .parse2(list.tokens.clone())?;
        if skip_idents.is_empty() {
            return Err(syn::Error::new_spanned(
                arg,
                "`skip` requires at least one name",
            ));
        }
        derived.skipped.extend(skip_idents);
        return Ok(());
    }
    if named("skip_all") {
        if !matches!(arg, syn::Meta::Path(_)) {
            return Err(syn::Error::new_spanned(
                arg,
                "`skip_all` takes no arguments",
            ));
        }
        derived.skip_all = true;
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        arg,
        "#[macros::server] accepts only `input = …` (forwarded to #[server]), \
         `skip(...)` / `skip_all`, and empty `fields(...)` declarations \
         (forwarded to #[tracing::instrument])",
    ))
}

/// Everything `#[macros::server]` derives for one fn, or the first error found.
pub(crate) fn derive(
    file: &str,
    ident: &syn::Ident,
    args: &[syn::Meta],
) -> Result<Derived, syn::Error> {
    let vertical = vertical_of(file, ident)?;
    let mut derived = Derived {
        endpoint: format!("/{vertical}/{ident}"),
        span_name: format!("web.{vertical}.{ident}"),
        server_args: Vec::new(),
        instrument_args: Vec::new(),
        skipped: Vec::new(),
        skip_all: false,
    };
    for arg in args {
        route(arg, &mut derived)?;
    }
    Ok(derived)
}

/// The full expansion: both attributes, and the body wrapped in the error boundary.
///
/// `#[::leptos::server]` is emitted **first** so it stays the outer attribute. That
/// ordering is load-bearing twice over: it relocates the instrumented body into the
/// generated `__server_<ident>` fn (so the span wraps the boundary call), and it
/// discards that body on the wasm client (so the `#[cfg(feature = "server")]`
/// `server_boundary` never has to exist there).
///
/// Paths are absolute because attribute macros are **not** path-hygienic — a bare
/// `server` would resolve against whatever the call site happened to import.
pub(crate) fn expand(
    file: &str,
    args: &[syn::Meta],
    mut f: syn::ItemFn,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let Derived {
        endpoint,
        span_name,
        server_args,
        instrument_args,
        skipped,
        skip_all,
    } = derive(file, &f.sig.ident, args)?;

    let projected_fields: Vec<_> = if skip_all {
        Vec::new()
    } else {
        f.sig
            .inputs
            .iter()
            .filter_map(|arg| {
                let syn::FnArg::Typed(arg) = arg else {
                    return None;
                };
                let syn::Pat::Ident(pattern) = arg.pat.as_ref() else {
                    return None;
                };
                let ident = &pattern.ident;
                if skipped.iter().any(|skipped| skipped == ident) {
                    return None;
                }
                Some(quote! {
                    #ident = ?::common::trace_field::TraceField::trace_value(&#ident)
                })
            })
            .collect()
    };
    let declared_fields: Vec<_> = instrument_args
        .iter()
        .map(|arg| {
            let syn::Meta::List(list) = arg else {
                unreachable!("route stores only declaration-only fields lists")
            };
            &list.tokens
        })
        .collect();
    let fields_arg = if projected_fields.is_empty() && declared_fields.is_empty() {
        quote! {}
    } else {
        quote! {
            , fields(#(#projected_fields,)* #(#declared_fields),*)
        }
    };

    let body = &f.block;
    *f.block = syn::parse_quote!({
        crate::error::server_boundary(async move #body).await
    });

    let attrs = &f.attrs;
    let vis = &f.vis;
    let sig = &f.sig;
    let block = &f.block;

    Ok(quote! {
        #(#attrs)*
        #[::leptos::server(endpoint = #endpoint #(, #server_args)*)]
        #[::tracing::instrument(name = #span_name, skip_all #fields_arg)]
        #vis #sig #block

    })
}

#[cfg(test)]
mod tests {
    use super::{Derived, derive, expand};
    use syn::parse_quote;

    fn ident(s: &str) -> syn::Ident {
        syn::Ident::new(s, proc_macro2::Span::call_site())
    }

    /// `Result::unwrap_err` needs `Debug` on the `Ok` type, and `syn::Meta` only
    /// implements it under syn's `extra-traits` feature — not worth enabling
    /// workspace-wide for test ergonomics.
    fn expect_err(result: Result<Derived, syn::Error>) -> syn::Error {
        result
            .err()
            .expect("expected a rejection, got a successful derivation")
    }

    #[test]
    fn derives_endpoint_and_span_name_from_path_and_ident() {
        let d = derive("web/src/audiences/api.rs", &ident("rename"), &[]).expect("derives");
        assert_eq!(d.endpoint, "/audiences/rename");
        assert_eq!(d.span_name, "web.audiences.rename");
    }

    #[test]
    fn a_remapped_path_prefix_does_not_change_the_vertical() {
        // Nix builds may remap the path prefix; matching relative to `web/src/`
        // keeps the derived values identical to a host build.
        let d = derive(
            "/build/src-abc123/web/src/audiences/api.rs",
            &ident("rename"),
            &[],
        )
        .expect("derives");
        assert_eq!(d.endpoint, "/audiences/rename");
        assert_eq!(d.span_name, "web.audiences.rename");
    }

    #[test]
    fn routes_input_and_remembers_source_skip_all() {
        let args: Vec<syn::Meta> = vec![
            parse_quote!(input = MultipartFormData),
            parse_quote!(skip_all),
        ];
        let d = derive("web/src/media/api.rs", &ident("upload"), &args).expect("derives");
        assert_eq!(d.server_args.len(), 1);
        assert!(d.instrument_args.is_empty());
        assert!(d.skip_all);
    }

    #[test]
    fn forwards_empty_fields_to_instrument() {
        let args: Vec<syn::Meta> = vec![parse_quote!(fields(
            registration.policy = tracing::field::Empty
        ))];
        let d = derive("web/src/registration/api.rs", &ident("register"), &args).expect("derives");
        assert_eq!(d.instrument_args.len(), 1);
        assert!(d.server_args.is_empty());
    }

    #[test]
    fn rejects_fields_with_values_because_only_declarations_are_supported() {
        let args: Vec<syn::Meta> = vec![parse_quote!(fields(who = "x"))];
        let e = expect_err(derive("web/src/audiences/api.rs", &ident("rename"), &args));
        assert!(e.to_string().contains("tracing::field::Empty"), "{e}");
        let args: Vec<syn::Meta> = vec![parse_quote!(fields(
            who = {
                tracing::field::Empty;
                "x"
            }
        ))];
        let e = expect_err(derive("web/src/audiences/api.rs", &ident("rename"), &args));
        assert!(e.to_string().contains("tracing::field::Empty"), "{e}");
    }

    #[test]
    fn rejects_malformed_empty_field_declarations() {
        for args in [
            vec![parse_quote!(fields("who" = tracing::field::Empty))],
            vec![parse_quote!(fields(= tracing::field::Empty))],
            vec![parse_quote!(fields(who = Empty))],
            vec![parse_quote!(fields = "who")],
            vec![parse_quote!(fields())],
        ] {
            let e = expect_err(derive("web/src/audiences/api.rs", &ident("rename"), &args));
            assert!(
                e.to_string().contains("fields") || e.to_string().contains("Empty"),
                "{e}"
            );
        }
    }

    #[test]
    fn remembers_a_source_skip_list() {
        let args: Vec<syn::Meta> = vec![parse_quote!(skip(name))];
        let d = derive("web/src/audiences/api.rs", &ident("rename"), &args).expect("derives");
        assert_eq!(
            d.skipped
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["name"]
        );
        assert!(d.instrument_args.is_empty());
        assert!(d.server_args.is_empty());
    }

    #[test]
    fn rejects_malformed_skip_arguments() {
        for arg in [
            parse_quote!(skip),
            parse_quote!(skip()),
            parse_quote!(skip_all(name)),
        ] {
            let error = expect_err(derive("web/src/audiences/api.rs", &ident("rename"), &[arg]));
            assert!(error.to_string().contains("skip"), "{error}");
        }
    }

    #[test]
    fn rejects_a_passed_endpoint() {
        let args: Vec<syn::Meta> = vec![parse_quote!(endpoint = "/x/y")];
        let e = expect_err(derive("web/src/audiences/api.rs", &ident("rename"), &args));
        assert!(e.to_string().contains("endpoint"), "{e}");
    }

    #[test]
    fn rejects_a_passed_name() {
        let args: Vec<syn::Meta> = vec![parse_quote!(name = "web.x.y")];
        let e = expect_err(derive("web/src/audiences/api.rs", &ident("rename"), &args));
        assert!(e.to_string().contains("name"), "{e}");
    }

    #[test]
    fn rejects_an_unrecognized_key() {
        let args: Vec<syn::Meta> = vec![parse_quote!(ret)];
        let e = expect_err(derive("web/src/audiences/api.rs", &ident("rename"), &args));
        assert!(e.to_string().contains("skip"), "{e}");
    }

    #[test]
    fn rejects_a_path_with_no_web_src_marker() {
        let e = expect_err(derive("server/src/lib.rs", &ident("rename"), &[]));
        assert!(e.to_string().contains("web/src"), "{e}");
    }

    #[test]
    fn rejects_a_fn_directly_under_web_src() {
        let e = expect_err(derive("web/src/mail.rs", &ident("send"), &[]));
        assert!(e.to_string().contains("vertical"), "{e}");
    }

    #[test]
    fn rejects_a_nested_submodule() {
        // The case that makes `(vertical, ident)` lossy — the placement rule exists
        // for exactly this.
        let e = expect_err(derive(
            "web/src/posts/api/listing.rs",
            &ident("list_by_tag"),
            &[],
        ));
        assert!(e.to_string().contains("submodule"), "{e}");
    }

    #[test]
    fn expansion_skips_original_arguments_and_records_only_trace_projections() {
        let f: syn::ItemFn = parse_quote! {
            pub async fn list(enabled: bool, offset: Option<PageOffset>) -> WebResult<()> {
                do_it().await
            }
        };
        let out = expand("web/src/posts/api.rs", &[], f)
            .expect("expands")
            .to_string();

        assert!(out.contains("skip_all"), "{out}");
        for field in ["enabled", "offset"] {
            assert!(
                out.contains(&format!(
                    "{field} = ? :: common :: trace_field :: TraceField :: trace_value (& {field})"
                )),
                "missing projected field {field}: {out}"
            );
        }
    }

    #[test]
    fn expansion_omits_a_source_skipped_nonimplementer() {
        let f: syn::ItemFn = parse_quote! {
            pub async fn rename(name: AudienceName) -> WebResult<()> { do_it().await }
        };
        let out = expand("web/src/audiences/api.rs", &[parse_quote!(skip(name))], f)
            .expect("expands")
            .to_string();

        assert!(out.contains("skip_all"), "{out}");
        assert!(!out.contains("name = ?"), "{out}");
    }

    #[test]
    fn source_skip_all_preserves_manual_empty_fields_without_parameter_fields() {
        let f: syn::ItemFn = parse_quote! {
            pub async fn register(request: RegistrationRequest) -> WebResult<()> {
                do_it().await
            }
        };
        let args = [
            parse_quote!(skip_all),
            parse_quote!(fields(registration.outcome = tracing::field::Empty)),
        ];
        let out = expand("web/src/registration/api.rs", &args, f)
            .expect("expands")
            .to_string();

        assert!(out.contains("skip_all"), "{out}");
        assert!(
            out.contains("registration . outcome = tracing :: field :: Empty"),
            "{out}"
        );
        assert!(!out.contains("request = ?"), "{out}");
    }

    #[test]
    fn source_skip_all_safely_omits_pattern_bound_parameters() {
        let f: syn::ItemFn = parse_quote! {
            pub async fn pair((left, right): (u32, u32)) -> WebResult<()> {
                do_it().await
            }
        };
        let out = expand("web/src/posts/api.rs", &[parse_quote!(skip_all)], f)
            .expect("expands")
            .to_string();

        assert!(out.contains("skip_all"), "{out}");
        assert!(
            !out.contains("left = ?") && !out.contains("right = ?"),
            "{out}"
        );
    }

    #[test]
    fn expansion_ignores_receiver_and_pattern_inputs_without_projectable_names() {
        for f in [
            parse_quote! {
                pub async fn method(&self) -> WebResult<()> { do_it().await }
            },
            parse_quote! {
                pub async fn pair((left, right): (u32, u32)) -> WebResult<()> {
                    do_it().await
                }
            },
        ] {
            let out = expand("web/src/posts/api.rs", &[], f)
                .expect("expands")
                .to_string();
            assert!(out.contains("skip_all"), "{out}");
            assert!(
                !out.contains("left = ?") && !out.contains("right = ?"),
                "{out}"
            );
        }
    }

    #[test]
    fn expands_to_absolute_attribute_paths_in_order_with_a_wrapped_body() {
        let f: syn::ItemFn = parse_quote! {
            pub async fn rename(name: AudienceName) -> WebResult<()> { do_it().await }
        };
        let out = expand("web/src/audiences/api.rs", &[parse_quote!(skip(name))], f)
            .expect("expands")
            .to_string();

        // Attribute macros are not path-hygienic, so both paths must be absolute.
        let server_at = out
            .find(":: leptos :: server")
            .expect("emits ::leptos::server");
        let instrument_at = out
            .find(":: tracing :: instrument")
            .expect("emits ::tracing::instrument");
        // `#[server]` must stay OUTERMOST: it relocates the instrumented body and
        // discards it on the client.
        assert!(
            server_at < instrument_at,
            "::leptos::server must precede ::tracing::instrument: {out}"
        );

        assert!(out.contains(r#"endpoint = "/audiences/rename""#), "{out}");
        assert!(out.contains(r#"name = "web.audiences.rename""#), "{out}");
        assert!(out.contains("skip_all"), "{out}");
        assert!(out.contains("crate :: error :: server_boundary"), "{out}");
        assert!(out.contains("async move"), "{out}");
        assert!(
            !out.contains("register_explicit"),
            "registration belongs to the integration-test registrar: {out}"
        );
    }

    #[test]
    fn expand_propagates_a_derive_error() {
        let f: syn::ItemFn = parse_quote! {
            pub async fn x() -> WebResult<()> { y().await }
        };
        assert!(expand("web/src/posts/api/listing.rs", &[], f).is_err());
    }
}
