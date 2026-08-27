//! Shared enumeration of the `web` crate's `#[macros::server]` functions.
//!
//! Two static checks need the same list — [`server_fn_tracing_check`] (#511:
//! every one's arguments must be PII-safe) and the flow-coverage inventory
//! [`crate::server_fns`] (#681). Enumerating twice would let them drift, so the
//! walk lives here and each gate applies its own rule to the result.
//!
//! [`server_fn_tracing_check`]: crate::steps::server_fn_tracing_check
//!
//! The enumeration is deliberately **dumb**: it reports what it found and
//! interprets nothing. Whether a `#[macros::server(...)]` form is supported, which
//! argument types may be recorded — all of that is the consuming gate's judgment, so
//! this module stays policy-free and the gates can evolve independently.
//!
//! It is also **fail-loud**: a `syn` parse failure is returned as `Err`, never
//! swallowed. A file we cannot enumerate could hide a server fn from *every*
//! gate, which is the one failure mode none of them can detect for itself.
//!
//! Alongside the walk it holds [`vertical_of`], which each gate keys something on —
//! the derived span name and the derived endpoint — so a change to what "vertical"
//! means cannot land in one gate and not the other.

use std::path::{Path, PathBuf};

use syn::spanned::Spanned;

/// The `web` crate source root, scanned recursively by every gate.
pub const WEB_SRC: &str = "web/src";

/// One parameter of a server fn.
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

/// One `#[macros::server]` fn found in a `web` source file.
#[derive(Debug, Clone)]
pub struct WebServerFn {
    /// 1-based line of the `#[macros::server]` attribute.
    pub line: usize,
    /// The fn identifier, verbatim (`list_my_media`).
    pub ident: String,
    /// Parameters in declaration order.
    pub params: Vec<Param>,
    /// Every attribute on the fn, in source order — so a consumer can judge
    /// attribute *ordering*, not just presence.
    pub attrs: Vec<syn::Attribute>,
    /// Index into [`attrs`](Self::attrs) of the `#[macros::server]` attribute.
    pub server_attr_index: usize,
}

/// Every `#[macros::server]` fn in one source file, or a message describing why the
/// file could not be enumerated.
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

/// Whether `attr` is jaunder's `#[macros::server]` — the one spelling a `web`
/// server fn may use (#714).
///
/// Matched on the **whole path**, not its last segment: leptos's bare `#[server]`
/// is deliberately *not* enumerated, because a fn wearing it declares its own
/// endpoint and span and is exactly what this migration retired. Nothing in
/// `web/src` may reintroduce it. The runtime wire contract in
/// `server/tests/web/server_fn_wire.rs` keeps a generated-type backstop, while
/// the registrar gate's real-tree count assertion keeps an empty inventory from
/// looking like success.
fn is_server_attr(attr: &syn::Attribute) -> bool {
    let segments = &attr.path().segments;
    segments.len() == 2 && segments[0].ident == "macros" && segments[1].ident == "server"
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
    /// Reported rather than dropped: an unenumerated source could hide a server fn
    /// from a gate, which is a false pass. Every gate surfaces these alongside its
    /// own findings.
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
/// could hide a server fn, so the error propagates to
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

/// The arguments of a `#[macros::server(...)]` attribute, as `Meta` items.
///
/// `Ok(None)` for the bare `#[macros::server]` (there is no argument list);
/// `Ok(Some(args))` for `#[macros::server(input = Json, skip_all)]`; `Err` for the
/// `#[macros::server = …]` form or an argument list `syn` cannot parse as `Meta`.
///
/// Shared because both the registrar gate (which needs to know whether any argument
/// is a positional type rename) and the tracing gate (which reads `skip(...)` /
/// `skip_all`) must agree on how this attribute parses and on what an unparseable
/// one says — the same drift argument that put the enumeration here. What each gate
/// concludes from the arguments stays its own.
///
/// # Errors
///
/// Returns `Err` for a malformed or unexpected attribute form, never for an
/// argument this module does not recognise.
pub fn server_attr_args(attr: &syn::Attribute) -> Result<Option<Vec<syn::Meta>>, String> {
    match &attr.meta {
        syn::Meta::Path(_) => Ok(None),
        syn::Meta::NameValue(_) => Err("unexpected `#[macros::server = ...]` form".to_string()),
        syn::Meta::List(_) => {
            let args = attr
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .map_err(|e| format!("cannot parse #[macros::server(...)] arguments: {e}"))?;
            Ok(Some(args.into_iter().collect()))
        }
    }
}

/// The vertical a source file belongs to: the first path segment under `web/src`.
///
/// Every server-fn consumer keys something on it — the registrar's match key
/// (`(vertical, leaf)`), the span name (`web.<vertical>.<fn>`) and endpoint
/// (`/<vertical>/<fn>`) the macro derives, and coverage's `qualified()` — so it
/// lives here rather than in any one of them.
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
            "{path}: a #[macros::server] fn directly under {WEB_SRC} has no vertical directory — \
             move it into {WEB_SRC}/<vertical>/"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::retired_server_fn;

    #[test]
    fn captures_ident_line_and_params() {
        let src = "#[macros::server]\n\
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
                   #[cfg(feature = \"server\")]\n\
                   #[macros::server(skip_all)]\n\
                   pub async fn x() -> R {}\n";
        let fns = server_fns_in(src).unwrap();
        // doc comment + cfg + macros::server
        assert_eq!(fns[0].attrs.len(), 3);
        let located = fns[0].attrs[fns[0].server_attr_index].path();
        assert_eq!(located.segments.len(), 2);
        assert!(located.segments.last().unwrap().ident == "server");
        assert_eq!(fns[0].line, 3);
    }

    #[test]
    fn zero_arg_fn_has_no_params() {
        let src = "#[macros::server]\npub async fn logout() -> R {}\n";
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
        // Judging it is the consuming gate's job: the tracing gate must refuse it.
        // Reporting rather than erroring here keeps one gate's rule from silently
        // widening another's.
        let src = "#[macros::server]\npub async fn x((a, b): (u32, u32)) -> R {}\n";
        let fns = server_fns_in(src).unwrap();
        assert_eq!(fns[0].params.len(), 1);
        assert!(fns[0].params[0].name.is_none());
    }

    #[test]
    fn only_the_macro_spelling_enumerates() {
        // #714 retired leptos's bare `#[server]` from `web/src`: a fn wearing it
        // declares its own endpoint and span, which is what the macro exists to
        // derive. Enumerating it would resurrect the gate branches that tolerated
        // both spellings; the registrar gate's real-tree count assertion and the
        // runtime wire contract stop this narrowing from becoming a silent green.
        let src = format!(
            "{}#[macros::server(skip(name))]\npub async fn create(name: AudienceName) -> R {{}}\n",
            retired_server_fn(
                "(endpoint = \"/audiences/rename\")",
                "pub async fn rename(name: AudienceName) -> R {}"
            )
        );
        let fns = server_fns_in(&src).unwrap();
        let idents: Vec<&str> = fns.iter().map(|f| f.ident.as_str()).collect();
        assert_eq!(idents, vec!["create"]);
    }

    #[test]
    fn a_differently_qualified_server_attribute_does_not_enumerate() {
        // The predicate matches the whole `macros::server` path, so neither the bare
        // retired spelling nor some other crate's `#[foo::server]` counts.
        assert!(
            server_fns_in("#[foo::server]\npub async fn x() -> R {}\n")
                .unwrap()
                .is_empty()
        );
        let bare = retired_server_fn("", "pub async fn x() -> R {}");
        assert!(server_fns_in(&bare).unwrap().is_empty());
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
}
