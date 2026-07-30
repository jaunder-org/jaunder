//! The flow-coverage gate's view of the `#[server]` inventory (#681).
//!
//! [`crate::web_server_fns`] is deliberately **policy-free** (#511): it reports
//! idents, params and attributes, and leaves every judgement to the consuming
//! gate. The coverage gate needs three things that are derivations rather than
//! reports — the *declared endpoint*, the *module* the fn was written in, and
//! nothing else — so they are derived here instead of widening the shared
//! enumerator, which would silently widen the tracing and registrar gates too.
//!
//! Notably this does **not** carry the `PascalCase` generated type name: that is
//! the registrar gate's key, not coverage's, and duplicating its `pascal_case`
//! here would be a second source of truth for a mapping coverage never consults.

use std::path::Path;

/// One `#[server]` fn, as the coverage gate needs to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFn {
    /// The fn ident as written — what a derived span name carries (see
    /// `server_fn_coverage::extract`). **Not** the coverage key on its own: since
    /// #684 dropped the vertical noun from these idents, fifteen fns share six
    /// idents across verticals ([`ServerFn::qualified`]).
    pub ident: String,
    /// The endpoint the fn serves, leading slash stripped: the declared
    /// `endpoint = "…"` for a leptos `#[server]`, or the `<vertical>/<ident>` that
    /// `#[macros::server]` derives (#714). `None` for a bare `#[server]`, whose
    /// generated path carries a hash suffix and so cannot be matched by name — the
    /// verdict reports that as drift rather than guessing.
    pub endpoint: Option<String>,
    /// Crate-relative `::`-joined module path, from [`module_path_of`].
    pub module: String,
    /// 1-based line of the `#[server]` attribute.
    pub line: usize,
}

impl ServerFn {
    /// The vertical the fn belongs to: the module path's **first** segment, so
    /// `posts::api` and `posts::api::listing` both read as `posts`.
    ///
    /// One definition, used by every derivation that needs a vertical — the
    /// candidate span names, the coverage key, and the expected endpoint — so a
    /// change to what "vertical" means cannot land in one of them and not the
    /// others.
    pub fn vertical(&self) -> &str {
        self.module.split("::").next().unwrap_or(&self.module)
    }

    /// `<vertical>::<ident>` — how coverage identifies a fn.
    ///
    /// The bare ident is not enough: #684 dropped the vertical noun from the
    /// idents, so `posts::create`, `audiences::create` and `invites::create` all
    /// answer to `create`. Keying on the ident would have covered all three the
    /// moment any one of them was exercised — the gate would go green over two
    /// untested flows.
    ///
    /// `::` rather than `/`, because this is a Rust path and must not be mistaken
    /// for the `/`-separated endpoint. Vertical plus ident rather than the full
    /// module path, because `posts::api` and `posts::api::listing` denote one
    /// vertical — and because that is already ADR-0066's registrar key
    /// `(vertical, leaf)`, so all four `#[server]` gates share one notion of
    /// identity.
    pub fn qualified(&self) -> String {
        format!("{}::{}", self.vertical(), self.ident)
    }
}

/// Every `#[server]` fn in one source file, or a message describing why the file
/// could not be enumerated. `module` is the caller's [`module_path_of`] result for
/// the file, threaded through because the enumerator sees only the source text.
pub fn server_fns_in(src: &str, module: &str) -> Result<Vec<ServerFn>, String> {
    Ok(crate::web_server_fns::server_fns_in(src)?
        .into_iter()
        .map(|f| {
            let uses_macro_attr = f.uses_macro_attr;
            let mut out = ServerFn {
                endpoint: endpoint_of(&f.attrs[f.server_attr_index]),
                ident: f.ident,
                module: module.to_string(),
                line: f.line,
            };
            // `#[macros::server]` declares no `endpoint = "…"` — it *derives*
            // `/<vertical>/<ident>` (#714), so reading the attribute yields `None`
            // and every one of these fns would read as an unmatchable bare
            // `#[server]`, i.e. drift. Repeat the derivation instead, via
            // [`ServerFn::vertical`] so the notion of "vertical" stays single-sourced.
            // Stored **without** the leading slash, like every other value in this
            // field — `snapshot.rs` compares it against `"{vertical}/{ident}"`.
            if uses_macro_attr {
                out.endpoint = Some(format!("{}/{}", out.vertical(), out.ident));
            }
            out
        })
        .collect())
}

/// The `endpoint = "…"` value on a `#[server]` attribute, leading slash stripped.
///
/// Walks the argument list rather than assuming a position: the repo has
/// `#[server(input = MultipartFormData, endpoint = "/upload_media")]`, where
/// reading the first argument loses the endpoint **silently** — the fn then looks
/// like a bare `#[server]` and drops out of URI matching.
fn endpoint_of(attr: &syn::Attribute) -> Option<String> {
    let mut endpoint = None;
    // A bare `#[server]` has no list to walk. That is not an error here — it is
    // simply "no declared endpoint", which the verdict reports.
    let _ = attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("endpoint") {
            let literal: syn::LitStr = meta.value()?.parse()?;
            endpoint = Some(literal.value().trim_start_matches('/').to_string());
            return Ok(());
        }
        // Consume any other `key = value` argument so the walk reaches later ones.
        if meta.input.peek(syn::Token![=]) {
            let _: syn::Expr = meta.value()?.parse()?;
        }
        Ok(())
    });
    endpoint
}

/// `web/src`-relative source path → `::`-joined module path.
/// `lib.rs` → `""`; `site/api.rs` → `site::api`; `posts/mod.rs` → `posts`;
/// `posts/api/listing.rs` → `posts::api::listing`.
pub fn module_path_of(rel_path: &Path) -> String {
    let mut segments: Vec<String> = rel_path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if let Some(last) = segments.pop() {
        let stem = last.trim_end_matches(".rs");
        // `mod.rs` names its directory; `lib.rs` at the root names nothing.
        if stem != "mod" && stem != "lib" {
            segments.push(stem.to_string());
        }
    }
    segments.join("::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_ident_endpoint_and_module() {
        let src = "#[server(endpoint = \"/list_my_media\")]\npub async fn list_my_media() {}\n";
        let fns = server_fns_in(src, "media::api").expect("enumerates");
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].ident, "list_my_media");
        assert_eq!(fns[0].endpoint.as_deref(), Some("list_my_media"));
        assert_eq!(fns[0].module, "media::api");
    }

    #[test]
    fn endpoint_is_found_after_another_argument() {
        // The `upload_media` shape. Reading the first argument would lose it, and
        // lose it silently — the fn would read as a bare `#[server]`.
        let src = "#[server(input = MultipartFormData, endpoint = \"/upload_media\")]\n\
                   pub async fn upload_media() {}\n";
        let fns = server_fns_in(src, "media::api").expect("enumerates");
        assert_eq!(fns[0].endpoint.as_deref(), Some("upload_media"));
    }

    #[test]
    fn bare_server_attr_has_no_endpoint() {
        let src = "#[server]\npub async fn thing() {}\n";
        let fns = server_fns_in(src, "x").expect("enumerates");
        assert_eq!(fns[0].endpoint, None);
        assert_eq!(fns[0].ident, "thing");
    }

    #[test]
    fn macro_attr_endpoint_is_derived_without_a_leading_slash() {
        // `#[macros::server]` declares nothing, so the inventory must repeat the
        // macro's own derivation — and store it the way this field is stored, with
        // the leading slash off, since `snapshot.rs` matches `"{vertical}/{ident}"`.
        let src = "#[macros::server]\npub async fn create() {}\n";
        let fns = server_fns_in(src, "audiences::api").expect("enumerates");
        assert_eq!(fns[0].endpoint.as_deref(), Some("audiences/create"));
    }

    #[test]
    fn macro_attr_arguments_do_not_disturb_the_derived_endpoint() {
        // `skip(...)` is routed to the span, `input = …` to `#[server]`; neither
        // says anything about the wire path.
        let src = "#[macros::server(input = MultipartFormData, skip_all)]\n\
                   pub async fn upload() {}\n";
        let fns = server_fns_in(src, "media::api").expect("enumerates");
        assert_eq!(fns[0].endpoint.as_deref(), Some("media/upload"));
    }

    #[test]
    fn module_path_of_maps_the_repo_shapes() {
        assert_eq!(module_path_of(Path::new("lib.rs")), "");
        assert_eq!(module_path_of(Path::new("site/api.rs")), "site::api");
        assert_eq!(module_path_of(Path::new("posts/mod.rs")), "posts");
        assert_eq!(
            module_path_of(Path::new("posts/api/listing.rs")),
            "posts::api::listing"
        );
    }

    #[test]
    fn vertical_is_the_modules_first_segment() {
        let src = "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n";
        let deep = server_fns_in(src, "posts::api::listing").expect("enumerates");
        assert_eq!(deep[0].vertical(), "posts");
        let shallow = server_fns_in(src, "posts").expect("enumerates");
        assert_eq!(shallow[0].vertical(), "posts");
    }

    #[test]
    fn qualified_separates_one_ident_across_two_verticals() {
        // The #684 collision in miniature: three verticals declare `create`, and
        // the ident alone cannot tell them apart.
        let src = "#[server]\npub async fn create() {}\n";
        let posts = server_fns_in(src, "posts::api").expect("enumerates");
        let audiences = server_fns_in(src, "audiences::api").expect("enumerates");
        assert_eq!(posts[0].qualified(), "posts::create");
        assert_eq!(audiences[0].qualified(), "audiences::create");
        assert_ne!(posts[0].qualified(), audiences[0].qualified());
    }

    #[test]
    fn an_unparseable_file_is_an_error_not_an_empty_list() {
        assert!(server_fns_in("fn (", "x").is_err());
    }
}
