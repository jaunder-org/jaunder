use chrono::Utc;
use common::ids::ChannelId;
use common::password::Password;
use common::tag::Tag;
use common::test_support::parse_row_limit;
use common::username::Username;
use common::visibility::ViewerIdentity;
use sqlx::SqlitePool;
use storage::{AppState, DbConnectOptions};
use tempfile::TempDir;

use storage::test_support::{Backend, TestEnv, sqlite_url};

// ── Anonymous-viewer listing helpers ─────────────────────────────────────────
//
// 51 listing calls in this storage suite pass the same five arguments — no cursor,
// `&ViewerIdentity::Anonymous`, `Utc::now()` — and differ only in what they list and
// how many rows they want. Spelling all five out per call site buried the one or two
// that actually vary; #696 made it visible, because typing the limit pushed every such
// call past the line width and rustfmt exploded each into seven lines.
//
// These return the rows directly rather than the `Result`: the few tests that assert on
// an *error* call the store directly, and that difference is the point — a call that
// goes through a helper is one that expects rows.
pub(super) async fn anon_by_tag(
    state: &AppState,
    tag: &Tag,
    limit: &str,
) -> Vec<storage::PostRecord> {
    state
        .posts
        .list_posts_by_tag(
            tag,
            None,
            parse_row_limit(limit),
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .expect("list_posts_by_tag failed")
}

pub(super) async fn anon_published(state: &AppState, limit: &str) -> Vec<storage::PostRecord> {
    state
        .posts
        .list_published(
            None,
            parse_row_limit(limit),
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .expect("list_published failed")
}

pub(super) async fn open_pool(base: &TempDir) -> SqlitePool {
    let DbConnectOptions::Sqlite(opts) = sqlite_url(base) else {
        panic!("expected sqlite options");
    };
    let pool = SqlitePool::connect_with(opts.create_if_missing(true))
        .await
        .unwrap();
    sqlx::migrate!("../storage/migrations/sqlite")
        .run(&pool)
        .await
        .unwrap();
    pool
}

// Sibling of `lookup_names`: a raw SELECT of the seeded `local` channel id.
// The `local` channel is a lookup row present in every clone, so reading it via
// the per-test recorded URL (Postgres) or the same DB file (SQLite) both work;
// we use the established same-DB helpers for consistency — deliberately not the
// trait method `local_channel_id()`, which is what the test below asserts
// against, so it cannot also be the source of the expectation.
//
// Reads a `channels` row's id by name, on the FK-enabled pool for `backend`.
// Generalizes `local_channel_id` so a test can also reach a channel it seeded
// itself (e.g. the non-local `activitypub` row the impostor viewer sits on).
pub(super) async fn local_channel_id(backend: Backend, env: &TestEnv) -> ChannelId {
    channel_id_by_name(backend, env, "local").await
}

pub(super) async fn channel_id_by_name(backend: Backend, env: &TestEnv, name: &str) -> ChannelId {
    let sql = format!("SELECT channel_id FROM channels WHERE name = '{name}'");
    let sql = sql.as_str();
    match backend {
        Backend::Sqlite => sqlx::query_scalar::<_, ChannelId>(sql)
            .fetch_one(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => sqlx::query_scalar::<_, ChannelId>(sql)
            .fetch_one(env.base.pool().postgres())
            .await
            .unwrap(),
    }
}

pub(super) fn username(s: &str) -> Username {
    s.parse().unwrap()
}

pub(super) fn password(s: &str) -> Password {
    s.parse().unwrap()
}

// Run a statement on the FK-enabled pool for `backend`. This small per-backend
// helper mirrors `open_pool`/`open_pg_pool`: it unwraps. Inlining integer ids via
// `format!` is safe here (test-only, no untrusted input) and sidesteps the
// SQLite/Postgres placeholder divergence.
pub(super) async fn raw_exec(backend: Backend, env: &TestEnv, sql: &str) {
    let result = match backend {
        Backend::Sqlite => sqlx::query(sql)
            .execute(&open_pool(&env.base).await)
            .await
            .map(|_| ()),
        Backend::Postgres => sqlx::query(sql)
            .execute(env.base.pool().postgres())
            .await
            .map(|_| ()),
    };
    result.unwrap_or_else(|e| panic!("raw exec failed: {e}\nSQL: {sql}"));
}
