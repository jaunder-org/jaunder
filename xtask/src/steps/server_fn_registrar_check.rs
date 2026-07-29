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
//! **Enumeration** is shared with the `server-fn-tracing` gate (#511) via
//! [`crate::web_server_fns`]: it parses each `web/src/**/*.rs` with `syn` and
//! collects free fns carrying a `#[server]` attribute. This gate then maps each fn
//! ident to its generated type name (`PascalCase(ident)`). The repo uses only the
//! `#[server(endpoint = "…")]` form, so that mapping is exact; an unexpected
//! positional-rename form (`#[server(SomeName)]`) is a **hard error** rather than a
//! silent mis-name. That judgment stays here rather than in the shared enumerator —
//! it is about *this* gate's type-name mapping, and means nothing to the tracing gate.
//!
//! **Matching is on `(vertical, leaf)`** (#684), where the vertical is the first
//! path segment under `web/src` ([`crate::web_server_fns::vertical_of`]). The full
//! source module path is not usable: every vertical declares `mod api;` **privately**
//! and re-exports explicitly, so `web::posts::api::CreatePost` is not a nameable
//! path — the registrar can only spell `web::<vertical>::<Leaf>`, which is exactly
//! this key. It also makes the glob re-export (`web/src/posts/api.rs` does
//! `pub use listing::*;`) a non-issue: `posts/api.rs` and `posts/api/listing.rs`
//! share a vertical, so nothing needs resolving.
//!
//! The gate still **fails on a duplicate leaf within one vertical**, because two
//! such fns collapse to one key and a single registrar entry would satisfy both,
//! leaving the other to 404 silently (#358). The compiler does not own that case:
//! an item defined in `api.rs` silently *shadows* a glob-imported name of the same
//! ident from `listing`, so the pair compiles cleanly. Across different verticals
//! the same leaf is no longer a collision at all — that is what lets the fn idents
//! drop their vestigial vertical nouns.
//!
//! Only the *missing* direction is checked: a stale registrar entry (a type that
//! no longer exists) already fails to compile, so the compiler owns that side.
//! Unlike coverage exemption this gate is **fail-loud** — a parse failure is
//! reported, not swallowed, since a file we cannot enumerate could hide an
//! unregistered fn (a false pass).

use std::collections::BTreeMap;
use std::path::Path;

use syn::punctuated::Punctuated;
use syn::{Meta, Token};

use crate::result::{CommandResult, StepResult};
use crate::web_server_fns::{self, WEB_SRC};

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
///
/// A thin adapter over the shared [`web_server_fns::server_fns_in`]: the walk is
/// common to both server-fn gates, the `PascalCase` type-name mapping is this
/// gate's alone.
fn server_fns_in(src: &str) -> Result<Vec<ServerFn>, String> {
    let found = web_server_fns::server_fns_in(src)?;
    let mut fns = Vec::with_capacity(found.len());
    for f in found {
        let attr = &f.attrs[f.server_attr_index];
        match server_fn_default_named(attr) {
            Ok(true) => fns.push(ServerFn {
                name: pascal_case(&f.ident),
                line: f.line,
            }),
            Ok(false) => {
                return Err(format!(
                    "line {}: unsupported #[server(...)] form (a positional type rename?) — \
                     the registrar gate assumes endpoint-only naming so the generated type is \
                     PascalCase(fn); rename via `endpoint =` or extend the gate",
                    f.line
                ))
            }
            Err(e) => return Err(format!("line {}: {e}", f.line)),
        }
    }
    Ok(fns)
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

/// The `(vertical, leaf)` pairs registered via
/// `register_explicit::<web::<vertical>::<Leaf>>()` in the registrar source, plus
/// one message per entry that is not of that shape.
///
/// Parsed with `syn`, not a text scan: the registrar is the one file whose
/// accuracy is load-bearing, so a *commented-out* (or string-literal)
/// `register_explicit::<…>()` must NOT count — disabling a real registration is
/// exactly the omission this gate exists to catch (#358). An unparseable
/// registrar yields the empty set (→ every fn reads as missing, a loud failure),
/// never a false pass; the real file always compiles, so this is a safety net.
fn registered_entries(
    registrar_src: &str,
) -> (std::collections::BTreeSet<(String, String)>, Vec<String>) {
    let Ok(file) = syn::parse_file(registrar_src) else {
        return (std::collections::BTreeSet::new(), Vec::new());
    };
    let mut v = RegistrarVisitor {
        entries: std::collections::BTreeSet::new(),
        malformed: Vec::new(),
    };
    syn::visit::visit_file(&mut v, &file);
    (v.entries, v.malformed)
}

struct RegistrarVisitor {
    entries: std::collections::BTreeSet<(String, String)>,
    malformed: Vec<String>,
}

impl<'ast> syn::visit::Visit<'ast> for RegistrarVisitor {
    fn visit_expr_path(&mut self, ep: &'ast syn::ExprPath) {
        match register_explicit_entry(&ep.path) {
            Some(Ok(entry)) => {
                self.entries.insert(entry);
            }
            Some(Err(msg)) => self.malformed.push(msg),
            None => {}
        }
        syn::visit::visit_expr_path(self, ep);
    }
}

/// The `(vertical, leaf)` of a `…::register_explicit::<Type>` call path, or `None`
/// if the path is not a `register_explicit` turbofish.
///
/// `Type` must be exactly `web::<vertical>::<Leaf>`. Anything else is `Err`: a
/// longer path names a private module (`web::posts::api::Create` — `mod api` is
/// private in every vertical, so that path is not nameable), and a shorter one
/// omits the vertical this gate now matches on. Reporting rather than ignoring
/// matters because an unrecognized entry would otherwise register nothing and read
/// as a *missing* registration somewhere else — a confusing way to learn about a
/// typo. `Type`'s own generic args are ignored; only its segments are the key.
fn register_explicit_entry(path: &syn::Path) -> Option<Result<(String, String), String>> {
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
    let segments: Vec<String> = tp
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    let spelled = segments.join("::");
    match segments.as_slice() {
        [krate, vertical, leaf] if krate == "web" => Some(Ok((vertical.clone(), leaf.clone()))),
        _ => Some(Err(format!(
            "{REGISTRAR}: registrar entry `{spelled}` is not of the form \
             `web::<vertical>::<Type>` — the gate matches on (vertical, leaf), and every \
             vertical's `mod api` is private so only the re-export path is nameable"
        ))),
    }
}

/// The failure detail for every `web` `#[server]` fn absent from the registrar,
/// every per-file enumeration error, and every duplicate leaf name — or `None`
/// when the registrar covers every enumerated fn and no name collides. Pure
/// given its inputs, so it is unit-tested directly.
fn problems(web_sources: &[(String, String)], registrar_src: &str) -> Option<String> {
    let (registered, malformed) = registered_entries(registrar_src);
    let mut lines = malformed;
    // (vertical, fn) for every enumerated `#[server]` fn. The vertical comes from
    // the source path, not the fn — a file with no vertical directory yields no fns
    // and one error, because there is no key to match it on.
    let mut all_fns: Vec<(&str, &str, ServerFn)> = Vec::new();
    for (path, src) in web_sources {
        let vertical = match web_server_fns::vertical_of(path) {
            Ok(v) => v,
            Err(msg) => {
                // Only a file that actually declares a `#[server]` fn is a problem;
                // `web/src/lib.rs` and friends legitimately sit outside a vertical.
                if server_fns_in(src).is_ok_and(|fns| !fns.is_empty()) {
                    lines.push(msg);
                }
                continue;
            }
        };
        match server_fns_in(src) {
            Err(msg) => lines.push(format!("{path}: {msg}")),
            Ok(fns) => all_fns.extend(fns.into_iter().map(|f| (path.as_str(), vertical, f))),
        }
    }

    // Two `#[server]` fns with the same ident **in one vertical** collapse to a
    // single (vertical, leaf) key, so one registrar entry satisfies both and the
    // unregistered one would 404 silently (#358). The compiler does NOT catch this:
    // an item in `api.rs` silently shadows a glob-imported name of the same ident
    // from `pub use listing::*` (verified with rustc). Across *different* verticals
    // the same leaf is fine — that is what #684 unblocks.
    let mut by_key: BTreeMap<(&str, &str), Vec<String>> = BTreeMap::new();
    for (path, vertical, f) in &all_fns {
        by_key
            .entry((vertical, f.name.as_str()))
            .or_default()
            .push(format!("{path}:{}", f.line));
    }
    for ((vertical, name), locs) in &by_key {
        if locs.len() > 1 {
            lines.push(format!(
                "duplicate #[server] type name `{name}` within vertical `{vertical}` across {} — \
                 the registrar gate matches on (vertical, leaf) and cannot tell them apart; \
                 rename one",
                locs.join(", ")
            ));
        }
    }

    // Missing registration.
    for (path, vertical, f) in &all_fns {
        if !registered.contains(&((*vertical).to_string(), f.name.clone())) {
            lines.push(format!(
                "{path}:{}: web #[server] fn generating type `web::{vertical}::{}` is not \
                 registered in the test registrar",
                f.line, f.name
            ));
        }
    }

    if lines.is_empty() {
        return None;
    }
    lines.sort();
    lines.push(format!(
        "  recovery: add `server_fn::axum::register_explicit::<web::<vertical>::<Type>>();` to \
         ensure_server_fns_registered() in {REGISTRAR} — every web #[server] fn must be \
         registered (#426)"
    ));
    Some(lines.join("\n"))
}

/// Scan every `web/src` Rust file for `#[server]` fns and check each is
/// registered. A missing `web/src` tree or unreadable registrar is a hard
/// failure (not a silent pass), so a moved/renamed path can never quietly
/// disable the guard.
pub fn run(result: &mut CommandResult) {
    // A listed-but-unreadable file is surfaced as a failure, not dropped: an
    // unenumerated source could hide an unregistered `#[server]` fn (a false pass),
    // the same fail-loud rule the module doc states.
    let web = match web_server_fns::read_web_sources(Path::new(WEB_SRC)) {
        Ok(v) => v,
        Err(e) => {
            result.push(StepResult::fail("server-fn-registrar").detail(e));
            return;
        }
    };
    let mut read_errors = web.read_errors;
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
    let step = match (
        read_errors.is_empty(),
        problems(&web.sources, &registrar_src),
    ) {
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

    /// The `(vertical, leaf)` pairs a registrar body registers, ignoring the
    /// malformed-entry channel — most tests care only about what was accepted.
    fn entries_of(reg: &str) -> std::collections::BTreeSet<(String, String)> {
        registered_entries(reg).0
    }

    #[test]
    fn registered_entries_parses_vertical_and_leaf() {
        let reg = wrap_reg(
            "server_fn::axum::register_explicit::<web::posts::CreatePost>();\n\
             server_fn::axum::register_explicit::<web::media::ListMyMedia>();\n\
             let x = 1;",
        );
        let got = entries_of(&reg);
        assert!(got.contains(&("posts".to_string(), "CreatePost".to_string())));
        assert!(got.contains(&("media".to_string(), "ListMyMedia".to_string())));
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn registered_entries_ignores_a_commented_out_registration() {
        // A commented-out register_explicit disables the real registration — the
        // exact #358 omission the gate must catch — so it must NOT count. (A text
        // scan would; syn parsing does not.)
        let reg = wrap_reg(
            "server_fn::axum::register_explicit::<web::posts::CreatePost>();\n\
             // server_fn::axum::register_explicit::<web::posts::GetPost>();",
        );
        let got = entries_of(&reg);
        assert!(got.contains(&("posts".to_string(), "CreatePost".to_string())));
        assert!(
            !got.contains(&("posts".to_string(), "GetPost".to_string())),
            "commented-out reg must not count"
        );
    }

    #[test]
    fn registered_entries_takes_the_outer_leaf_of_a_generic_type() {
        // A turbofish with nested generics must reduce to the OUTER type's leaf,
        // not `Bar<Baz` (the old first-`>` text scan's bug).
        let reg = wrap_reg("server_fn::axum::register_explicit::<web::m::Bar<Baz>>();");
        assert_eq!(
            entries_of(&reg),
            std::collections::BTreeSet::from([("m".to_string(), "Bar".to_string())])
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
    fn problems_ignores_the_reexport_module_path_within_a_vertical() {
        // The fn lives in `posts::api::listing` but is registered at the re-export
        // path `web::posts::…`. Both sit under `web/src/posts/`, so the vertical
        // matches and the glob re-export needs no resolution.
        let sources = vec![(
            "web/src/posts/api/listing.rs".to_string(),
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
            "web/src/posts/api.rs".to_string(),
            "#[server(MyThing)]\npub async fn my_thing() {}\n".to_string(),
        )];
        let detail = problems(&sources, "").expect("a hard error is reported");
        assert!(detail.contains("web/src/posts/api.rs"));
    }

    // --- (vertical, leaf) matching (#684) ---

    #[test]
    fn same_leaf_in_two_verticals_both_registered_is_fine() {
        // The change that unblocks #684: `Create` in posts and audiences are
        // distinct keys, not a collision.
        let sources = vec![
            (
                "web/src/posts/api.rs".to_string(),
                "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
            ),
            (
                "web/src/audiences/api.rs".to_string(),
                "#[server(endpoint = \"/audiences/create\")]\npub async fn create() {}\n"
                    .to_string(),
            ),
        ];
        let registrar = wrap_reg(
            "server_fn::axum::register_explicit::<web::posts::Create>();\n\
             server_fn::axum::register_explicit::<web::audiences::Create>();",
        );
        assert_eq!(problems(&sources, &registrar), None);
    }

    #[test]
    fn the_unregistered_half_of_a_cross_vertical_pair_is_named_with_its_vertical() {
        let sources = vec![
            (
                "web/src/posts/api.rs".to_string(),
                "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
            ),
            (
                "web/src/audiences/api.rs".to_string(),
                "#[server(endpoint = \"/audiences/create\")]\npub async fn create() {}\n"
                    .to_string(),
            ),
        ];
        let registrar = wrap_reg("server_fn::axum::register_explicit::<web::audiences::Create>();");
        let detail = problems(&sources, &registrar).expect("posts::Create is unregistered");
        assert!(detail.contains("web/src/posts/api.rs"), "{detail}");
        assert!(
            detail.contains("posts"),
            "the vertical is what disambiguates the pair: {detail}"
        );
        assert!(!detail.contains("web/src/audiences/api.rs"), "{detail}");
    }

    #[test]
    fn a_duplicate_ident_within_one_vertical_fails_even_when_registered() {
        // Glob shadowing (`pub use listing::*;` at web/src/posts/api.rs:16) makes
        // this COMPILE — verified with rustc. Both fns collapse to (posts, Create),
        // one registrar entry satisfies both, and the unregistered one silently
        // 404s. That is the #358 hole; this gate is the only thing that catches it,
        // which is why the duplicate check is narrowed rather than deleted.
        let sources = vec![
            (
                "web/src/posts/api.rs".to_string(),
                "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
            ),
            (
                "web/src/posts/api/listing.rs".to_string(),
                "#[server(endpoint = \"/posts/create_other\")]\npub async fn create() {}\n"
                    .to_string(),
            ),
        ];
        let registrar = wrap_reg("server_fn::axum::register_explicit::<web::posts::Create>();");
        let detail =
            problems(&sources, &registrar).expect("a within-vertical duplicate is a problem");
        assert!(detail.contains("duplicate"), "{detail}");
        assert!(detail.contains("posts"), "{detail}");
        assert!(detail.contains("web/src/posts/api.rs"), "{detail}");
        assert!(detail.contains("web/src/posts/api/listing.rs"), "{detail}");
    }

    #[test]
    fn a_registrar_entry_that_is_not_web_vertical_leaf_is_reported_as_malformed() {
        let sources = vec![(
            "web/src/posts/api.rs".to_string(),
            "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
        )];
        // Four segments — `api` is a private module, so this path is not nameable.
        let registrar =
            wrap_reg("server_fn::axum::register_explicit::<web::posts::api::Create>();");
        let detail = problems(&sources, &registrar).expect("malformed entry is reported");
        assert!(detail.contains("web::posts::api::Create"), "{detail}");
    }

    #[test]
    fn a_two_segment_registrar_entry_is_reported_as_malformed() {
        let sources = vec![(
            "web/src/posts/api.rs".to_string(),
            "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
        )];
        let registrar = wrap_reg("server_fn::axum::register_explicit::<posts::Create>();");
        let detail = problems(&sources, &registrar).expect("malformed entry is reported");
        assert!(detail.contains("posts::Create"), "{detail}");
    }

    #[test]
    fn a_server_fn_directly_under_web_src_is_an_error() {
        let sources = vec![(
            "web/src/loose.rs".to_string(),
            "#[server(endpoint = \"/x\")]\npub async fn x() {}\n".to_string(),
        )];
        let detail = problems(&sources, &wrap_reg("")).expect("no vertical is an error");
        assert!(detail.contains("web/src/loose.rs"), "{detail}");
        assert!(detail.contains("vertical"), "{detail}");
    }
}
