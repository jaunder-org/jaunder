#![cfg(feature = "server")]

use std::collections::BTreeSet;

use common::feed::affected_feed_urls;
use common::{tag::Tag, username::Username};
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
    username: &Username,
    tag_slugs: &BTreeSet<Tag>,
) -> Result<(), FeedEventError> {
    for url in affected_feed_urls(username, tag_slugs) {
        events.enqueue(&url).await?;
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
