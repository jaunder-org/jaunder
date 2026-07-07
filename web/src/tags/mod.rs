use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {crate::error::InternalError, std::sync::Arc, storage::PostStorage};

use crate::error::WebResult;

/// Default number of suggestions returned to the autocomplete dropdown when
/// the caller doesn't specify a limit.
pub const DEFAULT_TAG_LIMIT: u32 = 10;

/// Hard upper bound on the autocomplete result set; protects the database
/// against pathological requests.
pub const MAX_TAG_LIMIT: u32 = 50;

/// A tag row returned by [`list_tags`].
///
/// `slug` is the canonical lowercase form used in URLs (`/tags/:slug`).
/// `display` is the case-preserving form the author most recently used; the
/// autocomplete dropdown should render this to the user. When a tag has been
/// applied with multiple casings across posts, `display` reflects whichever
/// row the underlying `SELECT` returned first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagSummary {
    pub slug: String,
    pub display: String,
}

/// Returns tag suggestions for the autocomplete dropdown.
///
/// `prefix` is a case-insensitive prefix match against the canonical slug;
/// `None` or whitespace-only returns the alphabetically-first tags. `limit`
/// defaults to [`DEFAULT_TAG_LIMIT`] and is clamped at [`MAX_TAG_LIMIT`].
#[server(endpoint = "/list_tags", input = Json)]
pub async fn list_tags(prefix: Option<String>, limit: Option<u32>) -> WebResult<Vec<TagSummary>> {
    boundary!("list_tags", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let resolved_limit = limit.unwrap_or(DEFAULT_TAG_LIMIT).clamp(1, MAX_TAG_LIMIT);
        let records = posts
            .list_tags(prefix.as_deref(), resolved_limit)
            .await
            .map_err(InternalError::storage)?;
        Ok(records
            .into_iter()
            .map(|rec| TagSummary {
                slug: rec.tag_slug.to_string(),
                display: rec.tag_slug.to_string(),
            })
            .collect())
    })
}

// ─── Pure helpers for the TagInput UI ─────────────────────────
// Client-side tag-slug validation, shared by the wasm-only `pages::ui::TagInput`
// and host-tested here. Kept in `web::tags` (both-target, no `target_arch` gate)
// rather than in `pages/` so it stays coverage-measured once `pages` is wasm-only.

/// Returns `true` when `s` is a valid tag slug: non-empty, first char
/// `[a-z0-9]`, remaining chars `[a-z0-9-]`.  The input must already be
/// lowercased (call [`normalize_tag_token`] first).
///
/// Mirrors [`common::tag::Tag::from_str`] so client and server agree on
/// validity without importing `common` into the WASM bundle.
#[must_use]
pub fn is_valid_tag_slug(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        None => false,
        Some(c) if !c.is_ascii_lowercase() && !c.is_ascii_digit() => false,
        _ => chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
    }
}

/// Trims whitespace from `raw` and lowercases the result.
#[must_use]
pub fn normalize_tag_token(raw: &str) -> String {
    raw.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{is_valid_tag_slug, normalize_tag_token};

    // ─── is_valid_tag_slug ────────────────────────────────────

    #[test]
    fn tag_slug_accepts_lowercase_alpha() {
        assert!(is_valid_tag_slug("rust"));
    }

    #[test]
    fn tag_slug_accepts_leading_digit() {
        assert!(is_valid_tag_slug("42things"));
    }

    #[test]
    fn tag_slug_accepts_hyphens_in_body() {
        assert!(is_valid_tag_slug("hello-world"));
    }

    #[test]
    fn tag_slug_accepts_single_char() {
        assert!(is_valid_tag_slug("a"));
        assert!(is_valid_tag_slug("0"));
    }

    #[test]
    fn tag_slug_rejects_empty() {
        assert!(!is_valid_tag_slug(""));
    }

    #[test]
    fn tag_slug_rejects_leading_hyphen() {
        assert!(!is_valid_tag_slug("-hello"));
    }

    #[test]
    fn tag_slug_rejects_uppercase() {
        assert!(!is_valid_tag_slug("Rust"));
        assert!(!is_valid_tag_slug("RUST"));
    }

    #[test]
    fn tag_slug_rejects_spaces() {
        assert!(!is_valid_tag_slug("hello world"));
    }

    #[test]
    fn tag_slug_rejects_special_chars() {
        assert!(!is_valid_tag_slug("tag@site"));
        assert!(!is_valid_tag_slug("tag_name"));
    }

    // ─── normalize_tag_token ──────────────────────────────────

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_tag_token("  rust  "), "rust");
    }

    #[test]
    fn normalize_lowercases() {
        assert_eq!(normalize_tag_token("Rust"), "rust");
        assert_eq!(normalize_tag_token("HELLO-WORLD"), "hello-world");
    }

    #[test]
    fn normalize_empty_stays_empty() {
        assert_eq!(normalize_tag_token(""), "");
        assert_eq!(normalize_tag_token("   "), "");
    }

    #[test]
    fn normalize_then_validate_roundtrip() {
        let normalized = normalize_tag_token("  Hello-World  ");
        assert!(is_valid_tag_slug(&normalized));
        assert_eq!(normalized, "hello-world");
    }
}
