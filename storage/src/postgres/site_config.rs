use sqlx::Postgres;

use crate::site_config::SiteConfigStore;

/// Postgres-backed site-config storage.
pub type PostgresSiteConfigStorage = SiteConfigStore<Postgres>;
