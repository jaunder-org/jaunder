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
//! 2. **The span name is derived, so it is checked by equality** —
//!    `web.<vertical>.<fn ident>`, where the vertical is the first path segment
//!    under `web/src`. Nothing is left to judgment, and when the fn idents later
//!    shed their vestigial vertical nouns (#684) every span name improves for free.
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
use syn::{Meta, Token};

use crate::result::{CommandResult, StepResult};
use crate::web_server_fns::{self, WebServerFn, WEB_SRC};

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
/// recorded, so the gate accepts and ignores them.
const IGNORED_ARGS: &[&str] = &["level", "target", "parent", "follows_from_expr"];

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

/// The vertical a source file belongs to: the first path segment under `web/src`.
///
/// A `#[server]` fn in a file directly under `web/src` has no vertical directory,
/// so there is no honest name to derive — that is an error naming the file, not a
/// guess at `web.foo.rs.…`.
fn vertical_of(path: &str) -> Result<&str, String> {
    let normalized = path.replace('\\', "/");
    let rest = normalized
        .split_once(&format!("{WEB_SRC}/"))
        .map(|(_, rest)| rest)
        .ok_or_else(|| format!("{path}: not under {WEB_SRC}/"))?;
    // Borrow from `path` rather than the temporary: find the same offset.
    let offset = path.len() - rest.len();
    let rest = &path[offset..];
    match rest.split_once('/') {
        Some((vertical, _)) => Ok(vertical),
        None => Err(format!(
            "{path}: a #[server] fn directly under {WEB_SRC} has no vertical directory, so its \
             span name cannot be derived — move it into {WEB_SRC}/<vertical>/"
        )),
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
        let mut any = Vec::new();
        for tt in entry.clone() {
            if !seen_eq {
                if let TokenTree::Punct(ref p) = tt {
                    if p.as_char() == '=' {
                        seen_eq = true;
                        continue;
                    }
                }
                any.push(tt);
            } else {
                value.extend(std::iter::once(tt));
            }
        }
        if seen_eq {
            collect_idents(&value, &mut out);
        } else {
            // A shorthand field (`fields(post_id)`) records the argument itself,
            // so its identifiers are values, not a bare name.
            let bare: TokenStream = any.into_iter().collect();
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

/// Every problem with one `#[server]` fn, as human-readable lines (without the
/// file prefix, which the caller adds).
fn problems_with(f: &WebServerFn, vertical: &str) -> Vec<String> {
    let at = format!("line {}: `{}`", f.line, f.ident);

    // A conditionally-present span is exactly the inconsistency this gate exists to
    // remove, so check it before "missing" — otherwise the message would mislead.
    if f.attrs.iter().any(is_cfg_attr_instrument) {
        return vec![format!(
            "{at}: #[tracing::instrument] is wrapped in a #[cfg_attr(...)] — a span that exists \
             only under some cfg is the inconsistency this gate prevents; apply it unconditionally"
        )];
    }

    let Some(index) = f.attrs.iter().position(is_instrument) else {
        return vec![format!(
            "{at}: #[server] fn has no #[tracing::instrument] — add \
             #[tracing::instrument(name = \"web.{vertical}.{}\")] directly after #[server] (#511)",
            f.ident
        )];
    };

    if index < f.server_attr_index {
        return vec![format!(
            "{at}: #[tracing::instrument] must come *after* #[server], not before"
        )];
    }

    let parsed = match parse_instrument(&f.attrs[index]) {
        Ok(p) => p,
        Err(e) => return vec![format!("{at}: {e}")],
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

    for (param, ty) in &f.params {
        let reduced = reduce_type(ty);
        let recordable = is_recordable(reduced.as_deref());
        let skipped = parsed.skip_all || parsed.skipped.contains(param);
        let shown = reduced.as_deref().unwrap_or("<unrecognized type>");
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
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    if let Err(e) = web_server_fns::rust_files(Path::new(WEB_SRC), &mut files) {
        result.push(
            StepResult::fail("server-fn-tracing").detail(format!("cannot scan {WEB_SRC}: {e}")),
        );
        return;
    }
    // A file we listed but cannot READ is surfaced as a failure, not dropped: an
    // unenumerated source could hide a bare `#[server]` fn (a false pass).
    let mut sources = Vec::new();
    let mut read_errors = Vec::new();
    for p in &files {
        match std::fs::read_to_string(p) {
            Ok(s) => sources.push((p.display().to_string(), s)),
            Err(e) => read_errors.push(format!("{}: cannot read: {e}", p.display())),
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

    // --- fail-loud enumeration (AC 12) ---

    #[test]
    fn an_unparseable_file_is_reported_not_skipped() {
        let s = src("posts", "fn broken( {{{ not valid");
        assert!(problems(&s)
            .expect("a parse failure is reported")
            .contains("web/src/posts/api.rs"));
    }
}
