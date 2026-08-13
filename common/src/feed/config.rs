use crate::feed::{FeedMinDays, FeedMinItems};
use crate::tagged_url::HubUrl;

/// Aggregate of the feed-generation settings stored in `site_config`
/// (`feeds.min_items`, `feeds.min_days`, `feeds.websub_hub_url`). Mirrors
/// [`crate::backup::BackupConfig`] so feed settings have a single grouped
/// getter/setter rather than per-key read chains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedsConfig {
    pub min_items: FeedMinItems,
    pub min_days: FeedMinDays,
    pub websub_hub_url: Option<HubUrl>,
}
