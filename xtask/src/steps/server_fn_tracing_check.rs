//! The `server-fn-tracing` static check (#511): every `#[macros::server]` fn in the
//! `web` crate must declare a PII-safe span.
//!
//! ADR-0011 wants end-to-end tracing, but 44 of the 55 `web` server fns shipped
//! with no span at all — a request into `create_post` produced nothing to correlate.
//! An unenforced convention is what allowed that gap, so the convention is a gate.
//!
//! **Enumeration** is shared through [`crate::web_server_fns`]; this module
//! supplies only the tracing rules.
//!
//! The span itself — its presence, its placement, and its `web.<vertical>.<ident>`
//! name — is no longer anyone's to get wrong: `#[macros::server]` emits the
//! `#[tracing::instrument]` and derives the name (#714), so the rules that policed a
//! hand-written attribute went with the hand-written attribute. What is left is the
//! source-shape judgments that remain outside type resolution:
//!
//! 1. **Trace admission is compiler-owned** — `#[macros::server]` hides every
//!    original parameter with generated `skip_all` and records named,
//!    non-skipped parameters only through `common::trace_field::TraceField`.
//!    Missing implementations fail compilation; this source gate never
//!    classifies a type name.
//! 2. **A parameter bound by a pattern cannot be projected by name** — a
//!    destructured argument has no single field identifier, so it is refused
//!    unless source `skip_all` explicitly covers it.
//! 3. **Source skip names must name real plain parameters** — stale or misspelled
//!    opt-outs fail rather than silently changing author intent.
//! 4. **Declared span fields stay declaration-only** — `fields(...)` is allowed
//!    only as `field = tracing::field::Empty`. Values are recorded later in the
//!    function body where the author has context.
//! 5. **An unmodelled attribute argument is refused** — the macro accepts only
//!    `skip(...)`/`skip_all`, empty `fields(...)`, and `input = …`. Anything else
//!    fails until both macro and gate model it deliberately.
//!
//! Like the registrar guard this is **mandatory with no per-fn opt-out**, and
//! **fail-loud**: an unparseable or unreadable file is reported, never skipped,
//! because a file we cannot enumerate could hide a server fn.

use std::collections::BTreeSet;
use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use syn::parse::{Parse, ParseStream, Parser};
use syn::{Expr, Meta, Token};

use crate::result::{CommandResult, StepResult};
use crate::web_server_fns::{self, WEB_SRC, WebServerFn, vertical_of};

/// What a `#[macros::server]` attribute declares about its span.
#[derive(Default)]
struct SpanArgs {
    /// Parameter names named in `skip(...)`.
    skipped: BTreeSet<String>,
    /// Whether `skip_all` was given.
    skip_all: bool,
}

/// Every identifier in a token stream, at any nesting depth.
fn idents_in(tokens: &TokenStream) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_idents(tokens, &mut out);
    out
}

fn collect_idents(tokens: &TokenStream, out: &mut BTreeSet<String>) {
    for tt in tokens.clone() {
        match tt {
            TokenTree::Ident(i) => {
                out.insert(i.to_string());
            }
            TokenTree::Group(g) => collect_idents(&g.stream(), out),
            _ => {}
        }
    }
}

/// A `path::to::thing` rendered for an error message.
fn path_text(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}
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

fn validate_empty_fields(list: &syn::MetaList) -> Result<(), String> {
    let fields = syn::punctuated::Punctuated::<EmptyFieldDeclaration, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .map_err(|error| error.to_string())?;
    if fields.is_empty() {
        return Err("`fields` accepts only `field = tracing::field::Empty` declarations".into());
    }
    Ok(())
}

/// What a `#[macros::server]` attribute declares about source skip intent.
///
/// The macro consumes `skip(...)` / `skip_all`, combines generated TraceField
/// projections with declaration-only `fields(...)`, and forwards `input = …` to
/// `#[server]`. `endpoint` and `name` remain derived. Default-deny grammar keeps
/// an argument neither macro nor gate models from silently changing the span.
fn span_args(attr: &syn::Attribute) -> Result<SpanArgs, String> {
    let mut out = SpanArgs::default();
    let Some(args) = web_server_fns::server_attr_args(attr)? else {
        // A bare attribute carries no explicit source skip intent.
        return Ok(out);
    };
    for arg in args {
        let Some(ident) = arg.path().get_ident().map(ToString::to_string) else {
            return Err(format!(
                "unrecognized #[macros::server] argument `{}`",
                path_text(arg.path())
            ));
        };
        match ident.as_str() {
            "skip" => {
                let Meta::List(list) = &arg else {
                    return Err("`skip` must be `skip(a, b)`".into());
                };
                out.skipped.extend(idents_in(&list.tokens));
            }
            "skip_all" => out.skip_all = true,
            "fields" => {
                let Meta::List(list) = &arg else {
                    return Err(
                        "`fields` must be `fields(name = tracing::field::Empty, ...)`".into(),
                    );
                };
                validate_empty_fields(list)?;
            }
            // Routed to `#[server]`, not to the span — it records nothing.
            "input" => {}
            other => {
                return Err(format!(
                    "unrecognized #[macros::server] argument `{other}` — accepted arguments are \
                     `skip(...)`/`skip_all`, empty `fields(...)` declarations, and `input = …`"
                ));
            }
        }
    }
    Ok(out)
}

/// Every problem with one `#[macros::server]` fn, as human-readable lines (without
/// the file prefix, which the caller adds).
fn problems_with(f: &WebServerFn) -> Vec<String> {
    let at = format!("line {}: `{}`", f.line, f.ident);
    let parsed = match span_args(&f.attrs[f.server_attr_index]) {
        Ok(parsed) => parsed,
        Err(e) => return vec![format!("{at}: {e}")],
    };
    let mut lines = Vec::new();
    let plain_params: BTreeSet<&str> = f
        .params
        .iter()
        .filter_map(|param| param.name.as_deref())
        .collect();
    for skipped in &parsed.skipped {
        if !plain_params.contains(skipped.as_str()) {
            lines.push(format!(
                "{at}: skip name `{skipped}` does not name a plain parameter"
            ));
        }
    }

    for param in &f.params {
        if param.name.is_none() && !parsed.skip_all {
            lines.push(format!(
                "{at}: a parameter is bound by a pattern rather than a plain identifier, so it \
                 cannot become a projected field or be named in skip(...) — bind it to an \
                 identifier, or use skip_all"
            ));
        }
    }

    lines
}

/// The failure detail for every non-conforming `#[macros::server]` fn, or `None`
/// when every one conforms. Pure given its inputs, so it is unit-tested directly.
fn problems(web_sources: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, src) in web_sources {
        let fns = match web_server_fns::server_fns_in(src) {
            Ok(fns) => fns,
            Err(msg) => {
                lines.push(format!("{path}: {msg}"));
                continue;
            }
        };
        if fns.is_empty() {
            continue;
        }
        // The vertical is no longer an input to any rule here — the macro derives
        // the span name itself — but a server fn with no vertical directory is still
        // reported, because the derivation, the registrar's `(vertical, leaf)` key
        // and the coverage inventory all break on it.
        if let Err(msg) = vertical_of(path) {
            lines.push(msg);
            continue;
        }
        for f in &fns {
            lines.extend(problems_with(f).into_iter().map(|l| format!("{path}: {l}")));
        }
    }

    if lines.is_empty() {
        return None;
    }
    lines.sort();
    lines.push(
        "  recovery: every skip name must name a plain parameter; every pattern-bound parameter \
         requires skip_all; fields(...) values must remain tracing::field::Empty declarations"
            .to_string(),
    );
    Some(lines.join("\n"))
}

/// Scan every `web/src` Rust file for `#[macros::server]` fns and check each declares
/// a PII-safe span. A missing `web/src` tree or an unreadable file is a hard
/// failure (not a silent pass), so a moved path can never quietly disable the guard.
pub fn run(result: &mut CommandResult) {
    let web = match web_server_fns::read_web_sources(Path::new(WEB_SRC)) {
        Ok(v) => v,
        Err(e) => {
            result.push(StepResult::fail("server-fn-tracing").detail(e));
            return;
        }
    };
    let mut read_errors = web.read_errors;
    let sources = web.sources;

    let step = match (read_errors.is_empty(), problems(&sources)) {
        (true, None) => StepResult::ok("server-fn-tracing"),
        (_, prob) => {
            read_errors.extend(prob);
            StepResult::fail("server-fn-tracing").detail(read_errors.join("\n"))
        }
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::retired_server_fn;

    /// Build a one-file source set rooted at a `web/src/<vertical>/api.rs` path.
    fn src(vertical: &str, body: &str) -> Vec<(String, String)> {
        vec![(format!("web/src/{vertical}/api.rs"), body.to_string())]
    }

    // --- type admission is compiler-owned; source skip intent stays checked ---

    #[test]
    fn named_arguments_are_not_classified_by_type() {
        for body in [
            "#[macros::server]\npub async fn verify_email(token: RawToken) -> R {}\n",
            "#[macros::server]\npub async fn delete_media(filename: Filename) -> R {}\n",
            "#[macros::server(input = Json)]\n\
             pub async fn get_post_preview(a: Option<PostId>, b: common::ids::PostId) -> R {}\n",
        ] {
            assert_eq!(
                problems(&src("posts", body)),
                None,
                "TraceField trait resolution, not this source gate, owns type admission"
            );
        }
    }

    #[test]
    fn skip_names_must_name_plain_parameters() {
        let unknown = src(
            "email",
            "#[macros::server(skip(missing))]\npub async fn verify_email(token: RawToken) -> R {}\n",
        );
        let detail = problems(&unknown).expect("unknown skip name is a problem");
        assert!(detail.contains("missing"), "{detail}");
        assert!(detail.contains("parameter"), "{detail}");

        let skipped = src(
            "email",
            "#[macros::server(skip(token))]\npub async fn verify_email(token: RawToken) -> R {}\n",
        );
        assert_eq!(problems(&skipped), None);
    }

    #[test]
    fn skip_all_covers_every_argument() {
        let s = src(
            "posts",
            "#[macros::server(skip_all)]\npub async fn create_post(args: CreatePostArgs) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn smtp_update_request_uses_skip_all() {
        let s = src(
            "smtp",
            "#[macros::server(skip_all)]\npub async fn update_settings(request: UpdateSettingsRequest) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn passes_a_conforming_zero_arg_fn() {
        // Nothing is left to demand of a zero-arg fn: the macro emits the span and
        // its name, so the bare attribute is complete.
        let s = src("tags", "#[macros::server]\npub async fn list() -> R {}\n");
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn a_server_fn_directly_under_web_src_is_a_hard_error() {
        let s = vec![(
            "web/src/loose.rs".to_string(),
            "#[macros::server]\npub async fn x() -> R {}\n".to_string(),
        )];
        assert!(
            problems(&s)
                .expect("no vertical directory is an error")
                .contains("web/src/loose.rs")
        );
    }

    // --- the nameless-parameter rule ---

    #[test]
    fn a_parameter_bound_by_a_pattern_cannot_be_named_in_skip() {
        // A destructured parameter has no single identifier, so `skip(...)` cannot
        // reach it — naming its *type* or a component does not count. Only `skip_all`
        // covers it, and the message must say so.
        let unskipped = src(
            "posts",
            "#[macros::server]\npub async fn x((a, b): (u32, u32)) -> R {}\n",
        );
        let detail = problems(&unskipped).expect("a nameless parameter is a problem");
        assert!(detail.contains("skip_all"), "{detail}");
        assert!(detail.contains("pattern"), "{detail}");

        // Naming the components does not cover it: they are not parameter names.
        let named = src(
            "posts",
            "#[macros::server(skip(a, b))]\npub async fn x((a, b): (u32, u32)) -> R {}\n",
        );
        assert!(problems(&named).is_some(), "skip(a, b) must not cover it");

        // `skip_all` does.
        let all = src(
            "posts",
            "#[macros::server(skip_all)]\npub async fn x((a, b): (u32, u32)) -> R {}\n",
        );
        assert_eq!(problems(&all), None);
    }

    #[test]
    fn every_pattern_bound_parameter_requires_skip_all() {
        // The type is not the question: a pattern has no single parameter
        // identifier from which the macro can generate a projected field.
        let s = src(
            "posts",
            "#[macros::server]\npub async fn x(Wrapper(id): Wrapper<PostId>) -> R {}\n",
        );
        assert!(
            problems(&s)
                .expect("a nameless parameter is a problem whatever its type")
                .contains("skip_all")
        );
    }

    // --- default-deny on the attribute's own arguments ---

    #[test]
    fn fields_accepts_only_empty_declarations() {
        let accepted = src(
            "posts",
            "#[macros::server(fields(policy = tracing::field::Empty), skip(token))]\n\
             pub async fn x(token: RawToken) -> R {}\n",
        );
        assert_eq!(problems(&accepted), None);

        let valued = src(
            "posts",
            "#[macros::server(fields(who = token), skip(token))]\npub async fn x(token: RawToken) -> R {}\n",
        );
        let detail = problems(&valued).expect("value fields are rejected");
        assert!(detail.contains("fields"), "{detail}");
        assert!(detail.contains("Empty"), "{detail}");

        let disguised = src(
            "posts",
            "#[macros::server(fields(who = { tracing::field::Empty; token }), skip(token))]\n\
             pub async fn x(token: RawToken) -> R {}\n",
        );
        let detail = problems(&disguised).expect("non-empty field expression is rejected");
        assert!(detail.contains("fields"), "{detail}");
        assert!(detail.contains("Empty"), "{detail}");
    }

    #[test]
    fn an_unmodelled_attribute_argument_is_rejected() {
        // Default-deny mirrors the macro's own `route`: an argument neither side
        // models cannot silently change the generated span.
        let s = src(
            "posts",
            "#[macros::server(unknown(flag))]\npub async fn x() -> R {}\n",
        );
        assert!(
            problems(&s)
                .expect("an unmodelled arg is rejected")
                .contains("unknown")
        );
    }

    #[test]
    fn the_instrument_arguments_the_macro_does_not_forward_are_rejected() {
        // `err`/`ret` and `level`/`target`/`parent` were `#[tracing::instrument]`
        // spellings. The macro forwards none of them, so they are unmodelled
        // arguments now rather than special cases.
        for arg in ["err", "ret", "level = \"info\"", "target = \"t\""] {
            let s = src(
                "posts",
                &format!("#[macros::server({arg})]\npub async fn x() -> R {{}}\n"),
            );
            assert!(
                problems(&s).is_some_and(|d| d.contains("unrecognized")),
                "`{arg}` must be rejected as unmodelled"
            );
        }
    }

    #[test]
    fn a_positional_argument_is_rejected_rather_than_read_as_a_skip() {
        let s = src(
            "posts",
            "#[macros::server(some::path)]\npub async fn x() -> R {}\n",
        );
        assert!(
            problems(&s)
                .expect("a path argument is unmodelled")
                .contains("some::path")
        );
    }

    // --- the retired spelling (#714) ---

    #[test]
    fn a_fn_in_the_retired_spelling_is_not_this_gates_business() {
        // `is_server_attr` matches only `#[macros::server]`, so a fn still wearing
        // leptos's `#[server]` never reaches these source-shape rules. That is
        // deliberate and is not a hole: the macro spelling is the only supported
        // server-fn surface, and the runtime wire contract plus coverage snapshot
        // keep the real tree from becoming an empty silent green.
        let s = src(
            "email",
            &retired_server_fn(
                "(endpoint = \"/email/verify\")",
                "pub async fn verify_email((token,): (RawToken,)) -> R {}",
            ),
        );
        assert_eq!(problems(&s), None);

        // The same pattern-bound fn in the current spelling is a problem. This
        // makes the assertion above about enumeration rather than type admission.
        let converted = src(
            "email",
            "#[macros::server]\n\
             pub async fn verify_email((token,): (RawToken,)) -> R {}\n",
        );
        assert!(problems(&converted).is_some());
    }

    // --- fail-loud enumeration (AC 12) ---

    #[test]
    fn an_unparseable_file_is_reported_not_skipped() {
        let s = src("posts", "fn broken( {{{ not valid");
        assert!(
            problems(&s)
                .expect("a parse failure is reported")
                .contains("web/src/posts/api.rs")
        );
    }
}
