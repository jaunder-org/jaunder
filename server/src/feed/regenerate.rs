use common::feed::{
    FeedFormat, FeedItem, FeedMetadata, FeedPath, FeedSurface, FeedTitle, HybridWindow, feed_etag,
    parse,
};
use common::tagged_url::{BaseUrl, CanonicalUrl, FeedUrl, Permalink, compose};
use common::time::UtcInstant;
use storage::{FeedCacheRow, FeedCacheStorage, PostRecord, PostStorage, SiteConfigStorage};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegenerateError {
    #[error("unparseable feed_url: {0}")]
    BadUrl(String),
    #[error("site.base_url must be configured to regenerate feeds")]
    BaseUrlRequired,
    #[error("storage error: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Regenerates a feed for the given URL by fetching published posts and
/// rendering the feed in the requested format, then upserting the result
/// into the feed cache.
///
/// Every URL in the returned feed body is absolute, composed from the required
/// `site.base_url` via [`common::tagged_url::compose`] (#560): the feed self/canonical
/// URLs and each per-item permalink. `site.base_url` is a precondition — regeneration
/// errors with `RegenerateError::BaseUrlRequired` when it is unset, so no relative
/// `atom:id` is ever emitted.
///
/// # Errors
///
/// Returns `RegenerateError::BaseUrlRequired` if `site.base_url` is unset,
/// `RegenerateError::Storage` if any database operation fails.
/// (`RegenerateError::BadUrl` is retained as a defensive, never-hit guard: a
/// `FeedPath` argument is always parseable, so that arm cannot fire.)
pub async fn regenerate_feed(
    site_config: &dyn SiteConfigStorage,
    posts: &dyn PostStorage,
    feed_cache: &dyn FeedCacheStorage,
    feed_path: &FeedPath,
) -> Result<FeedCacheRow, RegenerateError> {
    // A `FeedPath` is always parseable, so this never yields `None`; `BadUrl` is
    // retained as a mapped (never-hit) error rather than an `expect()`/panic.
    let (surface, format) =
        parse(feed_path).ok_or_else(|| RegenerateError::BadUrl(feed_path.to_string()))?; // cov:ignore

    let feeds = site_config.get_feeds_config().await.map_err(storage_err)?;
    let identity = site_config.get_identity().await.map_err(storage_err)?;

    let window = HybridWindow {
        min_items: feeds.min_items,
        min_days: feeds.min_days,
    };
    let now = UtcInstant::now();
    let published = posts
        // Published feeds are public-only (M8 / ADR-0020): regeneration resolves
        // posts as an anonymous viewer, so the resolution filter reduces to the
        // `public` EXISTS and only Public posts reach the feed. Anonymous is the
        // permanent, correct value here — feeds have no authenticated viewer.
        .list_published_in_window(
            &surface,
            &window,
            now,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .map_err(storage_err)?;

    // `site.base_url` is required to compose absolute feed URLs (#560); this is the
    // single narrowing guard, so every downstream `compose` is infallible.
    let base = identity
        .base_url
        .as_ref()
        .ok_or(RegenerateError::BaseUrlRequired)?;

    let items = build_feed_items(base, &published);

    let self_url: FeedUrl = compose(base, feed_path);
    let canonical_path = match &surface {
        FeedSurface::Site => "/".to_owned(),
        // urlencoding::encode (external) takes &str.
        FeedSurface::SiteTag { tag } => format!("/tags/{}/", urlencoding::encode(tag.as_ref())),
        FeedSurface::User { username } => format!("/~{username}/"),
        FeedSurface::UserTag { username, tag } => {
            format!("/~{username}/tags/{}/", urlencoding::encode(tag.as_ref()))
        }
    };
    let canonical_url: CanonicalUrl = compose(base, &canonical_path);

    let updated_at = items
        .iter()
        .map(|i| i.updated_at)
        .max()
        .unwrap_or_else(|| now.value());
    let title = FeedTitle::for_surface(&identity.title, &surface);

    let meta = FeedMetadata {
        title,
        description: None,
        canonical_url,
        self_url,
        hub_url: feeds.websub_hub_url,
        updated_at,
    };

    let body = match format {
        FeedFormat::Rss => common::feed::render_rss(&meta, &items),
        FeedFormat::Atom => common::feed::render_atom(&meta, &items),
        FeedFormat::Json => common::feed::render_json(&meta, &items),
    };
    let etag = feed_etag(&items, now.value());

    let row = FeedCacheRow {
        feed_path: feed_path.clone(),
        body,
        etag,
        content_type: format.content_type(),
        updated_at: UtcInstant::from(updated_at),
        generated_at: now,
    };

    feed_cache.upsert(row.clone()).await.map_err(storage_err)?;

    Ok(row)
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
                permalink: compose::<Permalink>(base, &p.permalink()),
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

    #[test]
    fn regenerate_error_storage_preserves_sqlx_source() {
        use std::error::Error;
        // §3.1a: storage_err boxes the originating error as a typed source
        // (downcastable for classification) instead of stringifying it.
        let err = storage_err(sqlx::Error::RowNotFound);
        let source = err.source().expect("Storage should expose a source");
        assert!(source.downcast_ref::<sqlx::Error>().is_some());
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn regenerate_user_tag_feed_emits_typed_composed_title_and_base_anchored_url() {
        use common::site::SiteIdentity;

        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config.expect_get_feeds_config().returning(|| {
            Ok(common::feed::FeedsConfig {
                min_items: common::test_support::parse_feed_min_items("10"),
                min_days: common::test_support::parse_feed_min_days("30"),
                websub_hub_url: None,
            })
        });
        site_config.expect_get_identity().returning(|| {
            Ok(SiteIdentity {
                title: common::test_support::parse_site_title("Jaunder"),
                base_url: Some(common::test_support::parse_url("https://example.com/")),
            })
        });

        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_list_published_in_window()
            .returning(|_, _, _, _| Ok(vec![]));

        let mut feed_cache = storage::MockFeedCacheStorage::new();
        feed_cache.expect_upsert().returning(|_| Ok(()));

        let row = regenerate_feed(
            &site_config,
            &posts,
            &feed_cache,
            &"/~alice/tags/rust/feed.json"
                .parse::<FeedPath>()
                .expect("valid feed path"),
        )
        .await
        .expect("user-tag feed regenerates");

        let body: serde_json::Value = serde_json::from_str(&row.body).expect("JSON Feed body");
        assert_eq!(body["title"], "Jaunder — @alice #rust");
        assert_eq!(
            body["home_page_url"],
            "https://example.com/~alice/tags/rust/"
        );
    }

    #[tokio::test]
    async fn regenerate_without_base_url_errors() {
        use common::site::SiteIdentity;

        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config.expect_get_feeds_config().returning(|| {
            Ok(common::feed::FeedsConfig {
                min_items: common::test_support::parse_feed_min_items("10"),
                min_days: common::test_support::parse_feed_min_days("30"),
                websub_hub_url: None,
            })
        });
        // No base_url configured: regeneration cannot emit spec-valid absolute URLs, so it
        // errors rather than emitting relative ones (#560, D1 — no relative atom:id).
        site_config.expect_get_identity().returning(|| {
            Ok(SiteIdentity {
                title: common::test_support::parse_site_title("Jaunder"),
                base_url: None,
            })
        });

        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_list_published_in_window()
            .returning(|_, _, _, _| Ok(vec![]));

        let feed_cache = storage::MockFeedCacheStorage::new();

        let err = regenerate_feed(
            &site_config,
            &posts,
            &feed_cache,
            &"/feed.rss".parse::<FeedPath>().expect("valid feed path"),
        )
        .await
        .expect_err("regeneration without base_url must error");

        assert!(matches!(err, RegenerateError::BaseUrlRequired));
    }
}
