//! Persistence layer for Jaunder.

// `mockall`'s `#[automock]` generates matcher code taking `&Option<&T>` for the
// traits with `Option<&…Cursor>`/`Option<&str>` args (PostStorage/UserStorage/
// MediaStorage), tripping `clippy::ref_option_ref` under `-D warnings`. The generated
// `Mock*` structs are module-level siblings of the traits, so the allow is scoped at
// the crate root and gated to the same `any(test, feature = "test-utils")` as the mocks
// (`storage`'s own `cfg(test)` build now uses them too, #517). No production code uses
// `&Option<&T>`, so nothing genuine is masked (#245).
#![cfg_attr(any(test, feature = "test-utils"), allow(clippy::ref_option_ref))]

mod app_state;
mod atomic;
mod audiences;
mod auth;
mod backend;
mod backup;
mod db;
mod email;
mod feed_cache;
mod feed_events;
mod helpers;
mod invites;
mod media;
mod media_manager;
mod password;
mod post_service;
mod postgres;
mod posts;
mod sessions;
mod site_config;
mod smtp;
pub(crate) mod sql;
mod sqlite;
mod subscriptions;
mod user_config;
mod users;

// Both-backend test harness (ADR-0033): available to `storage`'s own tests via
// `cfg(test)` and to external test crates (`server`) via the `test-support`
// feature.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use app_state::*;
pub use atomic::*;
pub use audiences::*;
pub use auth::*;
pub use backend::*;
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
pub use media_manager::{MediaError, MediaManager};
pub use password::*;
pub use post_service::*;
pub use postgres::{
    create_postgres_database_and_role, resolved_postgres_options, PgBootstrapError,
    PostgresAtomicOps, PostgresAudienceStorage, PostgresEmailVerificationStorage,
    PostgresFeedCacheStorage, PostgresFeedEventStorage, PostgresInviteStorage,
    PostgresMediaStorage, PostgresPasswordResetStorage, PostgresPostStorage,
    PostgresSessionStorage, PostgresSiteConfigStorage, PostgresSubscriptionStorage,
    PostgresUserConfigStorage, PostgresUserStorage,
};
pub use posts::*;
pub use sessions::*;
pub use site_config::*;
pub use smtp::*;
pub use sqlite::{
    SqliteAtomicOps, SqliteAudienceStorage, SqliteEmailVerificationStorage, SqliteFeedCacheStorage,
    SqliteFeedEventStorage, SqliteInviteStorage, SqliteMediaStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteSubscriptionStorage,
    SqliteUserConfigStorage, SqliteUserStorage,
};
pub use subscriptions::*;
pub use user_config::*;
pub use users::*;
