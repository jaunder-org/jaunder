//! Both-backend test harness for the `storage` crate's own tests and `server`'s
//! integration tests. Lives in `storage` (gated by the `test-support` feature) so
//! `storage`'s in-file tests use it from the same crate instance — avoiding the
//! two-`storage`-instances problem a separate crate would create (see ADR-0033).
//! `server` reaches it via `storage`'s `test-support` feature.

// Deliberately unwrap/expect-heavy test scaffolding (test-support feature, ADR-0033),
// so the workspace's `unwrap_used`/`expect_used = deny` lints are expected off for this
// module; `#[expect]` self-removes if the scaffolding ever stops unwrapping. Everything
// else clippy-pedantic flags is fixed in place rather than suppressed. (#94)
// lint-suppression:allow approved in #294; existing expectation documents intentional test-scaffolding or naming exception
#![expect(clippy::unwrap_used, clippy::expect_used)]

mod backend;
mod feed_cache;
mod feeds;
mod mail;
mod media;
mod post_service;
mod postgres;
mod posts;
mod subscriptions;
mod users;

#[cfg(any(test, feature = "test-utils"))]
pub use backend::mock_write_scope;
pub use backend::{
    Backend, CloseablePool, MediaReferenceWriteLock, PostWriteLock, SetupBuilder, TestBase,
    TestEnv, backends, backends_matrix, confirmed, confirmed_for, fixture_media_content_locks,
    inject_invalid_site_config, postgres_only, set_post_tags_confirmed, set_site_config,
    sqlite_only, sqlite_url, sqlite_write_scope,
};

pub use feed_cache::SeedFeedCache;
pub use feeds::fp;
pub use mail::noop_mailer;
pub(crate) use media::RawMediaFilename;
pub use media::{
    MEDIA_TEST_SHA256, fetch_post_media, media_ref_for, media_row_exists, media_url_for,
    raw_media_filename_exists, rewrite_media_filename_in_backup, seed_media,
};
pub use post_service::{
    create_draft_via_service, create_post_via_service, update_post_body_via_service,
};
pub use postgres::{
    PG_URL_FILE, PostgresDbGuard, PostgresTestConfig, nonexistent_postgres_url,
    recorded_postgres_url, template_postgres_url, unique_postgres_url,
};
pub(crate) use postgres::{TemplateDatabaseLockKey, TemplateDatabaseName};
pub use posts::{
    SeedPost, SeedRawPost, SeededPost, UpdateRawPost, create_posts_confirmed, seed_posts,
};
pub use subscriptions::seed_local_subscription;
pub use users::{SeedUser, SeededUser, seed_users};
