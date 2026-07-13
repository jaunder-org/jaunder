//! The `server-fn-registrar` static check (#426): every `#[server]`-annotated fn
//! in the `web` crate must be named in the test registrar
//! (`ensure_server_fns_registered()` in `server/tests/helpers/mod.rs`).
//!
//! The integration/router test binaries link `web`/`jaunder` as rlibs, where
//! dead-code elimination drops each `#[server]` macro's auto-registration unless
//! the generated type is referenced explicitly — so the tests hand-list every
//! server fn via `server_fn::axum::register_explicit::<web::…>()`. A hand list
//! rots: a new `#[server]` fn compiles and passes its own crate's tests, but its
//! route silently 404s in integration until someone remembers to register it
//! (#358). This gate makes that omission a host-side failure instead.
//!
//! **Enumeration** uses `syn` (like [`crate::coverage::exempt`]): parse each
//! `web/src/**/*.rs`, collect free fns carrying a `#[server]` attribute, and map
//! the fn ident to its generated type name (`PascalCase(ident)`). The repo uses
//! only the `#[server(endpoint = "…")]` form, so that mapping is exact; an
//! unexpected positional-rename form (`#[server(SomeName)]`) is a **hard error**
//! rather than a silent mis-name.
//!
//! **Matching is by leaf type name**, not module path: re-exports
//! (`web/src/posts/mod.rs` does `pub use listing::*;`) make the registrar path
//! (`web::posts::ListLocalTimeline`) differ from the source path
//! (`web::posts::listing::…`). Because leaf matching cannot tell apart two
//! same-named `#[server]` fns in different modules, the gate also **fails on a
//! duplicate leaf name** — otherwise it could be blind to one of a pair being
//! unregistered (the ADR's leaf-collision precondition, enforced continuously).
//!
//! Only the *missing* direction is checked: a stale registrar entry (a type that
//! no longer exists) already fails to compile, so the compiler owns that side.
//! Unlike coverage exemption this gate is **fail-loud** — a parse failure is
//! reported, not swallowed, since a file we cannot enumerate could hide an
//! unregistered fn (a false pass).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Meta, Token};

use crate::result::{CommandResult, StepResult};

/// The `web` crate source root, scanned recursively for `#[server]` fns.
const WEB_SRC: &str = "web/src";
/// The single canonical registrar the enumerated fns must appear in.
const REGISTRAR: &str = "server/tests/helpers/mod.rs";

/// A `#[server]` fn discovered in a `web` source file: the generated type name
/// (`PascalCase` of the fn ident) and the 1-based line of its `#[server]` attr.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerFn {
    name: String,
    line: usize,
}

/// Every `#[server]` fn in one source file, or an error describing why the file
/// could not be enumerated. `Err` on a `syn` parse failure, or on the
/// unsupported `#[server(SomeName)]` positional-rename form — both would let an
/// unregistered fn slip through, so they fail the gate rather than pass silently.
fn server_fns_in(src: &str) -> Result<Vec<ServerFn>, String> {
    let file = syn::parse_file(src).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut v = ServerFnVisitor {
        fns: Vec::new(),
        errors: Vec::new(),
    };
    syn::visit::visit_file(&mut v, &file);
    if let Some(err) = v.errors.first() {
        return Err(err.clone());
    }
    Ok(v.fns)
}

struct ServerFnVisitor {
    fns: Vec<ServerFn>,
    errors: Vec<String>,
}

impl<'ast> syn::visit::Visit<'ast> for ServerFnVisitor {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if let Some(attr) = f.attrs.iter().find(|a| a.path().is_ident("server")) {
            match server_fn_default_named(attr) {
                Ok(true) => self.fns.push(ServerFn {
                    name: pascal_case(&f.sig.ident.to_string()),
                    line: attr.span().start().line,
                }),
                Ok(false) => self.errors.push(format!(
                    "line {}: unsupported #[server(...)] form (a positional type rename?) — \
                     the registrar gate assumes endpoint-only naming so the generated type is \
                     PascalCase(fn); rename via `endpoint =` or extend the gate",
                    attr.span().start().line
                )),
                Err(e) => self
                    .errors
                    .push(format!("line {}: {e}", attr.span().start().line)),
            }
        }
        syn::visit::visit_item_fn(self, f);
    }
}

/// Whether a `#[server]` attribute leaves the generated type at its default name
/// (`PascalCase(fn)`). True for the bare `#[server]` and for a list of only
/// `key = value` arguments (`endpoint = "…"`, `input = Json`, …). A bare
/// positional argument (`#[server(SomeName)]`) renames the type → `Ok(false)`;
/// an argument list we cannot parse as `Meta` → `Err`. Both are hard errors at
/// the call site.
fn server_fn_default_named(attr: &syn::Attribute) -> Result<bool, String> {
    match &attr.meta {
        Meta::Path(_) => Ok(true),
        Meta::List(_) => {
            let args = attr
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .map_err(|e| format!("cannot parse #[server(...)] arguments: {e}"))?;
            // Only `key = value` args (NameValue) keep the default name; a bare
            // path arg is a positional rename.
            Ok(args.iter().all(|m| matches!(m, Meta::NameValue(_))))
        }
        Meta::NameValue(_) => Err("unexpected `#[server = ...]` form".to_string()),
    }
}

/// `snake_case` fn ident → `PascalCase` generated type name
/// (`list_my_media` → `ListMyMedia`).
fn pascal_case(ident: &str) -> String {
    ident
        .split('_')
        .filter(|s| !s.is_empty())
        .map(|seg| {
            let mut chars = seg.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// The leaf type names registered via `register_explicit::<web::…::LEAF>()` in
/// the registrar source. Leaf = the last path segment of the turbofish type, so
/// the re-export path (`web::posts::CreatePost`) and any longer source path both
/// reduce to the same key (`CreatePost`).
///
/// Parsed with `syn`, not a text scan: the registrar is the one file whose
/// accuracy is load-bearing, so a *commented-out* (or string-literal)
/// `register_explicit::<…>()` must NOT count — disabling a real registration is
/// exactly the omission this gate exists to catch (#358). An unparseable
/// registrar yields the empty set (→ every fn reads as missing, a loud failure),
/// never a false pass; the real file always compiles, so this is a safety net.
fn registered_names(registrar_src: &str) -> std::collections::BTreeSet<String> {
    let Ok(file) = syn::parse_file(registrar_src) else {
        return std::collections::BTreeSet::new();
    };
    let mut v = RegistrarVisitor {
        names: std::collections::BTreeSet::new(),
    };
    syn::visit::visit_file(&mut v, &file);
    v.names
}

struct RegistrarVisitor {
    names: std::collections::BTreeSet<String>,
}

impl<'ast> syn::visit::Visit<'ast> for RegistrarVisitor {
    fn visit_expr_path(&mut self, ep: &'ast syn::ExprPath) {
        if let Some(leaf) = register_explicit_leaf(&ep.path) {
            self.names.insert(leaf);
        }
        syn::visit::visit_expr_path(self, ep);
    }
}

/// The leaf type name of a `…::register_explicit::<Type>` call path, or `None`
/// if the path is not a `register_explicit` turbofish. `Type`'s own generic args
/// (if any) are ignored — only its last path segment is the leaf.
fn register_explicit_leaf(path: &syn::Path) -> Option<String> {
    let seg = path.segments.last()?;
    if seg.ident != "register_explicit" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(ab) = &seg.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(syn::Type::Path(tp)) = ab.args.first()? else {
        return None;
    };
    Some(tp.path.segments.last()?.ident.to_string())
}

/// The failure detail for every `web` `#[server]` fn absent from the registrar,
/// every per-file enumeration error, and every duplicate leaf name — or `None`
/// when the registrar covers every enumerated fn and no name collides. Pure
/// given its inputs, so it is unit-tested directly.
fn problems(web_sources: &[(String, String)], registrar_src: &str) -> Option<String> {
    let registered = registered_names(registrar_src);
    let mut lines = Vec::new();
    let mut all_fns: Vec<(&str, ServerFn)> = Vec::new();
    for (path, src) in web_sources {
        match server_fns_in(src) {
            Err(msg) => lines.push(format!("{path}: {msg}")),
            Ok(fns) => all_fns.extend(fns.into_iter().map(|f| (path.as_str(), f))),
        }
    }

    // Duplicate leaf name → the gate matches by leaf and cannot tell the pair
    // apart, so one could be unregistered and slip through. Fail loudly.
    let mut by_name: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (path, f) in &all_fns {
        by_name
            .entry(f.name.as_str())
            .or_default()
            .push(format!("{path}:{}", f.line));
    }
    for (name, locs) in &by_name {
        if locs.len() > 1 {
            lines.push(format!(
                "duplicate #[server] type name `{name}` across {} — the registrar gate matches \
                 by leaf name and cannot tell them apart; rename one or extend the gate",
                locs.join(", ")
            ));
        }
    }

    // Missing registration.
    for (path, f) in &all_fns {
        if !registered.contains(&f.name) {
            lines.push(format!(
                "{path}:{}: web #[server] fn generating type `{}` is not registered in the test \
                 registrar",
                f.line, f.name
            ));
        }
    }

    if lines.is_empty() {
        return None;
    }
    lines.sort();
    lines.push(format!(
        "  recovery: add `server_fn::axum::register_explicit::<web::<mod>::<Type>>();` to \
         ensure_server_fns_registered() in {REGISTRAR} — every web #[server] fn must be \
         registered (#426)"
    ));
    Some(lines.join("\n"))
}

/// Collect every `.rs` file under `dir`, recursively.
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

/// Scan every `web/src` Rust file for `#[server]` fns and check each is
/// registered. A missing `web/src` tree or unreadable registrar is a hard
/// failure (not a silent pass), so a moved/renamed path can never quietly
/// disable the guard.
pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    if let Err(e) = rust_files(Path::new(WEB_SRC), &mut files) {
        result.push(
            StepResult::fail("server-fn-registrar").detail(format!("cannot scan {WEB_SRC}: {e}")),
        );
        return;
    }
    let registrar_src = match std::fs::read_to_string(REGISTRAR) {
        Ok(s) => s,
        Err(e) => {
            result.push(
                StepResult::fail("server-fn-registrar")
                    .detail(format!("cannot read {REGISTRAR}: {e}")),
            );
            return;
        }
    };
    // A file we listed but cannot READ is surfaced as a failure, not dropped:
    // an unenumerated source could hide an unregistered `#[server]` fn (a false
    // pass), the same fail-loud rule the module doc states.
    let mut sources = Vec::new();
    let mut read_errors = Vec::new();
    for p in &files {
        match std::fs::read_to_string(p) {
            Ok(s) => sources.push((p.display().to_string(), s)),
            Err(e) => read_errors.push(format!("{}: cannot read: {e}", p.display())),
        }
    }
    let step = match (read_errors.is_empty(), problems(&sources, &registrar_src)) {
        (true, None) => StepResult::ok("server-fn-registrar"),
        (_, prob) => {
            read_errors.extend(prob);
            StepResult::fail("server-fn-registrar").detail(read_errors.join("\n"))
        }
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap registrar statements in a fn so `syn::parse_file` accepts them — the
    /// real registrar's `register_explicit` calls live inside
    /// `ensure_server_fns_registered`, and the parser needs items, not bare stmts.
    fn wrap_reg(body: &str) -> String {
        format!("fn ensure() {{\n{body}\n}}\n")
    }

    #[test]
    fn extracts_pascalcase_name_and_line() {
        let src = "#[server(endpoint = \"/create_post\")]\npub async fn create_post() {}\n";
        let fns = server_fns_in(src).unwrap();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "CreatePost");
        assert_eq!(fns[0].line, 1);
    }

    #[test]
    fn multi_segment_ident_pascalcases_every_segment() {
        assert_eq!(pascal_case("list_my_media"), "ListMyMedia");
        assert_eq!(pascal_case("get_post_preview"), "GetPostPreview");
    }

    #[test]
    fn ignores_non_server_fns() {
        let src = "pub async fn plain() {}\n#[tokio::test]\nasync fn t() {}\n";
        assert!(server_fns_in(src).unwrap().is_empty());
    }

    #[test]
    fn bare_server_attr_uses_default_name() {
        let src = "#[server]\npub async fn save() {}\n";
        assert_eq!(server_fns_in(src).unwrap()[0].name, "Save");
    }

    #[test]
    fn endpoint_and_input_forms_are_accepted() {
        let src = "#[server(endpoint = \"/x\", input = Json)]\npub async fn x() {}\n";
        assert_eq!(server_fns_in(src).unwrap()[0].name, "X");
    }

    #[test]
    fn positional_rename_form_is_a_hard_error() {
        let src = "#[server(MyThing)]\npub async fn my_thing() {}\n";
        assert!(server_fns_in(src).is_err());
    }

    #[test]
    fn syn_parse_failure_is_an_error() {
        assert!(server_fns_in("fn broken( {{{ not valid").is_err());
    }

    #[test]
    fn registered_names_parses_leaf_types() {
        let reg = wrap_reg(
            "server_fn::axum::register_explicit::<web::posts::CreatePost>();\n\
             server_fn::axum::register_explicit::<web::media::ListMyMedia>();\n\
             let x = 1;",
        );
        let got = registered_names(&reg);
        assert!(got.contains("CreatePost"));
        assert!(got.contains("ListMyMedia"));
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn registered_names_ignores_a_commented_out_registration() {
        // A commented-out register_explicit disables the real registration — the
        // exact #358 omission the gate must catch — so it must NOT count. (A text
        // scan would; syn parsing does not.)
        let reg = wrap_reg(
            "server_fn::axum::register_explicit::<web::posts::CreatePost>();\n\
             // server_fn::axum::register_explicit::<web::posts::GetPost>();",
        );
        let got = registered_names(&reg);
        assert!(got.contains("CreatePost"));
        assert!(!got.contains("GetPost"), "commented-out reg must not count");
    }

    #[test]
    fn registered_names_takes_the_outer_leaf_of_a_generic_type() {
        // A turbofish with nested generics must reduce to the OUTER type's leaf,
        // not `Bar<Baz` (the old first-`>` text scan's bug).
        let reg = wrap_reg("server_fn::axum::register_explicit::<web::m::Bar<Baz>>();");
        assert_eq!(
            registered_names(&reg),
            std::collections::BTreeSet::from(["Bar".to_string()])
        );
    }

    #[test]
    fn problems_flags_an_unregistered_fn_by_name_and_path() {
        let sources = vec![(
            "web/src/media/mod.rs".to_string(),
            "#[server(endpoint = \"/list_my_media\")]\npub async fn list_my_media() {}\n"
                .to_string(),
        )];
        let registrar = wrap_reg("server_fn::axum::register_explicit::<web::posts::CreatePost>();");
        let detail = problems(&sources, &registrar).expect("a problem");
        assert!(detail.contains("ListMyMedia"));
        assert!(detail.contains("web/src/media/mod.rs"));
    }

    #[test]
    fn problems_is_none_when_registrar_covers_every_fn() {
        let sources = vec![(
            "web/src/posts/mod.rs".to_string(),
            "#[server(endpoint = \"/create_post\")]\npub async fn create_post() {}\n".to_string(),
        )];
        let registrar = wrap_reg("server_fn::axum::register_explicit::<web::posts::CreatePost>();");
        assert_eq!(problems(&sources, &registrar), None);
    }

    #[test]
    fn problems_matches_by_leaf_ignoring_reexport_module_path() {
        // The fn lives in `posts::listing` but is registered at the re-export path
        // `web::posts::…`; leaf matching must treat them as the same.
        let sources = vec![(
            "web/src/posts/listing.rs".to_string(),
            "#[server(endpoint = \"/list_home_feed\")]\npub async fn list_home_feed() {}\n"
                .to_string(),
        )];
        let registrar =
            wrap_reg("server_fn::axum::register_explicit::<web::posts::ListHomeFeed>();");
        assert_eq!(problems(&sources, &registrar), None);
    }

    #[test]
    fn problems_surfaces_a_hard_error_with_the_file() {
        let sources = vec![(
            "web/src/x.rs".to_string(),
            "#[server(MyThing)]\npub async fn my_thing() {}\n".to_string(),
        )];
        let detail = problems(&sources, "").expect("a hard error is reported");
        assert!(detail.contains("web/src/x.rs"));
    }

    #[test]
    fn problems_flags_a_duplicate_leaf_name() {
        // Two `#[server]` fns generating the same type name in different modules:
        // the leaf-matching gate cannot tell them apart, so it must fail loudly
        // even when both happen to be registered.
        let sources = vec![
            (
                "web/src/a.rs".to_string(),
                "#[server(endpoint = \"/thing_a\")]\npub async fn thing() {}\n".to_string(),
            ),
            (
                "web/src/b.rs".to_string(),
                "#[server(endpoint = \"/thing_b\")]\npub async fn thing() {}\n".to_string(),
            ),
        ];
        let registrar = wrap_reg("server_fn::axum::register_explicit::<web::a::Thing>();");
        let detail = problems(&sources, &registrar).expect("a duplicate is a problem");
        assert!(detail.contains("duplicate"));
        assert!(detail.contains("Thing"));
    }
}
