//! The `server-fn-tracing` static check (#511): every `#[server]` fn in the `web`
//! crate must carry a PII-safe `#[tracing::instrument]` span.
//!
//! ADR-0011 wants end-to-end tracing, but 44 of the 55 `web` server fns shipped
//! with no span at all — a request into `create_post` produced nothing to correlate.
//! An unenforced convention is what allowed that gap, so the convention is a gate.
//!
//! **Enumeration** is shared with [`super::server_fn_registrar_check`] via
//! [`crate::web_server_fns`]; this module supplies only the tracing rules.
//!
//! Three things are checked:
//!
//! 1. **Presence and placement** — a `#[tracing::instrument]` after the `#[server]`
//!    attribute. All the pre-existing sites use that order and build for both
//!    targets, so it is the arrangement known to produce a server-side span.
//! 2. **The span name is derived, so the gate writes it** — `web.<vertical>.<fn
//!    ident>`, where the vertical is the first path segment under `web/src`. Because
//!    the name is a pure function of source path and identifier, asking an author to
//!    type it would be asking them to restate what the file already says, and to keep
//!    55 copies in sync by hand. So `cargo xtask check` **fills it in**
//!    ([`Mode::Fix`], the same contract `fmt` has) and `cargo xtask validate`
//!    verifies without mutating. Write `#[tracing::instrument(skip_all)]`; the gate
//!    supplies `name = "web.posts.create_post"`. The literal still lands in the
//!    source, so an operator reading a span name can grep for it — and when the fn
//!    idents shed their vestigial vertical nouns (#684), re-running the gate rewrites
//!    all 55.
//! 3. **PII discipline is a default-deny type allowlist** — every parameter is
//!    either skipped or has a type on [`RECORDABLE_TYPES`]. An unlisted type is not
//!    recordable, so a newly-introduced argument type fails this gate until someone
//!    classifies it: the PII decision is forced when it arises rather than left to a
//!    reviewer noticing. `fields(...)` value expressions are held to the same list,
//!    since `skip(email)` + `fields(who = %email)` would otherwise sail through.
//!
//! Like the registrar guard this is **mandatory with no per-fn opt-out**, and
//! **fail-loud**: an unparseable or unreadable file is reported, never skipped,
//! because a file we cannot enumerate could hide a bare `#[server]` fn.

use std::collections::BTreeSet;
use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Meta, Token};

use crate::result::{CommandResult, Mode, StepResult};
use crate::web_server_fns::{
    self, apply_fixes, rewrite_attr_arg, vertical_of, LineFix, WebServerFn, WEB_SRC,
};

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
    ("AbsoluteUrl", "operator-configured site base URL"),
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

/// `#[tracing::instrument]` arguments that neither name the span nor decide what is
/// recorded, so the gate accepts and ignores them. Everything not listed here and
/// not handled explicitly is rejected — including `follows_from`, whose expression
/// this gate does not inspect.
const IGNORED_ARGS: &[&str] = &["level", "target", "parent"];

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

/// What an `#[tracing::instrument]` attribute declares.
#[derive(Default)]
struct Instrument {
    /// The `name = "…"` literal, if present.
    name: Option<String>,
    /// Parameter names named in `skip(...)`.
    skipped: BTreeSet<String>,
    /// Whether `skip_all` was given.
    skip_all: bool,
    /// Identifiers appearing in the *value* half of each `fields(...)` entry.
    field_value_idents: BTreeSet<String>,
}

/// Parse the arguments of a `#[tracing::instrument(...)]` attribute.
///
/// Uses `syn::Meta`, not a hand-rolled token walk: `skip(a)` and
/// `fields(who = %token)` are both `Meta::List`, whose inner `TokenStream` is left
/// unparsed — the `%`/`?` sigils never reach a `Meta` parser. Only the *inside* of
/// `skip(...)` / `fields(...)` needs token-level handling.
fn parse_instrument(attr: &syn::Attribute) -> Result<Instrument, String> {
    let mut out = Instrument::default();
    let args = match &attr.meta {
        // A bare `#[tracing::instrument]`: no arguments at all, so no name.
        Meta::Path(_) => return Ok(out),
        Meta::NameValue(_) => return Err("unexpected `#[tracing::instrument = ...]` form".into()),
        Meta::List(_) => attr
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .map_err(|e| format!("cannot parse #[tracing::instrument(...)] arguments: {e}"))?,
    };

    for arg in args {
        let Some(ident) = arg.path().get_ident().map(ToString::to_string) else {
            return Err(format!(
                "unrecognized #[tracing::instrument] argument `{}`",
                path_text(arg.path())
            ));
        };
        match ident.as_str() {
            "name" => {
                let Meta::NameValue(nv) = &arg else {
                    return Err("`name` must be `name = \"…\"`".into());
                };
                let syn::Expr::Lit(lit) = &nv.value else {
                    return Err("`name` must be a string literal".into());
                };
                let syn::Lit::Str(s) = &lit.lit else {
                    return Err("`name` must be a string literal".into());
                };
                out.name = Some(s.value());
            }
            "skip" => {
                let Meta::List(list) = &arg else {
                    return Err("`skip` must be `skip(a, b)`".into());
                };
                out.skipped.extend(idents_in(&list.tokens));
            }
            "skip_all" => out.skip_all = true,
            "fields" => {
                let Meta::List(list) = &arg else {
                    return Err("`fields` must be `fields(k = v)`".into());
                };
                out.field_value_idents
                    .extend(field_value_idents(&list.tokens));
            }
            "err" | "ret" => {
                return Err(format!(
                    "`{ident}` is not permitted on a #[server] fn: it changes what the span \
                     records and needs its own PII review of the WebError Display chain \
                     (deliberately out of scope for #511)"
                ))
            }
            other if IGNORED_ARGS.contains(&other) => {}
            other => {
                return Err(format!(
                    "unrecognized #[tracing::instrument] argument `{other}` — this gate models \
                     only name/skip/skip_all/fields/level/target/parent, and an unmodelled \
                     argument could record a value the allowlist never inspected; extend the gate"
                ))
            }
        }
    }
    Ok(out)
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

/// The identifiers appearing in the *value* half of each `fields(...)` entry.
///
/// The field **name** (left of `=`) is deliberately excluded: a field may be named
/// after a skipped argument as long as its value does not read it, so
/// `fields(label = "redacted")` is fine. Collecting the left side would reject a
/// field that records nothing at all.
fn field_value_idents(tokens: &TokenStream) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for entry in split_top_level_commas(tokens) {
        // Split on the first top-level `=`; everything after it is the value.
        let mut value = TokenStream::new();
        let mut seen_eq = false;
        let mut name_tokens = Vec::new();
        for tt in entry.clone() {
            if !seen_eq {
                if let TokenTree::Punct(ref p) = tt {
                    if p.as_char() == '=' {
                        seen_eq = true;
                        continue;
                    }
                }
                name_tokens.push(tt);
            } else {
                value.extend(std::iter::once(tt));
            }
        }
        if seen_eq {
            collect_idents(&value, &mut out);
        } else {
            // A shorthand field (`fields(post_id)`) records the argument itself,
            // so its identifiers are values, not a bare name.
            let bare: TokenStream = name_tokens.into_iter().collect();
            collect_idents(&bare, &mut out);
        }
    }
    out
}

/// Split a token stream on top-level commas (commas inside a group are not
/// separators).
fn split_top_level_commas(tokens: &TokenStream) -> Vec<TokenStream> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    for tt in tokens.clone() {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == ',' => {
                out.push(current.drain(..).collect());
            }
            _ => current.push(tt),
        }
    }
    if !current.is_empty() {
        out.push(current.into_iter().collect());
    }
    out
}

/// A `path::to::thing` rendered for an error message.
fn path_text(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// Whether an attribute is a `#[tracing::instrument]` / `#[instrument]`.
///
/// Matched by the path's **last** segment, so an `use tracing::instrument;` import
/// style is not a silent gate bypass.
fn is_instrument(attr: &syn::Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|s| s.ident == "instrument")
}

/// Whether an attribute is a `cfg_attr` that wraps an `instrument`.
fn is_cfg_attr_instrument(attr: &syn::Attribute) -> bool {
    if !attr.path().is_ident("cfg_attr") {
        return false;
    }
    let Meta::List(list) = &attr.meta else {
        return false;
    };
    idents_in(&list.tokens).contains("instrument")
}

/// The span declaration to hold the PII rule against, plus any presence/placement
/// or `name` problems found on the way — or, in the `Err` arm, the problems that
/// leave nothing to hold it against at all.
///
/// Two sources, because two spellings coexist during the #714 conversion. Under
/// `#[macros::server]` rules 1 and 2 have nothing to inspect — the macro emits the
/// `#[tracing::instrument]` and its `name`, so neither can be missing, misplaced or
/// stale — and the `skip(...)` / `skip_all` the PII rule needs sit on the macro
/// attribute itself.
fn span_declaration(
    f: &WebServerFn,
    vertical: &str,
    at: &str,
) -> Result<(Instrument, Vec<String>), Vec<String>> {
    if f.uses_macro_attr {
        return match macro_attr_span(&f.attrs[f.server_attr_index]) {
            Ok(parsed) => Ok((parsed, Vec::new())),
            Err(e) => Err(vec![format!("{at}: {e}")]),
        };
    }

    // A conditionally-present span is exactly the inconsistency this gate exists to
    // remove, so check it before "missing" — otherwise the message would mislead.
    if f.attrs.iter().any(is_cfg_attr_instrument) {
        return Err(vec![format!(
            "{at}: #[tracing::instrument] is wrapped in a #[cfg_attr(...)] — a span that exists \
             only under some cfg is the inconsistency this gate prevents; apply it unconditionally"
        )]);
    }

    let Some(index) = f.attrs.iter().position(is_instrument) else {
        return Err(vec![format!(
            "{at}: #[server] fn has no #[tracing::instrument] — add \
             #[tracing::instrument(name = \"web.{vertical}.{}\")] directly after #[server] (#511)",
            f.ident
        )]);
    };

    if index < f.server_attr_index {
        return Err(vec![format!(
            "{at}: #[tracing::instrument] must come *after* #[server], not before"
        )]);
    }

    let parsed = match parse_instrument(&f.attrs[index]) {
        Ok(p) => p,
        Err(e) => return Err(vec![format!("{at}: {e}")]),
    };

    let mut lines = Vec::new();
    let expected = format!("web.{vertical}.{}", f.ident);
    match &parsed.name {
        None => lines.push(format!(
            "{at}: #[tracing::instrument] has no `name` — a span name is required, since tracing \
             would otherwise default it to the fn ident and lose the `web.` prefix; expected \
             name = \"{expected}\""
        )),
        Some(name) if *name != expected => lines.push(format!(
            "{at}: span name is \"{name}\" but must be \"{expected}\" — the name is derived from \
             the source path and fn ident"
        )),
        Some(_) => {}
    }
    Ok((parsed, lines))
}

/// What a `#[macros::server]` attribute declares about its span.
///
/// The macro forwards exactly `skip(...)` / `skip_all` to `#[tracing::instrument]`
/// and `input = …` to `#[server]`, and rejects everything else — `endpoint` and
/// `name` because it derives them, `fields(...)` because it retired the value-side
/// PII check along with it (`macros/src/server_fn.rs`'s `route`). This mirrors that
/// **default-deny**: an argument the macro might one day forward but this gate does
/// not model could record a value the allowlist never inspected, so it fails here
/// until modelled, exactly as an unmodelled `#[tracing::instrument]` argument does.
fn macro_attr_span(attr: &syn::Attribute) -> Result<Instrument, String> {
    let mut out = Instrument::default();
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
            // Routed to `#[server]`, not to the span — it records nothing.
            "input" => {}
            other => {
                return Err(format!(
                    "unrecognized #[macros::server] argument `{other}` — the macro forwards only \
                     `skip(...)`/`skip_all` to the span and `input = …` to #[server], and an \
                     unmodelled argument could record a value the allowlist never inspected"
                ))
            }
        }
    }
    Ok(out)
}

/// Every problem with one `#[server]` fn, as human-readable lines (without the
/// file prefix, which the caller adds).
fn problems_with(f: &WebServerFn, vertical: &str) -> Vec<String> {
    let at = format!("line {}: `{}`", f.line, f.ident);
    let (parsed, mut lines) = match span_declaration(f, vertical, &at) {
        Ok(v) => v,
        Err(fatal) => return fatal,
    };

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
        if !recordable && parsed.field_value_idents.contains(param) {
            lines.push(format!(
                "{at}: fields(...) reads argument `{param}: {shown}`, which is not recordable — \
                 skipping an argument does not permit recording it through a field"
            ));
        }
    }

    lines
}

/// The name rewrites `Mode::Fix` should apply to one source file.
///
/// Only fns that are *otherwise* well-formed are rewritten: if a fn has no
/// instrument attribute, or one this gate refuses to parse, there is nothing to
/// safely edit and the problem is reported instead.
fn name_fixes(path: &str, src: &str) -> Vec<LineFix> {
    let Ok(fns) = web_server_fns::server_fns_in(src) else {
        return Vec::new();
    };
    let Ok(vertical) = vertical_of(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = src.lines().collect();
    let mut fixes = Vec::new();
    for f in &fns {
        // The macro emits the span name; there is no source attribute to write it
        // into, and a stray hand-written `#[tracing::instrument]` left on a
        // converted fn must not be quietly patched up either — it should be deleted.
        if f.uses_macro_attr {
            continue;
        }
        if f.attrs.iter().any(is_cfg_attr_instrument) {
            continue;
        }
        let Some(index) = f.attrs.iter().position(is_instrument) else {
            continue;
        };
        if index < f.server_attr_index || parse_instrument(&f.attrs[index]).is_err() {
            continue;
        }
        let span = f.attrs[index].span();
        let (start, end) = (span.start().line, span.end().line);
        if start == 0 || end > lines.len() {
            continue;
        }
        let current = lines[start - 1..end].join("\n");
        let desired = format!("web.{vertical}.{}", f.ident);
        if let Some(replacement) = rewrite_attr_arg(&current, "instrument", "name", &desired, true)
        {
            fixes.push((start, end, replacement));
        }
    }
    fixes
}

/// The failure detail for every non-conforming `#[server]` fn, or `None` when
/// every one conforms. Pure given its inputs, so it is unit-tested directly.
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
        let vertical = match vertical_of(path) {
            Ok(v) => v,
            Err(msg) => {
                lines.push(msg);
                continue;
            }
        };
        for f in &fns {
            lines.extend(
                problems_with(f, vertical)
                    .into_iter()
                    .map(|l| format!("{path}: {l}")),
            );
        }
    }

    if lines.is_empty() {
        return None;
    }
    lines.sort();
    lines.push(
        "  recovery: every web #[server] fn carries \
         #[tracing::instrument(name = \"web.<vertical>.<fn>\")] after #[server], with every \
         argument skipped or its type on RECORDABLE_TYPES (#511; ADR-0011)"
            .to_string(),
    );
    Some(lines.join("\n"))
}

/// Scan every `web/src` Rust file for `#[server]` fns and check each carries a
/// conforming span. A missing `web/src` tree or an unreadable file is a hard
/// failure (not a silent pass), so a moved path can never quietly disable the guard.
pub fn run(mode: Mode, result: &mut CommandResult) {
    let web = match web_server_fns::read_web_sources(Path::new(WEB_SRC)) {
        Ok(v) => v,
        Err(e) => {
            result.push(StepResult::fail("server-fn-tracing").detail(e));
            return;
        }
    };
    let mut read_errors = web.read_errors;
    let mut sources = web.sources;

    // `Mode::Fix` (i.e. `cargo xtask check`) writes the derived span name, the
    // same contract `fmt` has; `Mode::Check` (`validate`) never mutates the tree.
    if matches!(mode, Mode::Fix) {
        for (path, src) in &mut sources {
            let fixes = name_fixes(path, src);
            if fixes.is_empty() {
                continue;
            }
            let fixed = apply_fixes(src, fixes);
            if let Err(e) = std::fs::write(&*path, &fixed) {
                read_errors.push(format!("{path}: cannot write span-name fix: {e}"));
                continue;
            }
            *src = fixed;
        }
    }

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

    /// Build a one-file source set rooted at a `web/src/<vertical>/api.rs` path.
    fn src(vertical: &str, body: &str) -> Vec<(String, String)> {
        vec![(format!("web/src/{vertical}/api.rs"), body.to_string())]
    }

    // --- presence (AC 1/8) ---

    #[test]
    fn flags_a_server_fn_with_no_instrument_attribute() {
        let s = src(
            "posts",
            "#[server(endpoint = \"/x\")]\npub async fn create_post() -> R {}\n",
        );
        let detail = problems(&s).expect("a bare #[server] fn is a problem");
        assert!(detail.contains("create_post"), "{detail}");
        assert!(detail.contains("web/src/posts/api.rs"), "{detail}");
    }

    #[test]
    fn passes_a_conforming_zero_arg_fn() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.list_drafts\")]\npub async fn list_drafts() -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    // --- ordering (AC 2) ---

    #[test]
    fn flags_instrument_placed_before_server() {
        let s = src(
            "posts",
            "#[tracing::instrument(name = \"web.posts.x\")]\n#[server]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s)
            .expect("wrong order is a problem")
            .contains("after"));
    }

    // --- derived name (AC 3/9) ---

    #[test]
    fn flags_a_span_name_that_is_not_the_derived_one() {
        let s = src(
            "tags",
            "#[server]\n#[tracing::instrument(name = \"tags.list\")]\npub async fn list_tags() -> R {}\n",
        );
        let detail = problems(&s).expect("a wrong name is a problem");
        assert!(detail.contains("web.tags.list_tags"), "{detail}");
        assert!(detail.contains("tags.list"), "{detail}");
    }

    #[test]
    fn derives_the_vertical_from_the_first_segment_not_the_file() {
        // posts/api/listing.rs -> vertical `posts`, not `api`.
        let s = vec![(
            "web/src/posts/api/listing.rs".to_string(),
            "#[server]\n#[tracing::instrument(name = \"web.posts.list_home_feed\")]\npub async fn list_home_feed() -> R {}\n".to_string(),
        )];
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn a_server_fn_directly_under_web_src_is_a_hard_error() {
        let s = vec![(
            "web/src/loose.rs".to_string(),
            "#[server]\n#[tracing::instrument(name = \"web.loose.x\")]\npub async fn x() -> R {}\n"
                .to_string(),
        )];
        assert!(problems(&s)
            .expect("no vertical directory is an error")
            .contains("web/src/loose.rs"));
    }

    // --- the PII allowlist (AC 4/10) ---

    #[test]
    fn flags_an_argument_that_is_neither_skipped_nor_recordable() {
        let s = src(
            "email",
            "#[server]\n#[tracing::instrument(name = \"web.email.verify_email\")]\npub async fn verify_email(token: RawToken) -> R {}\n",
        );
        let detail = problems(&s).expect("an unclassified arg is a problem");
        assert!(detail.contains("token"), "{detail}");
        assert!(detail.contains("RawToken"), "{detail}");
        assert!(detail.contains("skip"), "names the skip remedy: {detail}");
        assert!(
            detail.contains("RECORDABLE_TYPES"),
            "names the allowlist remedy: {detail}"
        );
    }

    #[test]
    fn accepts_a_recordable_arg_unskipped_and_reduces_option_and_path() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.get_post_preview\")]\n\
             pub async fn get_post_preview(a: Option<PostId>, b: common::ids::PostId) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn skip_all_covers_every_argument() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.create_post\", skip_all)]\npub async fn create_post(args: CreatePostArgs) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn filename_is_not_recordable() {
        // A media URL is only discoverable once a published post references it, and
        // a Filename is arbitrary user text — it fails the same test that skips Bio.
        let s = src(
            "media",
            "#[server]\n#[tracing::instrument(name = \"web.media.delete_media\")]\npub async fn delete_media(filename: Filename) -> R {}\n",
        );
        assert!(problems(&s)
            .expect("Filename must be skipped")
            .contains("Filename"));
    }

    // --- the fields(...) bypass (AC 11) ---

    #[test]
    fn flags_a_fields_value_reading_a_non_recordable_argument() {
        let s = src(
            "email",
            "#[server]\n#[tracing::instrument(name = \"web.email.verify_email\", skip(token), fields(who = %token))]\npub async fn verify_email(token: RawToken) -> R {}\n",
        );
        assert!(problems(&s)
            .expect("a fields bypass is a problem")
            .contains("token"));
    }

    #[test]
    fn allows_a_field_named_after_a_skipped_argument_when_the_value_does_not_read_it() {
        // The field-name position (left of `=`) is excluded from collection.
        let s = src(
            "auth",
            "#[server]\n#[tracing::instrument(name = \"web.auth.login\", skip(label), fields(label = \"redacted\"))]\npub async fn login(label: Option<String>) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn allows_a_fields_value_reading_a_recordable_argument() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.publish_post\", skip_all, fields(post_id = %post_id))]\npub async fn publish_post(post_id: PostId) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    // --- what counts as the attribute ---

    #[test]
    fn accepts_a_bare_instrument_path() {
        let s = src(
            "posts",
            "#[server]\n#[instrument(name = \"web.posts.list_drafts\")]\npub async fn list_drafts() -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn rejects_a_cfg_attr_wrapped_instrument() {
        let s = src(
            "posts",
            "#[server]\n#[cfg_attr(feature = \"server\", tracing::instrument(name = \"web.posts.x\"))]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s)
            .expect("cfg_attr is a hard error")
            .contains("cfg_attr"));
    }

    #[test]
    fn rejects_the_err_argument() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.x\", err)]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s).expect("err is rejected").contains("err"));
    }

    #[test]
    fn rejects_the_ret_argument() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.x\", ret)]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s).expect("ret is rejected").contains("ret"));
    }

    #[test]
    fn rejects_an_unrecognized_instrument_argument() {
        // Default-deny: an argument this gate does not model could record something
        // the allowlist never saw, so it fails until modelled.
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.x\", follows_from = y)]\npub async fn x() -> R {}\n",
        );
        assert!(problems(&s)
            .expect("an unmodelled arg is rejected")
            .contains("follows_from"));
    }

    #[test]
    fn tolerates_level_target_and_parent() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument(name = \"web.posts.x\", target = \"t\", parent = None)]\npub async fn x() -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn a_missing_name_is_its_own_failure() {
        let s = src(
            "posts",
            "#[server]\n#[tracing::instrument]\npub async fn x() -> R {}\n",
        );
        let detail = problems(&s).expect("a missing name is a problem");
        assert!(detail.contains("name"), "{detail}");
        assert!(detail.contains("required"), "{detail}");
    }

    // --- Mode::Fix — the gate writes the derived name ---

    // The attribute-rewrite primitive itself moved to `web_server_fns` in #684 and
    // is unit-tested there, including these `instrument`/`name` cases. What stays
    // here is this gate's use of it: `name_fixes` deciding *which* attributes to
    // rewrite and to what.

    #[test]
    fn name_fixes_targets_the_attribute_lines_and_apply_splices_them() {
        let src = "#[server(endpoint = \"/list_tags\")]\n\
                   #[tracing::instrument(skip(prefix))]\n\
                   pub async fn list_tags(prefix: Option<String>) -> R {}\n";
        let fixes = name_fixes("web/src/tags/api.rs", src);
        assert_eq!(fixes.len(), 1);
        assert_eq!((fixes[0].0, fixes[0].1), (2, 2));
        let fixed = apply_fixes(src, fixes);
        assert!(
            fixed.contains("name = \"web.tags.list_tags\", skip(prefix)"),
            "{fixed}"
        );
        // Untouched lines survive, and so does the trailing newline.
        assert!(
            fixed.starts_with("#[server(endpoint = \"/list_tags\")]\n"),
            "{fixed}"
        );
        assert!(fixed.ends_with("-> R {}\n"), "{fixed}");
    }

    #[test]
    fn fixing_a_conforming_tree_is_a_no_op() {
        let src = "#[server]\n#[tracing::instrument(name = \"web.tags.list_tags\")]\npub async fn list_tags() -> R {}\n";
        assert!(name_fixes("web/src/tags/api.rs", src).is_empty());
    }

    #[test]
    fn nothing_is_rewritten_when_there_is_no_instrument_to_edit() {
        // A missing span stays a *reported* problem: inserting the attribute would
        // mean guessing the skip list, which is the one judgment the gate refuses
        // to make for you.
        let code = "#[server]\npub async fn create_post(args: CreatePostArgs) -> R {}\n";
        assert!(name_fixes("web/src/posts/api.rs", code).is_empty());
        assert!(problems(&src("posts", code)).is_some());
    }

    #[test]
    fn nothing_is_rewritten_for_a_file_with_no_vertical_directory() {
        let src = "#[server]\n#[tracing::instrument]\npub async fn x() -> R {}\n";
        assert!(name_fixes("web/src/loose.rs", src).is_empty());
    }

    #[test]
    fn applying_two_fixes_in_one_file_keeps_both_line_ranges_valid() {
        let src = "#[server]\n#[tracing::instrument]\npub async fn a() -> R {}\n\
                   #[server]\n#[tracing::instrument]\npub async fn b() -> R {}\n";
        let fixes = name_fixes("web/src/posts/api.rs", src);
        assert_eq!(fixes.len(), 2);
        let fixed = apply_fixes(src, fixes);
        assert!(fixed.contains("name = \"web.posts.a\""), "{fixed}");
        assert!(fixed.contains("name = \"web.posts.b\""), "{fixed}");
    }

    // --- the `#[macros::server]` spelling (#714) ---

    #[test]
    fn the_pii_rule_still_bites_under_the_macro_spelling() {
        // The substantive guarantee: rules 1 and 2 belong to the macro, but an
        // unskipped non-recordable argument must still fail — and the same argument
        // skipped on the macro attribute must pass.
        let unskipped = src(
            "email",
            "#[macros::server]\npub async fn verify(token: RawToken) -> R {}\n",
        );
        let detail = problems(&unskipped).expect("an unclassified arg is still a problem");
        assert!(detail.contains("token"), "{detail}");
        assert!(detail.contains("RawToken"), "{detail}");

        let skipped = src(
            "email",
            "#[macros::server(skip(token))]\npub async fn verify(token: RawToken) -> R {}\n",
        );
        assert_eq!(problems(&skipped), None);
    }

    #[test]
    fn skip_all_on_the_macro_attribute_covers_every_argument() {
        let s = src(
            "posts",
            "#[macros::server(skip_all)]\npub async fn create(args: CreatePostArgs) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn a_macro_attr_fn_needs_no_instrument_attribute_or_name() {
        // Rule 1 (presence/placement) and rule 2 (the derived `name`) have nothing
        // to inspect: the macro emits both. Demanding them would fail all 55 fns.
        let s = src("tags", "#[macros::server]\npub async fn list() -> R {}\n");
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn a_recordable_argument_still_needs_no_skip_under_the_macro_spelling() {
        let s = src(
            "posts",
            "#[macros::server(input = Json)]\npub async fn get(id: Option<PostId>) -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn a_nameless_parameter_still_needs_skip_all_under_the_macro_spelling() {
        let s = src(
            "posts",
            "#[macros::server]\npub async fn x((a, b): (u32, u32)) -> R {}\n",
        );
        assert!(problems(&s)
            .expect("a destructured parameter cannot be named in skip(...)")
            .contains("skip_all"));
    }

    #[test]
    fn an_unmodelled_macro_attr_argument_is_rejected() {
        // Default-deny, mirroring the macro's own `route`: an argument this gate
        // does not model could record a value the allowlist never inspected.
        let s = src(
            "posts",
            "#[macros::server(fields(who = %token))]\npub async fn x(token: RawToken) -> R {}\n",
        );
        assert!(problems(&s)
            .expect("an unmodelled arg is rejected")
            .contains("fields"));
    }

    #[test]
    fn nothing_is_rewritten_for_a_macro_attr_fn() {
        let code = "#[macros::server(skip_all)]\npub async fn create(args: A) -> R {}\n";
        assert!(name_fixes("web/src/posts/api.rs", code).is_empty());
    }

    // --- fail-loud enumeration (AC 12) ---

    #[test]
    fn an_unparseable_file_is_reported_not_skipped() {
        let s = src("posts", "fn broken( {{{ not valid");
        assert!(problems(&s)
            .expect("a parse failure is reported")
            .contains("web/src/posts/api.rs"));
    }
}
