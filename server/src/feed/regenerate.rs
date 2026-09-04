use chrono::TimeDelta;

use common::{
    feed::{FeedFormat, FeedSurface},
    tagged_url::{self, BaseUrl, CanonicalUrl, FeedUrl, Permalink},
    time::UtcInstant,
    visibility::ViewerIdentity,
};
use host::etag;
use host::feed::{self, FeedItem, FeedMetadata, FeedPath, FeedTitle, HybridWindow};
use storage::{FeedCacheRow, PostRecord, PostStorage, PublisherSnapshot};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegenerateError {
    #[error("unparseable feed_url: {0}")]
    BadUrl(String),
    #[error("site.base_url must be configured to regenerate feeds")]
    BaseUrlRequired,
    #[error("storage error: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("publisher operation failed: {0}")]
    Publisher(#[source] anyhow::Error),
}

/// Renders a feed from one coherent publisher snapshot.
///
/// # Errors
///
/// Returns a rendering or post-storage error. Callers own generation-fenced
/// cache finalization; this function never writes the cache.
pub async fn render(
    snapshot: &PublisherSnapshot,
    posts: &dyn PostStorage,
    feed_path: FeedPath,
) -> Result<FeedCacheRow, RegenerateError> {
    // A `FeedPath` is always parseable, so this never yields `None`; `BadUrl` is
    // retained as a mapped (never-hit) error rather than an `expect()`/panic.
    let (surface, format) =
        feed::parse(&feed_path).ok_or_else(|| RegenerateError::BadUrl(feed_path.to_string()))?; // cov:ignore

    let window = HybridWindow {
        min_items: snapshot.feeds.min_items,
        min_days: snapshot.feeds.min_days,
    };
    let generated_at = UtcInstant::now();
    let representation_modified_at = UtcInstant::from(
        generated_at.value()
            - TimeDelta::nanoseconds(i64::from(generated_at.value().timestamp_subsec_nanos())),
    );
    let published = posts
        .list_published_in_window(&surface, &window, generated_at, &ViewerIdentity::Anonymous)
        .await
        .map_err(storage_err)?;

    let base = snapshot
        .identity
        .base_url
        .as_ref()
        .ok_or(RegenerateError::BaseUrlRequired)?;
    let items = build_feed_items(base, &published);
    let self_url: FeedUrl = tagged_url::compose(base, &feed_path);
    let canonical_path = match &surface {
        FeedSurface::Site => "/".to_owned(),
        FeedSurface::SiteTag { tag } => format!("/tags/{}/", urlencoding::encode(tag.as_ref())),
        FeedSurface::User { username } => format!("/~{username}/"),
        FeedSurface::UserTag { username, tag } => {
            format!("/~{username}/tags/{}/", urlencoding::encode(tag.as_ref()))
        }
    };
    let canonical_url: CanonicalUrl = tagged_url::compose(base, &canonical_path);
    let meta = FeedMetadata {
        title: FeedTitle::for_surface(&snapshot.identity.title, &surface),
        description: None,
        canonical_url,
        self_url,
        hub_url: snapshot.feeds.websub_hub_url.clone(),
        representation_modified_at: representation_modified_at.value(),
    };
    let body = match format {
        FeedFormat::Rss => feed::render_rss(&meta, &items),
        FeedFormat::Atom => feed::render_atom(&meta, &items),
        FeedFormat::Json => feed::render_json(&meta, &items),
    };
    let fingerprint = etag::feed_semantic_fingerprint(format, &meta, &items);
    let etag = etag::feed_etag(&fingerprint, representation_modified_at.value());
    FeedCacheRow::new(
        feed_path,
        body,
        etag,
        representation_modified_at,
        generated_at,
    )
    .map_err(|error| RegenerateError::BadUrl(error.to_string()))
}

fn storage_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> RegenerateError {
    RegenerateError::Storage(Box::new(e))
}

/// Builds the feed's items from the records the listing query already returned.
///
/// Tags come from [`PostRecord::tags`], which `list_published_in_window` populates
/// from the same query that loaded the rest of the row, slug-ordered (#772) — so
/// this performs **no** storage access at all. That is why it takes no
/// `PostStorage`, is not `async`, and cannot fail: a per-post tag read cannot be
/// reintroduced here without changing the signature.
fn build_feed_items(base: &BaseUrl, records: &[PostRecord]) -> Vec<FeedItem> {
    records
        .iter()
        .map(|p| {
            // list_published_in_window guarantees published_at IS NOT NULL,
            // but we fall back to created_at rather than panic if the
            // invariant is ever violated (matches PostRecord::permalink).
            let published_at = p.published_at.unwrap_or(p.created_at);
            FeedItem {
                id: p.post_id,
                // FeedItem carries the post's PostTitle unflattened (#470); renderers
                // read it out via Deref/Display at the external-crate boundary.
                title: p.title.clone(),
                // Compose the root-relative permalink to an absolute per-item feed URL
                // (atom Entry.id/link, RSS link/guid, JSON item url) — no relative atom:id
                // (#560, D1). `base` is the required site origin.
                // A struct-literal field cannot be ascribed, so the role is spelled as a
                // turbofish on the tag — the alias rule's stated exception.
                permalink: tagged_url::compose::<Permalink>(base, &p.permalink()),
                summary: p.summary.clone(),
                // FeedItem carries the post's RenderedHtml unflattened (#470); the value
                // is already rendered — no from_trusted rebuild, just propagate it.
                content_html: p.rendered_html.clone(),
                published_at: published_at.value(),
                updated_at: p.updated_at.value(),
                tags: p.tags.iter().map(|t| t.tag_display.clone()).collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Error as SqlxError;

    #[test]
    fn regenerate_error_storage_preserves_sqlx_source() {
        // §3.1a: storage_err boxes the originating error as a typed source
        // (downcastable for classification) instead of stringifying it.
        let err = storage_err(SqlxError::RowNotFound);
        let source = std::error::Error::source(&err).expect("Storage should expose a source");
        assert!(source.downcast_ref::<SqlxError>().is_some());
    }
}
