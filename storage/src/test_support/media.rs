//! Media fixtures and inspection helpers: canonical media identities, seeded rows,
//! backup mutation, and raw/current-reference assertions. Database provisioning and
//! pool dispatch remain in [`super::backend`].
use super::confirmed_for;
#[cfg(any(test, feature = "test-utils"))]
use crate::MockSiteConfigStorage;
use crate::media::MediaRecord;
use crate::posts::media::{MediaReferenceEvidence, PersistedMediaReference};
use crate::sql::Exists;
use crate::sql::QueryStorageExt;
use crate::{
    DbConnectOptions, ForeignEvidenceSink, InstanceId, LocalMediaSink,
    MediaReferenceOwnershipResolver, MediaStorage, PostMediaOwnership, ProvenLocalMediaRefs,
    StorageRuntimeConfig, WriteScope, resolved_postgres_options,
};

use async_trait::async_trait;
use common::media::{Filename, MediaRef, MediaReference, MediaSource, detect_content_type, url};
#[cfg(any(test, feature = "test-utils"))]
use common::site::{SiteIdentity, SiteTitle};
use common::tagged_url::BaseUrl;
use common::test_support::{parse_byte_size, parse_content_hash};
use common::time::UtcInstant;
use sqlx::{PgPool, SqlitePool, postgres::PgConnectOptions, sqlite::SqliteConnectOptions};
use std::{fmt::Write as _, path::Path, sync::Arc};

/// Rewrites every row in a directory backup's `media.ndjson` to use `filename`.
///
/// # Panics
///
/// If the backup file cannot be read, parsed, serialized, or written.
pub fn rewrite_media_filename_in_backup(backup_path: &Path, filename: &str) {
    let media_ndjson = backup_path.join("db").join("media.ndjson");
    let mut rows: Vec<serde_json::Map<String, serde_json::Value>> =
        std::fs::read_to_string(&media_ndjson)
            .expect("read media backup")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .expect("parse media backup rows");
    for row in &mut rows {
        row.insert("filename".to_owned(), serde_json::json!(filename));
    }

    let mut rewritten = String::new();
    for row in rows {
        writeln!(
            rewritten,
            "{}",
            serde_json::to_string(&row).expect("serialize media row")
        )
        .expect("append media row");
    }
    std::fs::write(media_ndjson, rewritten).expect("write media backup");
}

const RAW_MEDIA_FILENAME_EXISTS_SQL: &str =
    "SELECT EXISTS(SELECT 1 FROM media WHERE filename = $1)";

/// A deliberately unvalidated filename used only to prove restore rejects a
/// corrupt media row before decode policy can admit it.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct RawMediaFilename(String);

impl RawMediaFilename {
    fn new(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Returns whether the live `media` table contains `filename` as its raw stored value.
///
/// # Panics
///
/// If connecting to the configured test database or querying `media` fails.
pub async fn raw_media_filename_exists(db: &DbConnectOptions, filename: &str) -> bool {
    let filename = RawMediaFilename::new(filename);
    match db {
        DbConnectOptions::Sqlite(options) => {
            raw_media_filename_exists_sqlite(options, &filename).await
        }
        DbConnectOptions::Postgres { options, .. } => {
            raw_media_filename_exists_postgres(options, &filename).await
        }
    }
}

async fn raw_media_filename_exists_sqlite(
    options: &SqliteConnectOptions,
    filename: &RawMediaFilename,
) -> bool {
    let pool = SqlitePool::connect_with(options.clone())
        .await
        .expect("connect sqlite");
    sqlx::query_scalar::<_, Exists>(RAW_MEDIA_FILENAME_EXISTS_SQL)
        .bind_storage(filename)
        .fetch_one(&pool)
        .await
        .expect("query sqlite media")
        .into_bool()
}

async fn raw_media_filename_exists_postgres(
    options: &PgConnectOptions,
    filename: &RawMediaFilename,
) -> bool {
    let options = resolved_postgres_options(options, &StorageRuntimeConfig::default());
    let pool = PgPool::connect_with(options)
        .await
        .expect("connect postgres");
    sqlx::query_scalar::<_, Exists>(RAW_MEDIA_FILENAME_EXISTS_SQL)
        .bind_storage(filename)
        .fetch_one(&pool)
        .await
        .expect("query postgres media")
        .into_bool()
}

/// The content hash every media fixture is stored under, re-exported from
/// [`common::test_support`] so `common`'s media-layout tests and this crate's fixtures
/// share one digest rather than restating it. Re-exported (rather than reached for
/// directly) because it is part of what a fixture caller expects from this module, next
/// to [`media_ref_for`]; public because a test spelling the `AtomPub` member layout
/// (`/atompub/<user>/media/<sha>/<name>`) needs the digest itself, not a serve URL.
pub use common::test_support::MEDIA_TEST_SHA256;

struct NoMediaOwnership;

#[async_trait]
impl MediaReferenceOwnershipResolver for NoMediaOwnership {
    // cov:ignore-start: this complete test resolver trait implementation is required, but the local-only fixture has no foreign-resolution caller.
    async fn resolve(
        &self,
        _references: &[PersistedMediaReference],
        _instance_id: &InstanceId,
        _base_url: Option<&BaseUrl>,
        foreign: ForeignEvidenceSink,
    ) -> MediaReferenceEvidence {
        foreign.finish()
    }
    // cov:ignore-stop

    async fn resolve_local(
        &self,
        _references: &[MediaReference],
        _instance_id: &InstanceId,
        _base_url: Option<&BaseUrl>,
        local: LocalMediaSink,
    ) -> ProvenLocalMediaRefs {
        local.finish()
    }
}

#[cfg(any(test, feature = "test-utils"))]
/// Returns a post-media ownership service that proves no rendered reference local.
///
/// # Panics
///
/// If the fixed canonical test instance ID stops parsing.
#[must_use]
pub fn fixture_post_media_ownership() -> PostMediaOwnership {
    let mut site_config = MockSiteConfigStorage::new();
    site_config.expect_get_identity().returning(|| {
        Ok(SiteIdentity {
            title: SiteTitle::default(),
            base_url: None,
        })
    });
    PostMediaOwnership::new(
        Arc::new(NoMediaOwnership),
        "123e4567-e89b-12d3-a456-426614174000"
            .parse()
            .expect("canonical test instance ID"),
        Arc::new(site_config),
    )
}

/// The [`MediaRef`] naming the fixture entry called `name`.
///
/// `name` is the **raw** name a person types; it goes through
/// [`Filename::sanitized`] — the upload-intake door — so a fixture spelling
/// `"my photo.jpg"` yields the stored `my%20photo.jpg` and a test never hand-encodes.
///
/// # Panics
///
/// If `name` is not a usable filename leaf.
#[must_use]
pub fn media_ref_for(name: &str) -> MediaRef {
    MediaRef {
        source: MediaSource::Upload,
        sha256: parse_content_hash(MEDIA_TEST_SHA256),
        filename: Filename::sanitized(name).expect("valid test media filename"),
    }
}

/// The canonical serve URL for `name` under the shared test digest — the single place
/// a test spells a media URL, composed by the production [`url`] rather than by
/// re-writing the layout.
#[must_use]
pub fn media_url_for(name: &str) -> String {
    let media = media_ref_for(name);
    url(&media.source, &media.sha256, &media.filename).to_string()
}

/// Seeds a `media` row owned by `user_id` for the fixture entry called `name`, and
/// returns the [`MediaRef`] naming it — the entry a post's `post_media` row resolves
/// to. Content type is derived from the name, as the real upload path derives it.
///
/// # Panics
///
/// If the row cannot be created — happy-path setup only, like [`SeedUser::seed`](super::SeedUser::seed).
pub async fn seed_media(
    media_store: Arc<dyn MediaStorage>,
    write_scope: WriteScope,
    user_id: common::ids::UserId,
    name: &str,
) -> MediaRef {
    let media = media_ref_for(name);
    let record = MediaRecord {
        user_id,
        sha256: media.sha256.clone(),
        filename: media.filename.clone(),
        source: media.source,
        content_type: detect_content_type(&media.filename),
        size_bytes: parse_byte_size("1"),
        source_url: None,
        created_at: UtcInstant::now(),
    };
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move { media_store.create_media(transaction, &record).await })
        })
        .await
        .expect("seed media should be created");
    confirmed_for(outcome, "seed media");
    media
}

/// Whether a `media` row exists for `user_id` and `media` — the ownership-scoped
/// existence question, asked through the real store rather than raw SQL.
///
/// # Panics
///
/// If the lookup fails.
pub async fn media_row_exists(
    media_store: Arc<dyn MediaStorage>,
    user_id: common::ids::UserId,
    media: &MediaRef,
) -> bool {
    media_store
        .get_media(user_id, &media.sha256, &media.filename, &media.source)
        .await
        .expect("media lookup should succeed")
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::{raw_media_filename_exists, seed_media};
    use crate::DbConnectOptions;
    use crate::test_support::{Backend, SeedUser, backends, recorded_postgres_url, sqlite_url};

    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn raw_media_filename_exists_reports_the_stored_existence_fact(#[case] backend: Backend) {
        let env = backend.setup().await;
        let db: DbConnectOptions = match backend {
            Backend::Sqlite => sqlite_url(&env.base),
            Backend::Postgres => recorded_postgres_url(&env.base)
                .parse()
                .expect("recorded Postgres URL parses"),
        };
        let filename = "exists.jpg";
        assert!(
            !raw_media_filename_exists(&db, filename).await,
            "a filename with no media row is absent"
        );
        let author = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await;
        seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            author.user_id,
            filename,
        )
        .await;
        assert!(
            raw_media_filename_exists(&db, filename).await,
            "a stored media row is present"
        );
    }
}
