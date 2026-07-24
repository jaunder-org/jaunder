use leptos::prelude::*;
use leptos::server_fn::codec::Json;

// `TagLabel` is only named in the server-only `list_tags` body (the client build
// strips it via the `#[server]` stub), so gate it to match — the wire `TagSummary`
// it builds now lives in `common::seed`.
#[cfg(feature = "server")]
use {common::tag::TagLabel, std::sync::Arc, storage::PostStorage};

use common::seed::TagSummary;

use crate::error::WebResult;

/// Default number of suggestions returned to the autocomplete dropdown when
/// the caller doesn't specify a limit.
pub const DEFAULT_TAG_LIMIT: u32 = 10;

/// Hard upper bound on the autocomplete result set; protects the database
/// against pathological requests.
pub const MAX_TAG_LIMIT: u32 = 50;

/// Returns tag suggestions for the autocomplete dropdown.
///
/// `prefix` is a case-insensitive prefix match against the canonical slug;
/// `None` or whitespace-only returns the alphabetically-first tags. `limit`
/// defaults to [`DEFAULT_TAG_LIMIT`] and is clamped at [`MAX_TAG_LIMIT`].
///
/// `prefix` stays `String` (not `Tag`): it is a partial search fragment matched
/// with SQL `LIKE prefix%`, not a complete tag value — typing it `Tag` would
/// reject valid partials (ADR-0063 §4 boundary policy; #409 Decision 7).
#[server(endpoint = "/list_tags", input = Json)]
pub async fn list_tags(prefix: Option<String>, limit: Option<u32>) -> WebResult<Vec<TagSummary>> {
    boundary!("list_tags", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let resolved_limit = limit.unwrap_or(DEFAULT_TAG_LIMIT).clamp(1, MAX_TAG_LIMIT);
        let records = posts.list_tags(prefix.as_deref(), resolved_limit).await?;
        Ok(records
            .into_iter()
            .map(|rec| TagSummary {
                slug: rec.tag_slug.clone(),
                display: TagLabel::from(rec.tag_slug),
            })
            .collect())
    })
}

#[cfg(test)]
mod tests {
    use common::seed::TagSummary;
    use common::tag::TagLabel;

    /// #416 agreement: the `TagInput` commit path validates the raw token with
    /// `TagLabel::from_str` (the same rule the server applies at arg-decode), so
    /// client and server accept/reject identically — no re-implemented validator
    /// can drift. A trimmed, mixed-case token is accepted with its casing kept.
    #[test]
    fn tag_label_validation_agrees_client_and_server() {
        assert!("Rust".parse::<TagLabel>().is_ok());
        assert_eq!(" ab ".parse::<TagLabel>().unwrap().as_ref(), "ab");
        assert!("bad tag".parse::<TagLabel>().is_err());
    }

    /// Committing "Rust" yields a `TagSummary` whose `slug` is the canonical
    /// lowercase and whose `display` preserves the author's casing (Decision 4).
    #[test]
    fn tag_summary_preserves_casing_with_canonical_slug() {
        let label: TagLabel = "Rust".parse().unwrap();
        let summary = TagSummary {
            slug: label.slug(),
            display: label,
        };
        assert_eq!(summary.slug, "rust");
        assert_eq!(summary.display, "Rust");
    }
}
