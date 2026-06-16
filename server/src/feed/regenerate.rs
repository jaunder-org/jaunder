use chrono::Utc;
use common::feed::{
    feed_etag, parse, FeedFormat, FeedItem, FeedMetadata, FeedSurface, HybridWindow,
};
use storage::{AppState, FeedCacheRow, PostRecord};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegenerateError {
    #[error("unparseable feed_url: {0}")]
    BadUrl(String),
    #[error("storage error: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Regenerates a feed for the given URL by fetching published posts and
/// rendering the feed in the requested format, then upserting the result
/// into the feed cache.
///
/// URLs in the returned feed body are relative (root-anchored paths) —
/// matching the project's convention for outgoing links. A reverse proxy
/// or feed reader is expected to resolve them against the public origin.
///
/// # Errors
///
/// Returns `RegenerateError::BadUrl` if `feed_url` cannot be parsed,
/// or `RegenerateError::Storage` if any database operation fails.
pub async fn regenerate_feed(
    state: &AppState,
    feed_url: &str,
) -> Result<FeedCacheRow, RegenerateError> {
    let (surface, format) =
        parse(feed_url).ok_or_else(|| RegenerateError::BadUrl(feed_url.into()))?;

    let feeds = state
        .site_config
        .get_feeds_config()
        .await
        .map_err(storage_err)?;
    let identity = state
        .site_config
        .get_identity()
        .await
        .map_err(storage_err)?;

    let window = HybridWindow {
        min_items: feeds.min_items,
        min_days: feeds.min_days,
    };
    let now = Utc::now();
    let posts = state
        .posts
        .list_published_in_window(&surface, &window, now)
        .await
        .map_err(storage_err)?;

    let items = build_feed_items(state, &posts).await?;

    let base = identity.base_url.as_deref().unwrap_or("");
    let self_url = format!("{base}{}", percent_encode_path(feed_url));
    let canonical_url = match &surface {
        FeedSurface::Site => format!("{base}/"),
        FeedSurface::SiteTag { tag } => {
            format!("{base}/tags/{}/", urlencoding::encode(tag.as_str()))
        }
        FeedSurface::User { username } => format!("{base}/~{username}/"),
        FeedSurface::UserTag { username, tag } => {
            format!(
                "{base}/~{username}/tags/{}/",
                urlencoding::encode(tag.as_str())
            )
        }
    };

    let updated_at = items.iter().map(|i| i.updated_at).max().unwrap_or(now);
    let title = compute_title(&identity.title, &surface);

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
    let etag = feed_etag(&items, now);

    let row = FeedCacheRow {
        feed_url: feed_url.to_string(),
        body,
        etag,
        content_type: format.content_type().to_string(),
        updated_at,
        generated_at: now,
    };

    state
        .feed_cache
        .upsert(row.clone())
        .await
        .map_err(storage_err)?;

    Ok(row)
}

fn storage_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> RegenerateError {
    RegenerateError::Storage(Box::new(e))
}

fn percent_encode_path(path: &str) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
    const PATH_RESERVED: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'<')
        .add(b'>')
        .add(b'`')
        .add(b'#')
        .add(b'?');
    utf8_percent_encode(path, PATH_RESERVED).to_string()
}

async fn build_feed_items(
    state: &AppState,
    posts: &[PostRecord],
) -> Result<Vec<FeedItem>, RegenerateError> {
    let mut items = Vec::with_capacity(posts.len());
    for p in posts {
        let tags = state
            .posts
            .get_tags_for_post(p.post_id)
            .await
            .map_err(storage_err)?;
        // list_published_in_window guarantees published_at IS NOT NULL,
        // but we fall back to created_at rather than panic if the
        // invariant is ever violated (matches PostRecord::permalink).
        let published_at = p.published_at.unwrap_or(p.created_at);
        items.push(FeedItem {
            id: p.post_id,
            title: p.title.clone(),
            permalink: p.permalink(),
            summary: p.summary.clone(),
            content_html: p.rendered_html.clone(),
            published_at,
            updated_at: p.updated_at,
            tags: tags.iter().map(|t| t.tag_display.clone()).collect(),
        });
    }
    Ok(items)
}

fn compute_title(site_title: &str, surface: &FeedSurface) -> String {
    match surface {
        FeedSurface::Site => site_title.to_string(),
        FeedSurface::SiteTag { tag } => format!("{site_title} — #{tag}"),
        FeedSurface::User { username } => format!("{site_title} — @{username}"),
        FeedSurface::UserTag { username, tag } => {
            format!("{site_title} — @{username} #{tag}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encode_path_encodes_query_marker() {
        let encoded = percent_encode_path("/feed.rss?key=value");
        assert!(encoded.contains("%3F"));
    }

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
    fn compute_title_for_each_surface() {
        assert_eq!(compute_title("Jaunder", &FeedSurface::Site), "Jaunder");
        let site_tag = compute_title(
            "Jaunder",
            &FeedSurface::SiteTag {
                tag: "rust".parse().unwrap(),
            },
        );
        assert!(site_tag.contains("rust"));
        let user = compute_title(
            "My Blog",
            &FeedSurface::User {
                username: "alice".parse().unwrap(),
            },
        );
        assert!(user.contains("My Blog") && user.contains("alice"));
        let user_tag = compute_title(
            "Jaunder",
            &FeedSurface::UserTag {
                username: "alice".parse().unwrap(),
                tag: "rust".parse().unwrap(),
            },
        );
        assert!(user_tag.contains("alice") && user_tag.contains("rust"));
    }
}
