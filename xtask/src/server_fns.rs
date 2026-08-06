//! The flow-coverage gate's view of the `#[macros::server]` inventory (#681).
//!
//! [`crate::web_server_fns`] is deliberately **policy-free** (#511): it reports
//! idents, params and attributes, and leaves every judgement to the consuming
//! gate. The coverage gate needs two things that are derivations rather than
//! reports — the *endpoint* the fn serves and the *module* it was written in — so
//! they are derived here instead of widening the shared enumerator, which would
//! silently widen the tracing and registrar gates too.
//!
//! Notably this does **not** carry the `PascalCase` generated type name: that is
//! the registrar gate's key, not coverage's, and duplicating its `pascal_case`
//! here would be a second source of truth for a mapping coverage never consults.

use std::path::Path;

/// One `#[macros::server]` fn, as the coverage gate needs to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFn {
    /// The fn ident as written — what a derived span name carries (see
    /// `server_fn_coverage::extract`). **Not** the coverage key on its own: since
    /// #684 dropped the vertical noun from these idents, fifteen fns share six
    /// idents across verticals ([`ServerFn::qualified`]).
    pub ident: String,
    /// The endpoint the fn serves, leading slash stripped: the `<vertical>/<ident>`
    /// that `#[macros::server]` derives (#714).
    ///
    /// Still an `Option` because the *verdict* owns the "unmatchable path" case: a
    /// `None` endpoint cannot be matched against a request URI by name, and
    /// `snapshot.rs` reports that as drift rather than guessing. Enumeration can no
    /// longer produce one — the endpoint is derived, never read — so the arm is a
    /// backstop, not a live spelling.
    pub endpoint: Option<String>,
    /// Crate-relative `::`-joined module path, from [`module_path_of`].
    pub module: String,
    /// 1-based line of the `#[macros::server]` attribute.
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
    /// `(vertical, leaf)`, so all three server-fn gates share one notion of
    /// identity.
    pub fn qualified(&self) -> String {
        format!("{}::{}", self.vertical(), self.ident)
    }
}

/// Every `#[macros::server]` fn in one source file, or a message describing why the
/// file could not be enumerated. `module` is the caller's [`module_path_of`] result
/// for the file, threaded through because the enumerator sees only the source text.
pub fn server_fns_in(src: &str, module: &str) -> Result<Vec<ServerFn>, String> {
    Ok(crate::web_server_fns::server_fns_in(src)?
        .into_iter()
        .map(|f| {
            let mut out = ServerFn {
                endpoint: None,
                ident: f.ident,
                module: module.to_string(),
                line: f.line,
            };
            // The attribute declares no `endpoint = "…"` — the macro *derives*
            // `/<vertical>/<ident>` (#714), so the inventory repeats that derivation
            // rather than reading anything. Via [`ServerFn::vertical`], so the notion
            // of "vertical" stays single-sourced, and stored **without** the leading
            // slash like every other value in this field — `snapshot.rs` compares it
            // against `"{vertical}/{ident}"`.
            out.endpoint = Some(format!("{}/{}", out.vertical(), out.ident));
            out
        })
        .collect())
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
    use crate::test_support::retired_server_fn;

    #[test]
    fn captures_ident_endpoint_and_module() {
        let src = "#[macros::server]\npub async fn list_my_media() {}\n";
        let fns = server_fns_in(src, "media::api").expect("enumerates");
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].ident, "list_my_media");
        assert_eq!(fns[0].endpoint.as_deref(), Some("media/list_my_media"));
        assert_eq!(fns[0].module, "media::api");
    }

    #[test]
    fn the_old_spelling_no_longer_enumerates() {
        // #714 retired leptos's bare `#[server]`. It must not reach the inventory:
        // it declares its own `endpoint = "…"`, which nothing here reads any more, so
        // an enumerated one would carry a derived endpoint that contradicts the wire.
        let declared = retired_server_fn(
            "(endpoint = \"/list_my_media\")",
            "pub async fn list_my_media() {}",
        );
        assert!(
            server_fns_in(&declared, "media::api")
                .expect("enumerates")
                .is_empty()
        );
        let bare = retired_server_fn("", "pub async fn thing() {}");
        assert!(server_fns_in(&bare, "x").expect("enumerates").is_empty());
    }

    #[test]
    fn the_endpoint_is_derived_without_a_leading_slash() {
        // The attribute declares nothing, so the inventory must repeat the macro's
        // own derivation — and store it the way this field is stored, with the
        // leading slash off, since `snapshot.rs` matches `"{vertical}/{ident}"`.
        let src = "#[macros::server]\npub async fn create() {}\n";
        let fns = server_fns_in(src, "audiences::api").expect("enumerates");
        assert_eq!(fns[0].endpoint.as_deref(), Some("audiences/create"));
    }

    #[test]
    fn attribute_arguments_do_not_disturb_the_derived_endpoint() {
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
        let src = "#[macros::server]\npub async fn create() {}\n";
        let deep = server_fns_in(src, "posts::api::listing").expect("enumerates");
        assert_eq!(deep[0].vertical(), "posts");
        let shallow = server_fns_in(src, "posts").expect("enumerates");
        assert_eq!(shallow[0].vertical(), "posts");
    }

    #[test]
    fn qualified_separates_one_ident_across_two_verticals() {
        // The #684 collision in miniature: three verticals declare `create`, and
        // the ident alone cannot tell them apart.
        let src = "#[macros::server]\npub async fn create() {}\n";
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
