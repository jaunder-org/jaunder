pub mod feed_path;
pub use feed_path::{
    affected_feed_urls, canonicalize, parse, FeedFormat, FeedPath, FeedSurface, InvalidFeedPath,
};

pub mod settings;
pub use settings::{FeedMinDays, FeedMinItems};

pub mod window;
pub use window::{HasPublishedAt, HybridWindow};

pub mod metadata;
pub use metadata::{feed_etag, FeedItem, FeedMetadata};

pub mod rss;
pub use rss::render_rss;

pub mod atom;
pub use atom::render_atom;

pub mod json;
pub use json::render_json;

/// Aggregate of the feed-generation settings stored in `site_config`
/// (`feeds.min_items`, `feeds.min_days`, `feeds.websub_hub_url`). Mirrors
/// [`crate::backup::BackupConfig`] so feed settings have a single grouped
/// getter/setter rather than per-key read chains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedsConfig {
    pub min_items: FeedMinItems,
    pub min_days: FeedMinDays,
    pub websub_hub_url: Option<String>,
}
