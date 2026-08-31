use common::{
    feed::{FeedFormat, FeedSurface},
    tagged_url::{self, BaseUrl, CanonicalUrl, FeedUrl, Permalink},
    time::UtcInstant,
};
use host::etag;
use host::feed::{self, FeedItem, FeedMetadata, FeedPath, FeedTitle, HybridWindow};
use std::sync::Arc;
use storage::{
    FeedCacheRow, FeedCacheStorage, PostRecord, PostStorage, SiteConfigStorage, WriteScope,
    WriteScopeError,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegenerateError {
    #[error("unparseable feed_url: {0}")]
    BadUrl(String),
    #[error("site.base_url must be configured to regenerate feeds")]
    BaseUrlRequired,
    #[error("storage error: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("feed cache commit acknowledgement was indeterminate")]
    CacheCommitIndeterminate,
}

/// Regenerates a feed for the given URL by fetching published posts and
/// rendering the feed in the requested format, then upserting the result
/// through its own cache-write scope.
///
/// Every URL in the returned feed body is absolute, composed from the required
/// [`common::tagged_url::BaseUrl`] via [`common::tagged_url::compose`] (#560):
/// the feed self/canonical URLs and each per-item permalink. `site.base_url` is
/// a precondition — regeneration errors with [`RegenerateError::BaseUrlRequired`]
/// when it is unset, so no relative `atom:id` is ever emitted.
///
/// # Errors
///
/// Returns [`RegenerateError::BaseUrlRequired`] if `site.base_url` is unset,
/// [`RegenerateError::Storage`] if any read or cache write scope operation fails,
/// [`RegenerateError::CacheCommitIndeterminate`] if the cache write may have
/// committed but its acknowledgement was lost, or [`RegenerateError::BadUrl`] for
/// the defensive, never-hit parse guard.
pub async fn regenerate_feed(
    site_config: &dyn SiteConfigStorage,
    posts: &dyn PostStorage,
    feed_cache: Arc<dyn FeedCacheStorage>,
    write_scope: &WriteScope,
    feed_path: FeedPath,
) -> Result<FeedCacheRow, RegenerateError> {
    // A `FeedPath` is always parseable, so this never yields `None`; `BadUrl` is
    // retained as a mapped (never-hit) error rather than an `expect()`/panic.
    let (surface, format) =
        feed::parse(&feed_path).ok_or_else(|| RegenerateError::BadUrl(feed_path.to_string()))?; // cov:ignore

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

    let self_url: FeedUrl = tagged_url::compose(base, &feed_path);
    let canonical_path = match &surface {
        FeedSurface::Site => "/".to_owned(),
        // urlencoding::encode (external) takes &str.
        FeedSurface::SiteTag { tag } => format!("/tags/{}/", urlencoding::encode(tag.as_ref())),
        FeedSurface::User { username } => format!("/~{username}/"),
        FeedSurface::UserTag { username, tag } => {
            format!("/~{username}/tags/{}/", urlencoding::encode(tag.as_ref()))
        }
    };
    let canonical_url: CanonicalUrl = tagged_url::compose(base, &canonical_path);

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
        FeedFormat::Rss => feed::render_rss(&meta, &items),
        FeedFormat::Atom => feed::render_atom(&meta, &items),
        FeedFormat::Json => feed::render_json(&meta, &items),
    };
    let etag = etag::feed_etag(&items, now.value());

    let Ok(row) = FeedCacheRow::new(
        feed_path.clone(),
        body,
        etag,
        UtcInstant::from(updated_at),
        now,
    ) else {
        unreachable!("renderer output and feed path share the parsed format")
    };

    let row_for_upsert = row.clone();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move { feed_cache.upsert(transaction, row_for_upsert).await })
        })
        .await
        .map_err(regenerate_write_scope_error)?;
    if matches!(
        outcome,
        common::mutation::MutationOutcome::CommitIndeterminate(())
    ) {
        return Err(RegenerateError::CacheCommitIndeterminate);
    }

    Ok(row)
}

fn storage_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> RegenerateError {
    RegenerateError::Storage(Box::new(e))
}

fn regenerate_write_scope_error(
    error: WriteScopeError<storage::FeedCacheError>,
) -> RegenerateError {
    match error {
        WriteScopeError::Operation(error) => storage_err(error),
        WriteScopeError::Begin(error) => storage_err(error),
    }
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

    #[test]
    fn regenerate_error_storage_preserves_sqlx_source() {
        use std::error::Error;
        // §3.1a: storage_err boxes the originating error as a typed source
        // (downcastable for classification) instead of stringifying it.
        let err = storage_err(sqlx::Error::RowNotFound);
        let source = err.source().expect("Storage should expose a source");
        assert!(source.downcast_ref::<sqlx::Error>().is_some());
    }

    #[test]
    fn regenerate_write_scope_operation_preserves_cache_sqlx_source() {
        use std::error::Error;

        let error = regenerate_write_scope_error(WriteScopeError::Operation(
            storage::FeedCacheError::Db(sqlx::Error::RowNotFound),
        ));

        let RegenerateError::Storage(source) = &error else {
            panic!("write operation errors must map to RegenerateError::Storage");
        };
        let cache = source
            .downcast_ref::<storage::FeedCacheError>()
            .expect("storage source should retain the cache error");
        assert!(matches!(
            cache
                .source()
                .and_then(|source| source.downcast_ref::<sqlx::Error>()),
            Some(sqlx::Error::RowNotFound)
        ));
    }

    #[test]
    fn regenerate_write_scope_begin_preserves_sqlx_source() {
        let error = regenerate_write_scope_error(WriteScopeError::Begin(sqlx::Error::PoolTimedOut));

        let RegenerateError::Storage(source) = &error else {
            panic!("write scope begin errors must map to RegenerateError::Storage");
        };
        assert!(matches!(
            source.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolTimedOut)
        ));
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn regenerate_user_tag_feed_emits_typed_composed_title_and_base_anchored_url() {
        use common::site::SiteIdentity;

        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config.expect_get_feeds_config().returning(|| {
            Ok(host::feed::FeedsConfig {
                min_items: host::test_support::parse_feed_min_items("10"),
                min_days: host::test_support::parse_feed_min_days("30"),
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
        feed_cache.expect_upsert().returning(|_, _| Ok(()));
        let feed_cache: Arc<dyn FeedCacheStorage> = Arc::new(feed_cache);
        let feed_path = "/~alice/tags/rust/feed.json"
            .parse::<FeedPath>()
            .expect("valid feed path");

        let row = regenerate_feed(
            &site_config,
            &posts,
            feed_cache,
            &storage::test_support::mock_write_scope(),
            feed_path,
        )
        .await
        .expect("user-tag feed regenerates");

        let body: serde_json::Value =
            serde_json::from_str(row.representation().body()).expect("JSON Feed body");
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
            Ok(host::feed::FeedsConfig {
                min_items: host::test_support::parse_feed_min_items("10"),
                min_days: host::test_support::parse_feed_min_days("30"),
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
        let feed_cache: Arc<dyn FeedCacheStorage> = Arc::new(feed_cache);
        let feed_path = "/feed.rss".parse::<FeedPath>().expect("valid feed path");

        let err = regenerate_feed(
            &site_config,
            &posts,
            feed_cache,
            &storage::test_support::mock_write_scope(),
            feed_path,
        )
        .await
        .expect_err("regeneration without base_url must error");

        assert!(matches!(err, RegenerateError::BaseUrlRequired));
    }
}
