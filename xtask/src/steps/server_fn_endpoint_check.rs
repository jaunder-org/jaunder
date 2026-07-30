//! The `server-fn-endpoint` static check (#684): every `#[server]` fn in the `web`
//! crate must carry `endpoint = "/<vertical>/<fn ident>"`.
//!
//! **Enumeration** is shared with [`super::server_fn_registrar_check`] and
//! [`super::server_fn_tracing_check`] via [`crate::web_server_fns`]; this module
//! supplies only the endpoint rule.
//!
//! Three things are checked:
//!
//! 1. **The endpoint is present.** Without it `server_fn` derives the URL from a
//!    hash of `CARGO_MANIFEST_DIR` + `module_path!()`, which varies by checkout
//!    directory — the wire path would depend on where the repository happens to sit
//!    on disk. So a missing `endpoint` is a hard error the author resolves; unlike
//!    the span name, [`Mode::Fix`] never synthesizes one.
//! 2. **The endpoint is derived, so the gate writes it** — `/<vertical>/<fn ident>`,
//!    where the vertical is the first path segment under `web/src`. Because it is a
//!    pure function of source path and identifier, asking an author to type it would
//!    be asking them to restate what the file already says and to keep 55 copies in
//!    sync by hand. So `cargo xtask check` **rewrites it** ([`Mode::Fix`], the same
//!    contract `fmt` and the span name have) and `cargo xtask validate` verifies
//!    without mutating. The literal still lands in the source, so a reader chasing a
//!    URL from a log or a Playwright spec can grep for it.
//! 3. **No two fns derive the same endpoint.** Defence in depth rather than an
//!    independent rule: since the endpoint is derived as `/<vertical>/<ident>`, two
//!    fns can collide only by sharing `(vertical, ident)` — which the registrar gate
//!    already hard-fails. It is cheap, and it holds the line if the derivation rule
//!    ever changes.
//!
//! Like the sibling server-fn gates this is **mandatory with no per-fn opt-out**,
//! and **fail-loud**: an unparseable or unreadable file is reported, never skipped,
//! because a file we cannot enumerate could hide a `#[server]` fn whose endpoint
//! nobody checked.

use std::collections::BTreeMap;
use std::path::Path;

use syn::spanned::Spanned;
use syn::Meta;

use crate::result::{CommandResult, Mode, StepResult};
use crate::web_server_fns::{self, apply_fixes, rewrite_attr_arg, vertical_of, LineFix, WEB_SRC};

/// The `endpoint = "…"` literal of a `#[server]` attribute.
///
/// `Ok(None)` when the attribute has no `endpoint` argument (the bare `#[server]`
/// included); `Err` when the argument list cannot be parsed as `Meta`, or when
/// `endpoint` is present but is not a string literal — both would leave the gate
/// guessing at what goes on the wire, so they fail rather than pass silently.
fn endpoint_of(attr: &syn::Attribute) -> Result<Option<String>, String> {
    let Some(args) = web_server_fns::server_attr_args(attr)? else {
        // The bare `#[server]` — no argument list, so no endpoint.
        return Ok(None);
    };
    for arg in args {
        let Meta::NameValue(nv) = &arg else {
            continue;
        };
        if !nv.path.is_ident("endpoint") {
            continue;
        }
        let syn::Expr::Lit(lit) = &nv.value else {
            return Err("`endpoint` must be a string literal".to_string());
        };
        let syn::Lit::Str(s) = &lit.lit else {
            return Err("`endpoint` must be a string literal".to_string());
        };
        return Ok(Some(s.value()));
    }
    Ok(None)
}

/// The failure detail for every non-conforming `#[server]` fn, or `None` when every
/// one conforms. Pure given its inputs, so it is unit-tested directly.
fn problems(web_sources: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    // Every `file:line` that derives a given endpoint, so a collision can name all
    // of its claimants rather than just the second one found.
    let mut claimants: BTreeMap<String, Vec<String>> = BTreeMap::new();

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
            let at = format!("line {}: `{}`", f.line, f.ident);
            let expected = format!("/{vertical}/{}", f.ident);
            claimants
                .entry(expected.clone())
                .or_default()
                .push(format!("{path}:{}", f.line));
            // `#[macros::server]` derives the endpoint from this same
            // `(vertical, ident)` pair (#714), so there is no declared literal to
            // check and nothing for `Mode::Fix` to write — rules 1 and 2 belong to
            // the macro. Rule 3 above still covers it: the derivation is identical,
            // so two such fns can still claim one wire path.
            if f.uses_macro_attr {
                continue;
            }
            match endpoint_of(&f.attrs[f.server_attr_index]) {
                Err(e) => lines.push(format!("{path}: {at}: {e}")),
                Ok(None) => lines.push(format!(
                    "{path}: {at}: #[server] has no `endpoint` — server_fn would derive the URL \
                     from a hash of CARGO_MANIFEST_DIR + module_path!(), which varies by checkout \
                     directory; write endpoint = \"{expected}\""
                )),
                Ok(Some(found)) if found != expected => lines.push(format!(
                    "{path}: {at}: endpoint is \"{found}\" but must be \"{expected}\" — the \
                     endpoint is derived from the source path and fn ident"
                )),
                Ok(Some(_)) => {}
            }
        }
    }

    for (endpoint, sites) in &claimants {
        if sites.len() > 1 {
            lines.push(format!(
                "endpoint \"{endpoint}\" is derived by more than one #[server] fn: {} — two fns \
                 cannot share a wire path",
                sites.join(", ")
            ));
        }
    }

    if lines.is_empty() {
        return None;
    }
    lines.sort();
    lines.push(
        "  recovery: every web #[server] fn carries endpoint = \"/<vertical>/<fn>\", derived from \
         its source path and ident; `cargo xtask check` writes it for you (#684)"
            .to_string(),
    );
    Some(lines.join("\n"))
}

/// The endpoint rewrites `Mode::Fix` should apply to one source file.
///
/// A missing `endpoint` is never synthesized — `rewrite_attr_arg` is called with
/// `insert_if_absent = false`, because inventing a wire path behind the author's
/// back is the one judgment this gate refuses to make. The problem is reported
/// instead. An attribute whose argument list this gate cannot parse is likewise
/// left alone: there is nothing safe to edit.
fn endpoint_fixes(path: &str, src: &str) -> Vec<LineFix> {
    let Ok(fns) = web_server_fns::server_fns_in(src) else {
        return Vec::new();
    };
    let Ok(vertical) = vertical_of(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = src.lines().collect();
    let mut fixes = Vec::new();
    for f in &fns {
        // The macro spelling owns its endpoint; there is no literal to rewrite, and
        // inserting one would be rejected by the macro itself.
        if f.uses_macro_attr {
            continue;
        }
        let attr = &f.attrs[f.server_attr_index];
        if endpoint_of(attr).is_err() {
            continue;
        }
        let span = attr.span();
        let (start, end) = (span.start().line, span.end().line);
        if start == 0 || end > lines.len() {
            continue;
        }
        let current = lines[start - 1..end].join("\n");
        let desired = format!("/{vertical}/{}", f.ident);
        if let Some(replacement) = rewrite_attr_arg(&current, "server", "endpoint", &desired, false)
        {
            fixes.push((start, end, replacement));
        }
    }
    fixes
}

/// Scan every `web/src` Rust file for `#[server]` fns and check each carries the
/// derived endpoint. A missing `web/src` tree or an unreadable file is a hard
/// failure (not a silent pass), so a moved path can never quietly disable the guard.
pub fn run(mode: Mode, result: &mut CommandResult) {
    let web = match web_server_fns::read_web_sources(Path::new(WEB_SRC)) {
        Ok(v) => v,
        Err(e) => {
            result.push(StepResult::fail("server-fn-endpoint").detail(e));
            return;
        }
    };
    let mut read_errors = web.read_errors;
    let mut sources = web.sources;

    // `Mode::Fix` (i.e. `cargo xtask check`) writes the derived endpoint, the same
    // contract `fmt` has; `Mode::Check` (`validate`) never mutates the tree. The
    // in-memory source is updated too, so `problems` below judges the fixed text and
    // one invocation suffices to go green.
    if matches!(mode, Mode::Fix) {
        for (path, src) in &mut sources {
            let fixes = endpoint_fixes(path, src);
            if fixes.is_empty() {
                continue;
            }
            let fixed = apply_fixes(src, fixes);
            if let Err(e) = std::fs::write(&*path, &fixed) {
                read_errors.push(format!("{path}: cannot write endpoint fix: {e}"));
                continue;
            }
            *src = fixed;
        }
    }

    let step = match (read_errors.is_empty(), problems(&sources)) {
        (true, None) => StepResult::ok("server-fn-endpoint"),
        (_, prob) => {
            read_errors.extend(prob);
            StepResult::fail("server-fn-endpoint").detail(read_errors.join("\n"))
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

    #[test]
    fn conforming_endpoint_is_accepted() {
        let s = src(
            "posts",
            "#[server(endpoint = \"/posts/create\")]\npub async fn create() -> R {}\n",
        );
        assert_eq!(problems(&s), None);
    }

    #[test]
    fn a_stale_endpoint_is_flagged_with_the_expected_value() {
        let s = src(
            "posts",
            "#[server(endpoint = \"/create_post\")]\npub async fn create() -> R {}\n",
        );
        let detail = problems(&s).expect("stale endpoint is a problem");
        assert!(
            detail.contains("/posts/create"),
            "must state the expected value: {detail}"
        );
    }

    #[test]
    fn a_missing_endpoint_is_flagged_as_a_hash_hazard() {
        // Without `endpoint`, server_fn derives the URL from a hash of
        // CARGO_MANIFEST_DIR + module_path!(), which varies by checkout directory.
        let s = src("tags", "#[server]\npub async fn list() -> R {}\n");
        let detail = problems(&s).expect("a missing endpoint is a problem");
        assert!(detail.contains("CARGO_MANIFEST_DIR"), "{detail}");
    }

    #[test]
    fn two_fns_deriving_the_same_endpoint_are_flagged_with_both_locations() {
        let s = vec![
            (
                "web/src/posts/api.rs".to_string(),
                "#[server(endpoint = \"/posts/create\")]\npub async fn create() -> R {}\n"
                    .to_string(),
            ),
            (
                "web/src/posts/api/listing.rs".to_string(),
                "#[server(endpoint = \"/posts/create\")]\npub async fn create() -> R {}\n"
                    .to_string(),
            ),
        ];
        let detail = problems(&s).expect("a duplicate endpoint is a problem");
        assert!(detail.contains("web/src/posts/api.rs"), "{detail}");
        assert!(detail.contains("web/src/posts/api/listing.rs"), "{detail}");
    }

    #[test]
    fn a_server_fn_directly_under_web_src_is_an_error() {
        let s = vec![(
            "web/src/loose.rs".to_string(),
            "#[server(endpoint = \"/x\")]\npub async fn x() -> R {}\n".to_string(),
        )];
        let detail = problems(&s).expect("no vertical is an error");
        assert!(detail.contains("web/src/loose.rs"), "{detail}");
    }

    #[test]
    fn fix_rewrites_the_endpoint_preserving_other_arguments() {
        let src_text =
            "#[server(endpoint = \"/create_post\", input = Json)]\npub async fn create() -> R {}\n";
        let fixes = endpoint_fixes("web/src/posts/api.rs", src_text);
        let fixed = web_server_fns::apply_fixes(src_text, fixes);
        assert!(fixed.contains("endpoint = \"/posts/create\""), "{fixed}");
        assert!(
            fixed.contains("input = Json"),
            "other args must survive: {fixed}"
        );
    }

    // --- the `#[macros::server]` spelling (#714) ---

    #[test]
    fn a_macro_attr_fn_is_not_reported_as_missing_an_endpoint() {
        // The macro derives `/audiences/rename` itself, so the "no `endpoint`"
        // hard error would be a false failure — and `Mode::Fix` deliberately never
        // synthesizes one, leaving the tree permanently red.
        let s = src(
            "audiences",
            "#[macros::server]\npub async fn rename() -> R {}\n",
        );
        assert_eq!(problems(&s), None);
        let with_args = src(
            "media",
            "#[macros::server(input = MultipartFormData, skip_all)]\npub async fn upload() -> R {}\n",
        );
        assert_eq!(problems(&with_args), None);
    }

    #[test]
    fn two_macro_attr_fns_deriving_one_endpoint_are_still_flagged() {
        // Skipping the presence check must not skip the collision rule: the macro
        // derives from `(vertical, ident)` exactly as this gate does.
        let s = vec![
            (
                "web/src/posts/api.rs".to_string(),
                "#[macros::server]\npub async fn create() -> R {}\n".to_string(),
            ),
            (
                "web/src/posts/api/listing.rs".to_string(),
                "#[macros::server]\npub async fn create() -> R {}\n".to_string(),
            ),
        ];
        let detail = problems(&s).expect("a duplicate endpoint is a problem");
        assert!(detail.contains("web/src/posts/api.rs"), "{detail}");
        assert!(detail.contains("web/src/posts/api/listing.rs"), "{detail}");
    }

    #[test]
    fn fix_leaves_a_macro_attr_fn_alone() {
        let src_text = "#[macros::server(skip_all)]\npub async fn create() -> R {}\n";
        assert!(endpoint_fixes("web/src/posts/api.rs", src_text).is_empty());
    }

    #[test]
    fn fix_does_not_synthesize_a_missing_endpoint() {
        // A missing `endpoint` is a hard error for the author to resolve, not
        // something Mode::Fix invents.
        let src_text = "#[server]\npub async fn list() -> R {}\n";
        assert!(endpoint_fixes("web/src/tags/api.rs", src_text).is_empty());
    }
}
