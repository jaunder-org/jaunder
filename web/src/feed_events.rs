#![cfg(feature = "ssr")]

use std::collections::BTreeSet;

use common::feed::{canonicalize, FeedFormat, FeedSurface};
use storage::{FeedEventError, FeedEventStorage};

#[cfg(test)]
use storage::PostTag;

/// Extract tag slugs from a slice of `PostTag` records, returning them as a
/// deduplicated, sorted set of lowercased slug strings.
#[cfg(test)]
fn tag_slugs(tags: &[PostTag]) -> BTreeSet<String> {
    tags.iter()
        .map(|t| t.tag_slug.as_str().to_string())
        .collect()
}

/// Enqueue feed-regeneration events for every feed surface a post mutation
/// could affect: the site feed, the author's feed, and the site-/user-tag
/// feeds for the union of old and new tag slugs. Three rows per surface
/// (one per format).
///
/// # Errors
///
/// Returns an error if the feed event storage operation fails.
pub async fn enqueue_feed_events(
    events: &dyn FeedEventStorage,
    username: &str,
    tag_slugs: &BTreeSet<String>,
) -> Result<(), FeedEventError> {
    let mut surfaces = vec![
        FeedSurface::Site,
        FeedSurface::User {
            username: username.to_string(),
        },
    ];
    for tag in tag_slugs {
        surfaces.push(FeedSurface::SiteTag { tag: tag.clone() });
        surfaces.push(FeedSurface::UserTag {
            username: username.to_string(),
            tag: tag.clone(),
        });
    }
    for surface in &surfaces {
        for format in [FeedFormat::Rss, FeedFormat::Atom, FeedFormat::Json] {
            events.enqueue(&canonicalize(surface, format)).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_slugs_deduplicates_and_sorts() {
        let tags = vec![
            PostTag {
                post_id: 1,
                tag_id: 1,
                tag_slug: "web".parse().unwrap(),
                tag_display: "Web".to_string(),
            },
            PostTag {
                post_id: 1,
                tag_id: 2,
                tag_slug: "rust".parse().unwrap(),
                tag_display: "Rust".to_string(),
            },
            PostTag {
                post_id: 1,
                tag_id: 1,
                tag_slug: "web".parse().unwrap(),
                tag_display: "Web".to_string(),
            },
        ];
        let slugs = tag_slugs(&tags);
        let mut sorted: Vec<_> = slugs.into_iter().collect();
        sorted.sort();
        assert_eq!(sorted, vec!["rust".to_string(), "web".to_string()]);
    }

    #[test]
    fn test_tag_slugs_empty() {
        let tags: Vec<PostTag> = vec![];
        let slugs = tag_slugs(&tags);
        assert!(slugs.is_empty());
    }
}
