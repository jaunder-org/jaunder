//! Shared enumeration of the `web` crate's `#[server]` functions.
//!
//! Two static checks need the same list — [`server_fn_registrar_check`] (#426,
//! ADR-0066: every `#[server]` fn must appear in the test registrar) and
//! [`server_fn_tracing_check`] (#511: every one must carry a PII-safe
//! `#[tracing::instrument]` span). Enumerating twice would let the two drift, so
//! the walk lives here and each gate applies its own rule to the result.
//!
//! [`server_fn_registrar_check`]: crate::steps::server_fn_registrar_check
//! [`server_fn_tracing_check`]: crate::steps::server_fn_tracing_check
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

impl<'ast> syn::visit::Visit<'ast> for Visitor {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if let Some(index) = f.attrs.iter().position(|a| a.path().is_ident("server")) {
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
}
