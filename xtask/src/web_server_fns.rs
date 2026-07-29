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

/// One `#[server]` fn found in a `web` source file.
///
/// Parameter types are carried **parsed** rather than rendered to a string:
/// consumers match on their structure (unwrap `Option<…>`, take the last path
/// segment), which is exact, whereas a rendered string would need whitespace
/// normalization and would mangle a lifetime (`&'a str` → `&'astr`).
#[derive(Debug, Clone)]
pub struct WebServerFn {
    /// 1-based line of the `#[server]` attribute.
    pub line: usize,
    /// The fn identifier, verbatim (`list_my_media`).
    pub ident: String,
    /// Parameters in declaration order: (name, parsed type).
    pub params: Vec<(String, syn::Type)>,
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
/// Returns `Err` when `src` is not parseable Rust, or when a `#[server]` fn takes
/// a parameter whose pattern is not a plain identifier — such a parameter has no
/// single name a gate could skip or record, and dropping it silently would hide it
/// from the tracing gate's allowlist.
pub fn server_fns_in(src: &str) -> Result<Vec<WebServerFn>, String> {
    let file = syn::parse_file(src).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut visitor = Visitor {
        fns: Vec::new(),
        errors: Vec::new(),
    };
    syn::visit::visit_file(&mut visitor, &file);
    if let Some(err) = visitor.errors.first() {
        return Err(err.clone());
    }
    Ok(visitor.fns)
}

struct Visitor {
    fns: Vec<WebServerFn>,
    errors: Vec<String>,
}

impl<'ast> syn::visit::Visit<'ast> for Visitor {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if let Some(index) = f.attrs.iter().position(|a| a.path().is_ident("server")) {
            match params_of(f) {
                Ok(params) => self.fns.push(WebServerFn {
                    line: f.attrs[index].span().start().line,
                    ident: f.sig.ident.to_string(),
                    params,
                    attrs: f.attrs.clone(),
                    server_attr_index: index,
                }),
                Err(e) => self.errors.push(e),
            }
        }
        syn::visit::visit_item_fn(self, f);
    }
}

/// The (name, type) of every parameter, or an error naming the first parameter
/// whose pattern is not a plain identifier.
fn params_of(f: &syn::ItemFn) -> Result<Vec<(String, syn::Type)>, String> {
    let mut params = Vec::new();
    for arg in &f.sig.inputs {
        // A free fn has no receiver; a `self` parameter cannot occur here.
        let syn::FnArg::Typed(typed) = arg else {
            continue;
        };
        let syn::Pat::Ident(pat) = typed.pat.as_ref() else {
            return Err(format!(
                "line {}: `{}` takes a parameter whose pattern is not a plain identifier — \
                 it has no single name to skip or record; bind it to one",
                typed.span().start().line,
                f.sig.ident
            ));
        };
        params.push((pat.ident.to_string(), typed.ty.as_ref().clone()));
    }
    Ok(params)
}

/// Collect every `.rs` file under `dir`, recursively.
///
/// # Errors
///
/// Returns the underlying [`std::io::Error`] if `dir` (or a subdirectory) cannot
/// be read — a directory we cannot list could hide a `#[server]` fn, so callers
/// surface this as a gate failure rather than an empty result.
pub fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
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
        let names: Vec<&str> = fns[0].params.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["sha256", "force"]);
        // Types are carried parsed; consumers reduce them structurally.
        assert!(matches!(fns[0].params[0].1, syn::Type::Path(_)));
        assert!(matches!(fns[0].params[1].1, syn::Type::Path(_)));
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
    fn a_non_ident_parameter_pattern_is_an_error() {
        // No single name to skip or record — dropping it would hide the argument
        // from the tracing gate's allowlist.
        let src = "#[server]\npub async fn x((a, b): (u32, u32)) -> R {}\n";
        assert!(server_fns_in(src).is_err());
    }
}
