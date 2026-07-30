//! Shared enumeration of the `web` crate's `#[server]` functions.
//!
//! Three static checks need the same list — [`server_fn_registrar_check`] (#426,
//! ADR-0066: every `#[server]` fn must appear in the test registrar),
//! [`server_fn_tracing_check`] (#511: every one must carry a PII-safe
//! `#[tracing::instrument]` span), and [`server_fn_endpoint_check`] (#684: every
//! one's `endpoint` must be `/<vertical>/<ident>`). Enumerating three times would
//! let them drift, so the walk lives here and each gate applies its own rule to
//! the result.
//!
//! [`server_fn_registrar_check`]: crate::steps::server_fn_registrar_check
//! [`server_fn_tracing_check`]: crate::steps::server_fn_tracing_check
//! [`server_fn_endpoint_check`]: crate::steps::server_fn_endpoint_check
//!
//! The enumeration is deliberately **dumb**: it reports what it found and
//! interprets nothing. Whether a `#[server(...)]` form is supported, what a span
//! name should be, which argument types may be recorded — all of that is the
//! consuming gate's judgment, so this module stays policy-free and both gates can
//! evolve independently.
//!
//! It is also **fail-loud**: a `syn` parse failure is returned as `Err`, never
//! swallowed. A file we cannot enumerate could hide a `#[server]` fn from *both*
//! gates, which is the one failure mode neither can detect for itself.
//!
//! Alongside the walk it holds the pieces more than one gate needs to *act* on the
//! result: [`vertical_of`] (each gate keys something on the vertical) and the
//! attribute-literal rewrite primitives [`rewrite_attr_arg`] / [`apply_fixes`],
//! which let a gate write a derived literal under `Mode::Fix`. These stay
//! policy-free too — they are told which attribute, which argument, and what value;
//! deciding those is the gate's job.

use std::path::{Path, PathBuf};

use syn::spanned::Spanned;

/// The `web` crate source root, scanned recursively by both gates.
pub const WEB_SRC: &str = "web/src";

/// One parameter of a `#[server]` fn.
///
/// The type is carried **parsed** rather than rendered to a string: consumers match
/// on its structure (unwrap `Option<…>`, take the last path segment), which is
/// exact, whereas a rendered string would need whitespace normalization and would
/// mangle a lifetime (`&'a str` → `&'astr`).
#[derive(Debug, Clone)]
pub struct Param {
    /// The binding name, or `None` when the pattern is not a plain identifier (a
    /// destructured tuple, say).
    ///
    /// Reported rather than rejected: whether a nameless parameter is a problem is
    /// the consuming gate's call. It means nothing to the registrar guard, which
    /// only maps fn idents to type names, but the tracing gate must refuse it —
    /// there is no single name to put in `skip(...)`.
    pub name: Option<String>,
    /// The parameter's type, as written.
    pub ty: syn::Type,
}

/// One `#[server]` fn found in a `web` source file.
#[derive(Debug, Clone)]
pub struct WebServerFn {
    /// 1-based line of the `#[server]` attribute.
    pub line: usize,
    /// The fn identifier, verbatim (`list_my_media`).
    pub ident: String,
    /// Parameters in declaration order.
    pub params: Vec<Param>,
    /// Every attribute on the fn, in source order — so a consumer can judge
    /// attribute *ordering*, not just presence.
    pub attrs: Vec<syn::Attribute>,
    /// Index into [`attrs`](Self::attrs) of the `#[server]` attribute.
    pub server_attr_index: usize,
    /// Whether the attribute is `#[macros::server]` rather than leptos's bare
    /// `#[server]` (#714).
    ///
    /// The two carry different information, so every consumer has to branch on it:
    /// the macro form *derives* the endpoint and span name instead of declaring
    /// them, and it carries the `skip`/`skip_all` arguments that used to sit on
    /// `#[tracing::instrument]`. Reported rather than judged, like everything else
    /// here — this module says which spelling it found; the gates decide what that
    /// means for them.
    ///
    /// Transitional: once every fn is converted, this is always `true` and the
    /// field goes.
    pub uses_macro_attr: bool,
}

/// Every `#[server]` fn in one source file, or a message describing why the file
/// could not be enumerated.
///
/// # Errors
///
/// Returns `Err` only when `src` is not parseable Rust. Everything else this
/// module finds is *reported*, not judged — a stricter rule belongs to whichever
/// gate needs it, so adding one here cannot silently widen the other gate.
pub fn server_fns_in(src: &str) -> Result<Vec<WebServerFn>, String> {
    let file = syn::parse_file(src).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut visitor = Visitor { fns: Vec::new() };
    syn::visit::visit_file(&mut visitor, &file);
    Ok(visitor.fns)
}

struct Visitor {
    fns: Vec<WebServerFn>,
}

/// Whether `attr` declares a server fn, in either spelling.
///
/// Matched on the path's **last segment**, so both leptos's `#[server]` and
/// jaunder's `#[macros::server]` (#714) are found. An `is_ident("server")` check
/// would be false for the two-segment path — and because every gate here returns
/// "no problems" on an empty enumeration, missing a spelling is a **silent green**
/// across the registrar, tracing, and coverage gates at once, not a loud failure.
fn is_server_attr(attr: &syn::Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|s| s.ident == "server")
}

impl<'ast> syn::visit::Visit<'ast> for Visitor {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if let Some(index) = f.attrs.iter().position(is_server_attr) {
            self.fns.push(WebServerFn {
                line: f.attrs[index].span().start().line,
                ident: f.sig.ident.to_string(),
                params: params_of(f),
                attrs: f.attrs.clone(),
                server_attr_index: index,
                uses_macro_attr: f.attrs[index].path().segments.len() > 1,
            });
        }
        syn::visit::visit_item_fn(self, f);
    }
}

/// Every parameter of a free fn, in declaration order.
fn params_of(f: &syn::ItemFn) -> Vec<Param> {
    f.sig
        .inputs
        .iter()
        .filter_map(|arg| {
            // A free fn has no receiver; a `self` parameter cannot occur here.
            let syn::FnArg::Typed(typed) = arg else {
                return None;
            };
            let name = match typed.pat.as_ref() {
                syn::Pat::Ident(pat) => Some(pat.ident.to_string()),
                _ => None,
            };
            Some(Param {
                name,
                ty: typed.ty.as_ref().clone(),
            })
        })
        .collect()
}

/// The `web/src` tree as a gate sees it.
pub struct WebSources {
    /// Each readable Rust file, as (path, contents).
    pub sources: Vec<(String, String)>,
    /// One message per file that was listed but could not be *read*.
    ///
    /// Reported rather than dropped: an unenumerated source could hide a
    /// `#[server]` fn from a gate, which is a false pass. Both gates surface these
    /// alongside their own findings.
    pub read_errors: Vec<String>,
}

/// Read every Rust file under `root`.
///
/// # Errors
///
/// Returns `Err` if `root` cannot be scanned at all — a moved or renamed tree must
/// fail loudly rather than quietly yield nothing to check.
pub fn read_web_sources(root: &Path) -> Result<WebSources, String> {
    let mut files = Vec::new();
    rust_files(root, &mut files).map_err(|e| format!("cannot scan {}: {e}", root.display()))?;
    let mut sources = Vec::new();
    let mut read_errors = Vec::new();
    for path in &files {
        match std::fs::read_to_string(path) {
            Ok(s) => sources.push((path.display().to_string(), s)),
            Err(e) => read_errors.push(format!("{}: cannot read: {e}", path.display())),
        }
    }
    Ok(WebSources {
        sources,
        read_errors,
    })
}

/// Collect every `.rs` file under `dir`, recursively. A directory we cannot list
/// could hide a `#[server]` fn, so the error propagates to
/// [`read_web_sources`] rather than yielding a short list.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// The arguments of a `#[server(...)]` attribute, as `Meta` items.
///
/// `Ok(None)` for the bare `#[server]` (there is no argument list); `Ok(Some(args))`
/// for `#[server(endpoint = "…", input = Json)]`; `Err` for the `#[server = …]` form
/// or an argument list `syn` cannot parse as `Meta`.
///
/// Shared because both the registrar gate (which needs to know whether any argument
/// is a positional type rename) and the endpoint gate (which reads
/// `endpoint = "…"`) must agree on how this attribute parses and on what an
/// unparseable one says — the same drift argument that put the enumeration here.
/// What each gate concludes from the arguments stays its own.
///
/// # Errors
///
/// Returns `Err` for a malformed or unexpected attribute form, never for an
/// argument this module does not recognise.
pub fn server_attr_args(attr: &syn::Attribute) -> Result<Option<Vec<syn::Meta>>, String> {
    match &attr.meta {
        syn::Meta::Path(_) => Ok(None),
        syn::Meta::NameValue(_) => Err("unexpected `#[server = ...]` form".to_string()),
        syn::Meta::List(_) => {
            let args = attr
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .map_err(|e| format!("cannot parse #[server(...)] arguments: {e}"))?;
            Ok(Some(args.into_iter().collect()))
        }
    }
}

/// The vertical a source file belongs to: the first path segment under `web/src`.
///
/// All three server-fn gates key something on it — the registrar's match key
/// (`(vertical, leaf)`), the span name (`web.<vertical>.<fn>`), and the endpoint
/// (`/<vertical>/<fn>`) — so it lives here rather than in any one of them.
///
/// # Errors
///
/// Returns `Err` when `path` is not under `web/src/`, or sits directly under it
/// with no vertical directory. The latter is an error naming the file rather than
/// a guess, because there is no honest vertical to derive.
pub fn vertical_of(path: &str) -> Result<&str, String> {
    let (_, rest) = path
        .split_once(&format!("{WEB_SRC}/"))
        .ok_or_else(|| format!("{path}: not under {WEB_SRC}/"))?;
    match rest.split_once('/') {
        Some((vertical, _)) => Ok(vertical),
        None => Err(format!(
            "{path}: a #[server] fn directly under {WEB_SRC} has no vertical directory — \
             move it into {WEB_SRC}/<vertical>/"
        )),
    }
}

/// One file's pending rewrites, as (1-based line range of the attribute,
/// replacement text). Applied bottom-up so earlier ranges stay valid.
pub type LineFix = (usize, usize, String);

/// Rewrite one attribute's source so its `key = "…"` argument is `desired`, or
/// `None` when it already is (or is absent and `insert_if_absent` is false).
///
/// Textual rather than a token-stream re-emit: the attribute has already been
/// parsed and validated by the calling gate, and a re-emit would reformat the
/// author's own `skip(...)`/`fields(...)` spelling. Three shapes occur:
///
/// - `#[attr]`                   → `#[attr(key = "…")]`        (insert)
/// - `#[attr(other)]`            → `#[attr(key = "…", other)]` (insert)
/// - `#[attr(key = "x", other)]` → the literal is replaced in place
///
/// `attr_name` is the attribute's own identifier (`instrument`, `server`); the
/// insert path needs it to locate where the argument list begins, and hardcoding
/// it here would give a key-general signature a hidden attribute-specific
/// assumption. `insert_if_absent` separates the two callers: the tracing gate
/// synthesizes a missing `name`, while the endpoint gate treats a missing
/// `endpoint` as a hard error for the author to resolve, never something to invent
/// behind their back.
pub fn rewrite_attr_arg(
    attr_src: &str,
    attr_name: &str,
    key: &str,
    desired: &str,
    insert_if_absent: bool,
) -> Option<String> {
    let quoted = format!("\"{desired}\"");
    // An existing `key = "…"`: replace just the literal, leaving everything else.
    if let Some(eq) = find_attr_arg_eq(attr_src, key) {
        let rest = &attr_src[eq..];
        let open = rest.find('"')?;
        let close = rest[open + 1..].find('"')? + open + 1;
        if rest[open..=close] == quoted {
            return None;
        }
        let mut out = String::with_capacity(attr_src.len() + desired.len());
        out.push_str(&attr_src[..eq + open]);
        out.push_str(&quoted);
        out.push_str(&rest[close + 1..]);
        return Some(out);
    }
    if !insert_if_absent {
        return None;
    }
    let after_name = attr_src.find(attr_name)? + attr_name.len();
    let tail = &attr_src[after_name..];
    if let Some(paren) = tail.find('(') {
        // `…attr(args)]` → `…attr(key = "…", args)]`
        let at = after_name + paren + 1;
        let sep = if tail[paren + 1..].trim_start().starts_with(')') {
            String::new()
        } else {
            ", ".to_string()
        };
        return Some(format!(
            "{}{key} = {quoted}{sep}{}",
            &attr_src[..at],
            &attr_src[at..]
        ));
    }
    // Bare `#[…attr]` → `#[…attr(key = "…")]`
    let close = attr_src.rfind(']')?;
    Some(format!(
        "{}({key} = {quoted}){}",
        &attr_src[..close],
        &attr_src[close..]
    ))
}

/// Byte offset just past the `=` of an existing `key =` argument, or `None`.
///
/// Matched on the identifier boundary so a `…_key` argument cannot be mistaken for
/// it. The `fields(` guard is deliberately **not** parameterized: it exists because
/// `#[tracing::instrument(fields(name = %user))]` would otherwise read as a
/// top-level `name =`, and neither attribute this serves today (`instrument`,
/// `server`) has any other nested argument list. A future attribute that did would
/// need it generalized.
fn find_attr_arg_eq(attr_src: &str, key: &str) -> Option<usize> {
    let bytes = attr_src.as_bytes();
    let mut from = 0;
    while let Some(rel) = attr_src[from..].find(key) {
        let start = from + rel;
        let end = start + key.len();
        let before_ok = start
            .checked_sub(1)
            .is_none_or(|i| !bytes[i].is_ascii_alphanumeric() && bytes[i] != b'_');
        let after = attr_src[end..].trim_start();
        let is_field_arg = attr_src[..start].contains("fields(");
        if before_ok && after.starts_with('=') && !is_field_arg {
            let eq = end + (attr_src[end..].len() - after.len()) + 1;
            return Some(eq);
        }
        from = end;
    }
    None
}

/// Apply line-range replacements bottom-up and return the new file text.
pub fn apply_fixes(src: &str, mut fixes: Vec<LineFix>) -> String {
    let mut lines: Vec<String> = src.lines().map(ToString::to_string).collect();
    fixes.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, replacement) in fixes {
        let new: Vec<String> = replacement.lines().map(ToString::to_string).collect();
        lines.splice(start - 1..end, new);
    }
    let mut out = lines.join("\n");
    if src.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_ident_line_and_params() {
        let src = "#[server(endpoint = \"/x\")]\n\
                   pub async fn delete_media(sha256: ContentHash, force: Option<bool>) -> R {}\n";
        let fns = server_fns_in(src).unwrap();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].ident, "delete_media");
        assert_eq!(fns[0].line, 1);
        let names: Vec<Option<&str>> = fns[0].params.iter().map(|p| p.name.as_deref()).collect();
        assert_eq!(names, vec![Some("sha256"), Some("force")]);
        // Types are carried parsed; consumers reduce them structurally.
        assert!(matches!(fns[0].params[0].ty, syn::Type::Path(_)));
        assert!(matches!(fns[0].params[1].ty, syn::Type::Path(_)));
    }

    #[test]
    fn captures_every_attribute_and_locates_the_server_one() {
        let src = "/// doc\n\
                   #[server(endpoint = \"/x\")]\n\
                   #[tracing::instrument(name = \"web.a.x\")]\n\
                   pub async fn x() -> R {}\n";
        let fns = server_fns_in(src).unwrap();
        // doc comment + server + instrument
        assert_eq!(fns[0].attrs.len(), 3);
        assert!(fns[0].attrs[fns[0].server_attr_index]
            .path()
            .is_ident("server"));
        assert_eq!(fns[0].line, 2);
    }

    #[test]
    fn zero_arg_fn_has_no_params() {
        let src = "#[server]\npub async fn logout() -> R {}\n";
        assert!(server_fns_in(src).unwrap()[0].params.is_empty());
    }

    #[test]
    fn ignores_non_server_fns() {
        let src = "pub async fn plain() {}\n#[tokio::test]\nasync fn t() {}\n";
        assert!(server_fns_in(src).unwrap().is_empty());
    }

    #[test]
    fn syn_parse_failure_is_an_error() {
        let err = server_fns_in("fn broken( {{{ not valid").unwrap_err();
        // The registrar gate's own test asserts only that this is an error, but the
        // message text is part of that gate's observable output, so pin it here.
        assert!(err.starts_with("cannot parse as Rust: "), "{err}");
    }

    #[test]
    fn a_non_ident_parameter_pattern_is_reported_as_nameless_not_rejected() {
        // Judging it is the consuming gate's job: the registrar guard does not care,
        // the tracing gate must refuse it. Reporting rather than erroring here keeps
        // one gate's rule from silently widening the other's.
        let src = "#[server]\npub async fn x((a, b): (u32, u32)) -> R {}\n";
        let fns = server_fns_in(src).unwrap();
        assert_eq!(fns[0].params.len(), 1);
        assert!(fns[0].params[0].name.is_none());
    }

    #[test]
    fn both_spellings_enumerate_and_only_the_macro_one_says_so() {
        // Matching on the path's last segment is what finds `#[macros::server]`;
        // an `is_ident("server")` check would miss it, and because every consuming
        // gate reports "no problems" on an empty enumeration that miss is a silent
        // green rather than a failure.
        let src = "#[server(endpoint = \"/audiences/rename\")]\n\
                   pub async fn rename(name: AudienceName) -> R {}\n\
                   #[macros::server(skip(name))]\n\
                   pub async fn create(name: AudienceName) -> R {}\n";
        let fns = server_fns_in(src).unwrap();
        assert_eq!(fns.len(), 2);
        let seen: Vec<(&str, bool)> = fns
            .iter()
            .map(|f| (f.ident.as_str(), f.uses_macro_attr))
            .collect();
        assert_eq!(seen, vec![("rename", false), ("create", true)]);
        // The located attribute is the macro one, not some other attribute.
        assert_eq!(
            fns[1].attrs[fns[1].server_attr_index].path().segments.len(),
            2
        );
    }

    #[test]
    fn a_bare_macro_server_attr_is_still_the_macro_spelling() {
        let fns = server_fns_in("#[macros::server]\npub async fn x() -> R {}\n").unwrap();
        assert!(fns[0].uses_macro_attr);
    }

    // --- vertical_of ---

    #[test]
    fn vertical_of_takes_the_first_segment_under_web_src() {
        assert_eq!(
            vertical_of("web/src/posts/api/listing.rs").unwrap(),
            "posts"
        );
        assert_eq!(vertical_of("web/src/tags/api.rs").unwrap(), "tags");
    }

    #[test]
    fn vertical_of_rejects_a_file_directly_under_web_src() {
        let err = vertical_of("web/src/loose.rs").unwrap_err();
        assert!(err.contains("web/src/loose.rs"), "{err}");
        assert!(err.contains("vertical"), "{err}");
    }

    #[test]
    fn vertical_of_rejects_a_path_outside_web_src() {
        assert!(vertical_of("server/src/lib.rs").is_err());
    }

    // --- rewrite_attr_arg: the `instrument`/`name` cases the tracing gate relies on ---

    #[test]
    fn rewrite_adds_a_name_to_a_bare_instrument() {
        assert_eq!(
            rewrite_attr_arg(
                "#[tracing::instrument]",
                "instrument",
                "name",
                "web.posts.x",
                true
            )
            .unwrap(),
            "#[tracing::instrument(name = \"web.posts.x\")]"
        );
    }

    #[test]
    fn rewrite_inserts_the_name_before_existing_arguments() {
        assert_eq!(
            rewrite_attr_arg(
                "#[tracing::instrument(skip_all)]",
                "instrument",
                "name",
                "web.posts.create_post",
                true
            )
            .unwrap(),
            "#[tracing::instrument(name = \"web.posts.create_post\", skip_all)]"
        );
    }

    #[test]
    fn rewrite_replaces_a_wrong_name_leaving_other_arguments_alone() {
        assert_eq!(
            rewrite_attr_arg(
                "#[tracing::instrument(name = \"stale\", skip(prefix))]",
                "instrument",
                "name",
                "web.tags.list_tags",
                true
            )
            .unwrap(),
            "#[tracing::instrument(name = \"web.tags.list_tags\", skip(prefix))]"
        );
    }

    #[test]
    fn rewrite_is_a_no_op_when_the_name_is_already_right() {
        assert_eq!(
            rewrite_attr_arg(
                "#[tracing::instrument(name = \"web.tags.list_tags\", skip(prefix))]",
                "instrument",
                "name",
                "web.tags.list_tags",
                true
            ),
            None
        );
    }

    #[test]
    fn rewrite_does_not_mistake_a_field_called_name_for_the_span_name() {
        // `fields(name = %username)` must not be rewritten into the span name —
        // that would silently change what the span records.
        let src = "#[tracing::instrument(skip_all, fields(name = %username))]";
        assert_eq!(
            rewrite_attr_arg(src, "instrument", "name", "web.profile.get_profile", true).unwrap(),
            "#[tracing::instrument(name = \"web.profile.get_profile\", skip_all, \
             fields(name = %username))]"
        );
    }

    #[test]
    fn rewrite_spans_a_multi_line_attribute() {
        let src = "#[tracing::instrument(\n    name = \"stale\",\n    skip(a, b)\n)]";
        let got = rewrite_attr_arg(
            src,
            "instrument",
            "name",
            "web.backup.update_backup_settings",
            true,
        )
        .unwrap();
        assert!(got.contains("web.backup.update_backup_settings"), "{got}");
        assert!(got.contains("skip(a, b)"), "{got}");
        assert!(!got.contains("stale"), "{got}");
    }

    // --- rewrite_attr_arg: the generalization the endpoint gate (#684) needs ---

    #[test]
    fn rewrite_attr_arg_replaces_an_arbitrary_key_on_an_arbitrary_attribute() {
        let attr = "#[server(endpoint = \"/create_post\", input = Json)]";
        assert_eq!(
            rewrite_attr_arg(attr, "server", "endpoint", "/posts/create", false).unwrap(),
            "#[server(endpoint = \"/posts/create\", input = Json)]",
            "other arguments must survive verbatim"
        );
    }

    #[test]
    fn rewrite_attr_arg_leaves_a_missing_key_alone_when_not_inserting() {
        // The endpoint gate treats a missing `endpoint` as a hard error for the
        // author, not something to synthesize behind their back.
        assert_eq!(
            rewrite_attr_arg("#[server]", "server", "endpoint", "/tags/list", false),
            None
        );
        assert_eq!(
            rewrite_attr_arg(
                "#[server(input = Json)]",
                "server",
                "endpoint",
                "/t/l",
                false
            ),
            None
        );
    }

    #[test]
    fn rewrite_attr_arg_inserts_into_the_named_attribute_not_a_hardcoded_one() {
        // Pins the `attr_name` parameter: the insert path must not assume
        // "instrument", which is what the pre-#684 implementation hardcoded.
        assert_eq!(
            rewrite_attr_arg(
                "#[server(input = Json)]",
                "server",
                "endpoint",
                "/tags/list",
                true
            )
            .unwrap(),
            "#[server(endpoint = \"/tags/list\", input = Json)]"
        );
        assert_eq!(
            rewrite_attr_arg("#[server]", "server", "endpoint", "/tags/list", true).unwrap(),
            "#[server(endpoint = \"/tags/list\")]"
        );
    }

    // --- apply_fixes ---

    #[test]
    fn apply_fixes_replaces_line_ranges_bottom_up() {
        // Two fixes at different offsets: applying the later one first is what
        // keeps the earlier range valid.
        let src = "one\ntwo\nthree\nfour\n";
        let out = apply_fixes(
            src,
            vec![(1, 1, "ONE".to_string()), (3, 4, "THREE-FOUR".to_string())],
        );
        assert_eq!(out, "ONE\ntwo\nTHREE-FOUR\n");
    }

    #[test]
    fn apply_fixes_expands_a_single_line_into_several() {
        let src = "a\nb\n";
        let out = apply_fixes(src, vec![(2, 2, "b1\nb2".to_string())]);
        assert_eq!(out, "a\nb1\nb2\n");
    }

    #[test]
    fn apply_fixes_preserves_a_missing_trailing_newline() {
        assert_eq!(apply_fixes("a\nb", vec![(1, 1, "A".to_string())]), "A\nb");
    }
}
