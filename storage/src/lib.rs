//! Persistence layer for Jaunder.

mod app_state;
mod atomic;
mod auth;
mod backup;
mod db;
mod email;
mod feed_cache;
mod feed_events;
mod helpers;
mod invites;
mod media;
mod password;
mod post_service;
mod postgres;
mod posts;
mod sessions;
mod site_config;
mod smtp;
mod sqlite;
mod user_config;
mod users;

pub use app_state::*;
pub use atomic::*;
pub use auth::*;
pub use backup::{
    export_backup, restore_backup, BackupError, BackupExportOptions, BackupManifest, BackupMode,
    BackupRestoreOptions,
};
pub use db::*;
pub use email::*;
pub use feed_cache::*;
pub use feed_events::*;
pub use invites::*;
pub use media::*;
pub use password::*;
pub use post_service::*;
pub use postgres::{
    create_postgres_database_and_role, resolved_postgres_options, PgBootstrapError,
    PostgresAtomicOps, PostgresEmailVerificationStorage, PostgresFeedCacheStorage,
    PostgresFeedEventStorage, PostgresInviteStorage, PostgresMediaStorage,
    PostgresPasswordResetStorage, PostgresPostStorage, PostgresSessionStorage,
    PostgresSiteConfigStorage, PostgresUserConfigStorage, PostgresUserStorage,
};
pub use posts::*;
pub use sessions::*;
pub use site_config::*;
pub use smtp::*;
pub use sqlite::{
    SqliteAtomicOps, SqliteEmailVerificationStorage, SqliteFeedCacheStorage,
    SqliteFeedEventStorage, SqliteInviteStorage, SqliteMediaStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserConfigStorage,
    SqliteUserStorage,
};
pub use user_config::*;
pub use users::*;

// `RenderError` lives in `common::render` but is part of storage's public
// surface: `web` and `server` match on it via the `Perform*Error::Render`
// arms, so re-export it here to keep the `storage::RenderError` path stable.
pub use common::render::RenderError;
