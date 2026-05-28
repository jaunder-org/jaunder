use serde::{Deserialize, Serialize};

/// Site-wide identity and configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteIdentity {
    /// Human-facing title for the site, used in feed metadata and similar contexts.
    pub title: String,
    /// Public-facing base URL (scheme + host, no trailing slash), if set.
    /// When absent, callers should emit root-relative URLs.
    pub base_url: Option<String>,
}

/// Default site title when no custom value is configured.
pub const DEFAULT_SITE_TITLE: &str = "Jaunder";
