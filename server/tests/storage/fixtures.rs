use std::sync::Arc;

use common::tag::Tag;
use common::test_support::parse_row_limit;
use common::username::Username;
use common::visibility::ViewerIdentity;
use host::password::Password;
use sqlx::SqlitePool;
use storage::DbConnectOptions;
use tempfile::TempDir;

use storage::test_support::sqlite_url;

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
    posts: Arc<dyn storage::PostStorage>,
    tag: &Tag,
    limit: &str,
) -> Vec<storage::PostRecord> {
    posts
        .list_posts_by_tag(
            tag,
            storage::PublishedPageRequest {
                cursor: None,
                order: common::seed::TimelineOrder::Newest,
                limit: parse_row_limit(limit),
            },
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("list_posts_by_tag failed")
}

pub(super) async fn anon_published(
    posts: Arc<dyn storage::PostStorage>,
    limit: &str,
) -> Vec<storage::PostRecord> {
    posts
        .list_published(
            storage::PublishedPageRequest {
                cursor: None,
                order: common::seed::TimelineOrder::Newest,
                limit: parse_row_limit(limit),
            },
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
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

pub(super) fn username(s: &str) -> Username {
    s.parse().unwrap()
}

pub(super) fn password(s: &str) -> Password {
    s.parse().unwrap()
}
