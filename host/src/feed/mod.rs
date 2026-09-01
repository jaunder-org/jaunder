//! Host-owned Syndication Feed identities, policy, models, and serializers.
//!
//! The dual-target representation/surface grammar remains in `common::feed` so
//! CSR discovery and host producers build identical canonical URLs. This module
//! owns everything that requires host storage, scheduling, or serialization.

#[cfg(test)]
mod test_support;

pub mod feed_path;
pub use feed_path::{FeedPath, InvalidFeedPath, affected_feed_urls, parse};

pub mod event_status;
pub use event_status::{FeedEventStatus, InvalidFeedEventStatus};

pub mod settings;
pub use settings::{FeedEventClaimLimit, FeedMinDays, FeedMinItems};

pub mod window;
pub use window::{HasPublishedAt, HybridWindow};

pub mod metadata;
pub use metadata::{FeedDescription, FeedItem, FeedMetadata, FeedTitle};

pub mod representation;
pub use representation::{MismatchedStoredSyndicationFeedMetadata, SyndicationFeedRepresentation};

pub mod rss;
pub use rss::render_rss;

pub mod atom;
pub use atom::render_atom;

pub mod json;
pub use json::render_json;

pub mod config;
pub use config::FeedsConfig;
