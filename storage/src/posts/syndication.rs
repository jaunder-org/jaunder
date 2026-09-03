//! Syndication-window reads and feed catch-up projections for posts.

use sqlx::{Encode, Executor, Pool, Result, Row, Type};

use crate::posts::models::PostRecord;
use crate::posts::store::PostDialect;
use crate::posts::visibility;
use crate::sql::QueryStorageExt;
use common::ids::{ChannelId, UserId};
use common::tag::Tag;
use common::time::UtcInstant;
use common::username::Username;
use host::feed::{FeedMinItems, FeedPath};

/// Resolves whether a Post participates in the anonymous/Public projection used
/// by Syndication Feed due-time discovery.
const PUBLIC_AUDIENCE_PREDICATE: &str = "EXISTS (
    SELECT 1 FROM post_audiences pa
    JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
    WHERE pa.post_id = p.post_id AND tk.name = 'public'
)";

/// A post that crossed into "live" within a time window, carrying exactly the
/// data the feed worker needs to compute its affected feed URLs (the author's
/// username and the post's tag slugs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoLivePost {
    pub username: Username,
    pub tag_slugs: Vec<Tag>,
}

/// Runs the hybrid-window query for `surface`, returning [`PostRecord`]s.
///
/// Shared across backends: the four `FeedSurface` variants differ only in the
/// ranked-CTE source/predicate and bind list, and the JSON tag aggregation is
/// supplied by [`PostDialect::TAGS_SUBQUERY`].
pub(crate) async fn list_published_in_window_rows<DB>(
    pool: &Pool<DB>,
    surface: &common::feed::FeedSurface,
    now: UtcInstant,
    cutoff: UtcInstant,
    min_items: FeedMinItems,
    viewer: &common::visibility::ViewerIdentity,
) -> Result<Vec<PostRecord>>
where
    DB: PostDialect,
    PostRecord: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> FeedMinItems: Encode<'q, DB> + Type<DB>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    // The viewer-resolution binds are NULL-able (`ResolutionBinds::bind_onto`).
    for<'q> Option<UserId>: Encode<'q, DB> + Type<DB>,
    for<'q> Option<ChannelId>: Encode<'q, DB> + Type<DB>,
    for<'q> &'q common::visibility::SubscriberRef: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q common::visibility::SubscriberRef>: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q str>: Encode<'q, DB> + Type<DB>,
    // `Username`/`Tag` bind as themselves via the ADR-0071 sqlx bridge, for the
    // surface `username`/`tag` binds.
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    use common::feed::FeedSurface;
    let tags = DB::TAGS_SUBQUERY;
    match surface {
        FeedSurface::Site => {
            // Binds: $1 now, $2 min_items, $3 cutoff, then the variant-sized
            // resolution fragment from $4. It filters the ranked CTE and still
            // uses the last placeholders, so the returned `next` is discarded.
            let (resolution, binds, _) = visibility::resolution_where(viewer, 4);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind_storage(now)
                .bind_storage(min_items)
                .bind_storage(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::User { username } => {
            // Binds: $1 now, $2 username, $3 min_items, $4 cutoff, then the
            // variant-sized ranked-CTE resolution fragment from $5.
            let (resolution, binds, _) = visibility::resolution_where(viewer, 5);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind_storage(now)
                .bind_storage(username)
                .bind_storage(min_items)
                .bind_storage(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::SiteTag { tag } => {
            // Binds: $1 now, $2 tag, $3 min_items, $4 cutoff, then the
            // variant-sized ranked-CTE resolution fragment from $5.
            let (resolution, binds, _) = visibility::resolution_where(viewer, 5);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind_storage(now)
                .bind_storage(tag)
                .bind_storage(min_items)
                .bind_storage(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::UserTag { username, tag } => {
            // Binds: $1 now, $2 username, $3 tag, $4 min_items, $5 cutoff, then
            // the variant-sized ranked-CTE resolution fragment from $6.
            let (resolution, binds, _) = visibility::resolution_where(viewer, 6);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind_storage(now)
                .bind_storage(username)
                .bind_storage(tag)
                .bind_storage(min_items)
                .bind_storage(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
    }
}

/// Assembles the hybrid-window SQL for `surface`.
///
/// Pure string construction with no DB generics: the four near-identical
/// templates — differing only in the ranked-CTE source/predicate and bind
/// placeholders — live here, while [`list_published_in_window_rows`] keeps the
/// generic `where`-clause, per-surface bind list, and execution. `tags` supplies
/// the JSON tag aggregation ([`PostDialect::TAGS_SUBQUERY`]) and `resolution` the
/// audience-resolution predicate.
fn window_sql(surface: &common::feed::FeedSurface, tags: &str, resolution: &str) -> String {
    use common::feed::FeedSurface;
    match surface {
        FeedSurface::Site => format!(
            "WITH ranked AS (
     SELECT p.post_id, p.published_at,
            ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS rn
     FROM posts p
     WHERE p.published_at IS NOT NULL
       AND p.deleted_at IS NULL
       AND p.published_at <= $1
       AND {resolution}
)
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $2 OR r.published_at >= $3)
 ORDER BY p.published_at DESC, p.post_id DESC"
        ),
        FeedSurface::User { .. } => format!(
            "WITH ranked AS (
     SELECT p.post_id, p.published_at,
            ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS rn
     FROM posts p
     JOIN users u ON p.user_id = u.user_id
     WHERE p.published_at IS NOT NULL
       AND p.deleted_at IS NULL
       AND p.published_at <= $1
       AND u.username = $2
       AND {resolution}
)
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $3 OR r.published_at >= $4)
 ORDER BY p.published_at DESC, p.post_id DESC"
        ),
        FeedSurface::SiteTag { .. } => format!(
            "WITH ranked AS (
     SELECT p.post_id, p.published_at,
            ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS rn
     FROM posts p
     JOIN post_tags pt ON p.post_id = pt.post_id
     JOIN tags t ON pt.tag_id = t.tag_id
     WHERE p.published_at IS NOT NULL
       AND p.deleted_at IS NULL
       AND p.published_at <= $1
       AND t.tag_slug = $2
       AND {resolution}
)
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $3 OR r.published_at >= $4)
 ORDER BY p.published_at DESC, p.post_id DESC"
        ),
        FeedSurface::UserTag { .. } => format!(
            "WITH ranked AS (
     SELECT p.post_id, p.published_at,
            ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS rn
     FROM posts p
     JOIN users u ON p.user_id = u.user_id
     JOIN post_tags pt ON p.post_id = pt.post_id
     JOIN tags t ON pt.tag_id = t.tag_id
     WHERE p.published_at IS NOT NULL
       AND p.deleted_at IS NULL
       AND p.published_at <= $1
       AND u.username = $2
       AND t.tag_slug = $3
       AND {resolution}
)
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $4 OR r.published_at >= $5)
 ORDER BY p.published_at DESC, p.post_id DESC"
        ),
    }
}

/// Returns the projections needed by the steady-state go-live pass.
pub(crate) async fn list_posts_gone_live_between<DB>(
    pool: &Pool<DB>,
    after: UtcInstant,
    upto: UtcInstant,
) -> Result<Vec<GoLivePost>>
where
    DB: PostDialect,
    PostRecord: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    // `published_at > $1 AND published_at <= $2` selects exactly the posts
    // that crossed into "live" within the half-open window `(after, upto]`.
    // Only current Public projections earn Syndication Feed regeneration.
    // The standard post projection (incl. the JSON tag subquery) is reused so the row
    // decodes directly into `PostRecord`; we then keep only the username + tag slugs
    // the feed fan-out needs.
    let tags = DB::TAGS_SUBQUERY;
    let sql = format!(
        "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                    {tags} AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE p.published_at > $1
               AND p.published_at <= $2
               AND p.deleted_at IS NULL
               AND {PUBLIC_AUDIENCE_PREDICATE}
             ORDER BY p.published_at ASC, p.post_id ASC"
    );
    let rows = sqlx::query_as::<_, PostRecord>(&sql)
        .bind_storage(after)
        .bind_storage(upto)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|rec| GoLivePost {
            username: rec.author_username,
            tag_slugs: rec.tags.into_iter().map(|t| t.tag_slug).collect(),
        })
        .collect())
}

/// Returns cached feed URLs whose rendered content is stale relative to a live post.
pub(crate) async fn feed_urls_needing_catchup<DB>(
    pool: &Pool<DB>,
    now: UtcInstant,
) -> Result<Vec<FeedPath>>
where
    DB: PostDialect,
    for<'r> FeedPath: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> UtcInstant: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> &'r str: sqlx::ColumnIndex<DB::Row>,
    usize: sqlx::ColumnIndex<DB::Row>,
    (UtcInstant,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    // Cached feeds live in the same database, so they are enumerated here
    // and, for each, the newest live post on that surface is compared
    // against the feed's own `generated_at`. Feed count is small, so a
    // per-feed check is simpler than a set-based join.
    //
    // Rows are read one at a time rather than via `query_as` so a single bad
    // `feed_url` cannot fail the whole scan — see the skip below.
    let rows = sqlx::query("SELECT feed_url, generated_at FROM feed_cache")
        .fetch_all(pool)
        .await?;
    let mut needing = Vec::new();
    let mut decode_reported = false;
    for row in rows {
        let generated_at: UtcInstant = row.try_get("generated_at")?;
        let feed_path = match row.try_get::<FeedPath, _>("feed_url") {
            Ok(path) => path,
            Err(error) => {
                if !decode_reported {
                    host::error::report_swallowed(
                        host::error::ErrorKind::Storage,
                        host::error::ErrorClass::Bug,
                        "storage.feed_cache.decode_feed_path",
                        host::error::SwallowedSource::Error(&error),
                    );
                    decode_reported = true;
                }
                continue;
            }
        };
        // `parts` is an expected defensive grammar mismatch. Construction
        // currently guarantees it cannot occur, but it carries no failure
        // source and therefore remains ordinary non-reporting control flow.
        let Some((surface, _)) = feed_path.parts() else {
            continue; // cov:ignore
        };
        if let Some(max) = max_published_at_for_surface::<DB>(pool, &surface, now).await?
            && max > generated_at
        {
            needing.push(feed_path);
        }
    }
    Ok(needing)
}

/// The most recent `published_at` of a current Public, live post
/// (`published_at <= now`, not deleted) on `surface`, or `None` when the surface
/// has no such post. Each surface variant adds exactly the joins/predicates that
/// define its post set, mirroring the window query's surface filters. Used by
/// [`crate::posts::store::PostStorage::feed_urls_needing_catchup`] to detect a
/// cached feed that is stale relative to a go-live the worker may have missed
/// while down.
async fn max_published_at_for_surface<DB>(
    pool: &Pool<DB>,
    surface: &common::feed::FeedSurface,
    now: UtcInstant,
) -> Result<Option<UtcInstant>>
where
    DB: PostDialect,
    (UtcInstant,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    // `Username`/`Tag` bind as themselves via the ADR-0071 sqlx bridge, for the
    // surface `username`/`tag` binds.
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    use common::feed::FeedSurface;
    let row: Option<(UtcInstant,)> = match surface {
        FeedSurface::Site => {
            sqlx::query_as(&format!(
                "SELECT p.published_at FROM posts p
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL
                   AND {PUBLIC_AUDIENCE_PREDICATE}
                 ORDER BY p.published_at DESC LIMIT 1"
            ))
            .bind_storage(now)
            .fetch_optional(pool)
            .await?
        }
        FeedSurface::User { username } => {
            sqlx::query_as(&format!(
                "SELECT p.published_at FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL AND u.username = $2
                   AND {PUBLIC_AUDIENCE_PREDICATE}
                 ORDER BY p.published_at DESC LIMIT 1"
            ))
            .bind_storage(now)
            .bind_storage(username)
            .fetch_optional(pool)
            .await?
        }
        FeedSurface::SiteTag { tag } => {
            sqlx::query_as(&format!(
                "SELECT p.published_at FROM posts p
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL AND t.tag_slug = $2
                   AND {PUBLIC_AUDIENCE_PREDICATE}
                 ORDER BY p.published_at DESC LIMIT 1"
            ))
            .bind_storage(now)
            .bind_storage(tag)
            .fetch_optional(pool)
            .await?
        }
        FeedSurface::UserTag { username, tag } => {
            sqlx::query_as(&format!(
                "SELECT p.published_at FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL AND u.username = $2 AND t.tag_slug = $3
                   AND {PUBLIC_AUDIENCE_PREDICATE}
                 ORDER BY p.published_at DESC LIMIT 1"
            ))
            .bind_storage(now)
            .bind_storage(username)
            .bind_storage(tag)
            .fetch_optional(pool)
            .await?
        }
    };
    Ok(row.map(|(published_at,)| published_at))
}
#[cfg(test)]
mod tests {
    use super::*;
    use common::feed::FeedSurface;
    use common::tag::Tag;
    use common::test_support::parse_username;

    #[test]
    fn publication_window_sql_preserves_surface_specific_bind_order() {
        let username = parse_username("alice");
        let tag: Tag = "rust".parse().expect("valid tag");
        let cases = [
            (
                FeedSurface::Site,
                "p.published_at <= $1",
                "(r.rn <= $2 OR r.published_at >= $3)",
            ),
            (
                FeedSurface::User {
                    username: username.clone(),
                },
                "u.username = $2",
                "(r.rn <= $3 OR r.published_at >= $4)",
            ),
            (
                FeedSurface::SiteTag { tag: tag.clone() },
                "t.tag_slug = $2",
                "(r.rn <= $3 OR r.published_at >= $4)",
            ),
            (
                FeedSurface::UserTag { username, tag },
                "u.username = $2",
                "(r.rn <= $4 OR r.published_at >= $5)",
            ),
        ];

        for (surface, source_bind, window_binds) in cases {
            let sql = window_sql(&surface, "tag_json", "visible");
            assert!(
                sql.contains(source_bind),
                "{surface:?}: source bind changed"
            );
            assert!(
                sql.contains(window_binds),
                "{surface:?}: window bind order changed"
            );
            assert!(
                sql.contains("tag_json AS tags"),
                "{surface:?}: tag projection"
            );
            assert!(
                sql.contains("AND visible"),
                "{surface:?}: resolution predicate"
            );
        }
    }
}
