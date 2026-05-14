use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::error::InternalError;
use crate::error::WebResult;

#[cfg(feature = "ssr")]
use common::storage::AppState;
#[cfg(feature = "ssr")]
use std::sync::Arc;

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
    crate::web_server_fn!("list_tags", prefix, limit => {
        let state = expect_context::<Arc<AppState>>();
        let resolved_limit = limit.unwrap_or(DEFAULT_TAG_LIMIT).clamp(1, MAX_TAG_LIMIT);
        let records = state
            .posts
            .list_tags(prefix.as_deref(), resolved_limit)
            .await
            .map_err(InternalError::storage)?;
        // `display` mirrors the canonical slug for now; the next milestone step
        // populates a real display string once the tag input emits one.
        Ok(records
            .into_iter()
            .map(|rec| TagSummary {
                slug: rec.tag_slug.to_string(),
                display: rec.tag_slug.to_string(),
            })
            .collect())
    })
}
