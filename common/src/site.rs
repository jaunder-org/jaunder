use serde::{Deserialize, Serialize};

use crate::absolute_url::AbsoluteUrl;

/// Site-wide identity and configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteIdentity {
    /// Human-facing title for the site, used in feed metadata and similar contexts.
    pub title: String,
    /// Public-facing base URL (an absolute `http(s)` origin, normalized to its
    /// canonical form with a trailing slash), if set. When absent, callers emit
    /// root-relative URLs.
    pub base_url: Option<AbsoluteUrl>,
}

/// Default site title when no custom value is configured.
pub const DEFAULT_SITE_TITLE: &str = "Jaunder";
