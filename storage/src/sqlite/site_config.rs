use sqlx::Sqlite;

use crate::site_config::SiteConfigStore;

/// SQLite-backed site-config storage.
pub type SqliteSiteConfigStorage = SiteConfigStore<Sqlite>;
