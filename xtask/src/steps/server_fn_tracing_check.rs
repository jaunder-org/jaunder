//! The `server-fn-tracing` static check (#511): every `#[macros::server]` fn in the
//! `web` crate must declare a PII-safe span.
//!
//! ADR-0011 wants end-to-end tracing, but 44 of the 55 `web` server fns shipped
//! with no span at all — a request into `create_post` produced nothing to correlate.
//! An unenforced convention is what allowed that gap, so the convention is a gate.
//!
//! **Enumeration** is shared with [`super::server_fn_registrar_check`] via
//! [`crate::web_server_fns`]; this module supplies only the tracing rules.
//!
//! The span itself — its presence, its placement, and its `web.<vertical>.<ident>`
//! name — is no longer anyone's to get wrong: `#[macros::server]` emits the
//! `#[tracing::instrument]` and derives the name (#714), so the rules that policed a
//! hand-written attribute went with the hand-written attribute. What is left is the
//! judgment the macro cannot make for an author:
//!
//! 1. **PII discipline is a default-deny type allowlist** — every parameter is
//!    either named in the attribute's `skip(...)` / covered by `skip_all`, or has a
//!    type on [`RECORDABLE_TYPES`]. An unlisted type is not recordable, so a
//!    newly-introduced argument type fails this gate until someone classifies it:
//!    the PII decision is forced when it arises rather than left to a reviewer
//!    noticing.
//! 2. **A parameter bound by a pattern cannot be skipped by name** — a destructured
//!    argument has no single identifier to write in `skip(...)`, so it is refused
//!    unless `skip_all` covers it. Otherwise it would be recorded by a span nobody
//!    could opt it out of.
//! 3. **Declared span fields must stay declaration-only** — `fields(...)` is allowed
//!    only as `field = tracing::field::Empty`. Values must be recorded later in the
//!    function body where the author has context, and a value expression in the macro
//!    argument is refused because the type allowlist never inspected it.
//! 4. **An unmodelled attribute argument is refused** — the macro forwards only
//!    `skip(...)`/`skip_all` and empty `fields(...)` declarations to the span, and
//!    `input = …` to `#[server]`. Anything else could record a value this allowlist
//!    never inspected, so it fails here until modelled.
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

/// Argument types whose values may be recorded on a span, each with the ground
/// that admits it (ADR-0011's addendum for #511).
///
/// The criterion is **"is this value already visible to the trace's reader, or
/// bounded by its own type?"** — not "did a user type it". Anything absent is
/// skipped, so adding an entry is a deliberate, reviewable change.
///
/// Note this is a list of *type names*, not a property the compiler can derive:
/// `Filename` and `AudienceName` are newtypes that validate a value's *shape*
/// while carrying arbitrary user text, whereas `u32` is a primitive that bounds
/// its contents completely. Newtype-ness is not the test.
const RECORDABLE_TYPES: &[(&str, &str)] = &[
    // Bounded by the type itself — admits no free text.
    ("PostId", "opaque row id"),
    ("AudienceId", "opaque row id"),
    ("SubscriptionId", "opaque row id"),
    ("ContentHash", "sha256 digest"),
    ("PageSize", "bounded page count"),
    ("PageOffset", "bounded page offset"),
    ("RetentionCount", "bounded count, min 1"),
    ("InviteTtlHours", "bounded hour count"),
    ("UtcInstant", "pagination cursor timestamp"),
    (
        "PageCursor",
        "keyset paging token: exactly the UtcInstant + PostId pair above",
    ),
    ("PostFormat", "bounded enum"),
    ("MediaSource", "bounded enum"),
    ("BackupMode", "bounded enum"),
    ("u32", "bounded integer"),
    ("bool", "two-valued flag"),
    // Operator configuration — set by the operator, who reads the traces.
    // ADR-0011 prohibits *user* PII and secrets; an operator's own settings are
    // neither, and they are the informative content of these write-path spans.
    ("DestinationPath", "operator-configured backup path"),
    ("SiteTitle", "operator-configured site title"),
    ("BaseUrl", "operator-configured site base URL"),
    ("BackupSchedule", "operator-configured cron expression"),
    // Already published — a component of a public permalink, so already in any
    // reverse-proxy access log.
    ("Slug", "public post permalink component"),
    ("PermalinkDate", "public post permalink component"),
    ("Tag", "public tag-listing URL component"),
    // Permitted outright by ADR-0011: "usernames are public identifiers and
    // acceptable".
    (
        "Username",
        "ADR-0011 carve-out: usernames are public identifiers",
    ),
];

/// Whether a type may be recorded, by its reduced name.
fn is_recordable(reduced: Option<&str>) -> bool {
    reduced.is_some_and(|name| RECORDABLE_TYPES.iter().any(|(t, _)| *t == name))
}

/// The comparable name of a parameter type: unwrap `&` and `Option<…>`, then take
/// the last path segment, so `common::media::Filename` and `Option<PostId>` reduce
/// to `Filename` and `PostId`.
///
/// Any shape this cannot reduce (a tuple, a slice, an `impl Trait`) yields `None`,
/// which is **not recordable** — default-deny, so an unrecognized type must be
/// skipped rather than silently recorded.
fn reduce_type(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => reduce_type(&r.elem),
        syn::Type::Paren(p) => reduce_type(&p.elem),
        syn::Type::Path(tp) => {
            let seg = tp.path.segments.last()?;
            if seg.ident == "Option" {
                let syn::PathArguments::AngleBracketed(ab) = &seg.arguments else {
                    return None;
                };
                let syn::GenericArgument::Type(inner) = ab.args.first()? else {
                    return None;
                };
                return reduce_type(inner);
            }
            Some(seg.ident.to_string())
        }
        _ => None,
    }
}

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

/// What a `#[macros::server]` attribute declares about its span.
///
/// The macro forwards `skip(...)` / `skip_all` plus empty `fields(...)`
/// declarations to `#[tracing::instrument]`, forwards `input = …` to `#[server]`,
/// and rejects everything else. `endpoint` and `name` are still derived, and
/// `fields(...)` is accepted only as declaration-only
/// `field = tracing::field::Empty` entries. This mirrors that **default-deny**:
/// an argument the macro might one day forward but this gate does not model could
/// record a value the allowlist never inspected, so it fails here until modelled.
fn span_args(attr: &syn::Attribute) -> Result<SpanArgs, String> {
    let mut out = SpanArgs::default();
    let Some(args) = web_server_fns::server_attr_args(attr)? else {
        // The bare `#[macros::server]`: nothing skipped, so every parameter must be
        // recordable on its own.
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
                    "unrecognized #[macros::server] argument `{other}` — the macro forwards only \
                     `skip(...)`/`skip_all` and empty `fields(...)` declarations to the span, \
                     forwards `input = …` to #[server], and an unmodelled argument could record a \
                     value the allowlist never inspected"
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

    for p in &f.params {
        let reduced = reduce_type(&p.ty);
        let shown = reduced.as_deref().unwrap_or("<unrecognized type>");
        // A destructured parameter has no single name to put in `skip(...)`, so it
        // can be neither skipped nor reasoned about. `skip_all` still covers it.
        let Some(param) = p.name.as_deref() else {
            if !parsed.skip_all {
                lines.push(format!(
                    "{at}: a parameter of type `{shown}` is bound by a pattern rather than a \
                     plain identifier, so it cannot be named in skip(...) — bind it to an \
                     identifier, or use skip_all"
                ));
            }
            continue;
        };
        let recordable = is_recordable(reduced.as_deref());
        let skipped = parsed.skip_all || parsed.skipped.contains(param);
        if !recordable && !skipped {
            lines.push(format!(
                "{at}: argument `{param}: {shown}` is neither skipped nor recordable — add it to \
                 skip(...) (or use skip_all), or, if the value carries no user data, add \
                 \"{shown}\" to RECORDABLE_TYPES in \
                 xtask/src/steps/server_fn_tracing_check.rs with its justification"
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
        "  recovery: every web #[macros::server] fn has every argument skipped — named in \
         skip(...) or covered by skip_all — or its type on RECORDABLE_TYPES (#511; ADR-0011)"
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

    // --- the PII allowlist (AC 4/10) ---

    #[test]
    fn flags_an_argument_that_is_neither_skipped_nor_recordable() {
        // The substantive guarantee: an unskipped non-recordable argument fails, and
        // the same argument named in `skip(...)` passes.
        let s = src(
            "email",
            "#[macros::server]\npub async fn verify_email(token: RawToken) -> R {}\n",
        );
        let detail = problems(&s).expect("an unclassified arg is a problem");
        assert!(detail.contains("token"), "{detail}");
        assert!(detail.contains("RawToken"), "{detail}");
        assert!(detail.contains("skip"), "names the skip remedy: {detail}");
        assert!(
            detail.contains("RECORDABLE_TYPES"),
            "names the allowlist remedy: {detail}"
        );

        let skipped = src(
            "email",
            "#[macros::server(skip(token))]\npub async fn verify_email(token: RawToken) -> R {}\n",
        );
        assert_eq!(problems(&skipped), None);
    }

    #[test]
    fn accepts_a_recordable_arg_unskipped_and_reduces_option_and_path() {
        let s = src(
            "posts",
            "#[macros::server(input = Json)]\n\
             pub async fn get_post_preview(a: Option<PostId>, b: common::ids::PostId) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
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
    fn filename_is_not_recordable() {
        // A media URL is only discoverable once a published post references it, and
        // a Filename is arbitrary user text — it fails the same test that skips Bio.
        let s = src(
            "media",
            "#[macros::server]\npub async fn delete_media(filename: Filename) -> R {}\n",
        );
        assert!(
            problems(&s)
                .expect("Filename must be skipped")
                .contains("Filename")
        );
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
    fn a_nameless_parameter_of_a_recordable_type_is_still_refused() {
        // Recordability is not the question — `PostId` is on the allowlist, but the
        // parameter still cannot be opted out of the span by name, so the rule bites
        // independently of the type allowlist.
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
        // Default-deny, mirroring the macro's own `route`: an argument this gate
        // does not model could record a value the allowlist never inspected.
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
        // leptos's `#[server]` never reaches these rules — its unskipped `RawToken`
        // reads as no problem at all. That is deliberate and is *not* a hole: what
        // catches a stray old-spelling fn is
        // `enumeration_of_web_src_matches_the_registrar` in the registrar gate, which
        // fails the moment the real enumeration and the registrar disagree.
        let s = src(
            "email",
            &retired_server_fn(
                "(endpoint = \"/email/verify\")",
                "pub async fn verify_email(token: RawToken) -> R {}",
            ),
        );
        assert_eq!(problems(&s), None);

        // The same fn in the current spelling is a problem, which is what makes the
        // assertion above about *enumeration* rather than about the PII rule.
        let converted = src(
            "email",
            "#[macros::server]\npub async fn verify_email(token: RawToken) -> R {}\n",
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
