//! Persistence layer for Jaunder.

// `mockall`'s `#[automock]` generates matcher code taking `&Option<&T>` for the
// traits with `Option<&…Cursor>`/`Option<&str>` args (PostStorage/UserStorage/
// MediaStorage), tripping `clippy::ref_option_ref` under `-D warnings`. The generated
// `Mock*` structs are module-level siblings of the traits, so the allow is scoped at
// the crate root and gated to the same `any(test, feature = "test-utils")` as the mocks
// (`storage`'s own `cfg(test)` build now uses them too, #517). No production code uses
// `&Option<&T>`, so nothing genuine is masked (#245).
// lint-suppression:allow approved in #294; cfg-gated mockall-generated matcher signature suppression
#![cfg_attr(any(test, feature = "test-utils"), expect(clippy::ref_option_ref))]

pub mod account_mutations;
mod app_state;
mod audiences;
mod backend;
mod backup;
mod db;
mod email;
mod error;
mod feed_cache;
mod feed_events;
mod helpers;
mod instance_identity;
mod invites;
mod media;
mod media_content_locks;
mod media_manager;
mod media_ownership;
#[cfg(test)]
mod migrations;
mod password;
mod post_service;
mod postgres;
mod posts;
mod role_instant;
mod sessions;
mod site_config;
mod smtp;
pub mod sql;
mod sqlite;
mod subscriptions;
mod user_config;
mod users;
mod write_scope;

// Both-backend test harness (ADR-0033): available to `storage`'s own tests via
// `cfg(test)` and to external test crates (`server`) via the `test-support`
// feature.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use app_state::*;
pub use audiences::*;
pub use backend::*;
pub use backup::{
    BackupError, BackupExportOptions, BackupManifest, BackupMode, BackupRestoreOptions,
    BackupRestoreOutcome, RestoreValidationIssue, RestoreValidationReport, export_backup,
    restore_backup,
};
pub use db::*;
pub use email::*;
pub use error::{MissingRow, RequireRow};
pub use feed_cache::*;
pub use feed_events::*;
pub use instance_identity::*;
pub use invites::*;
pub use media::*;
pub use media_content_locks::MediaContentLocks;
pub use media_manager::{
    MediaDeletionResult, MediaError, MediaManager, MediaTemporaryDirectoryError,
};
pub use media_ownership::*;
pub use password::*;
pub use post_service::*;
pub use postgres::{
    PgBootstrapError, PostgresAudienceStorage, PostgresEmailVerificationStorage,
    PostgresFeedCacheStorage, PostgresFeedEventStorage, PostgresInviteStorage,
    PostgresMediaStorage, PostgresPasswordResetStorage, PostgresPostStorage,
    PostgresSessionStorage, PostgresSiteConfigStorage, PostgresSubscriptionStorage,
    PostgresUserConfigStorage, PostgresUserStorage, create_postgres_database_and_role,
    resolved_postgres_options,
};
pub use posts::*;
pub use sessions::*;
pub use site_config::*;
pub use smtp::*;
pub use sqlite::{
    SqliteAudienceStorage, SqliteEmailVerificationStorage, SqliteFeedCacheStorage,
    SqliteFeedEventStorage, SqliteInviteStorage, SqliteMediaStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteSubscriptionStorage,
    SqliteUserConfigStorage, SqliteUserStorage,
};
pub use subscriptions::*;
pub use user_config::*;
pub use users::*;
pub use write_scope::*;
