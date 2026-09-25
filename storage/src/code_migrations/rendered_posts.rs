//! Offline rebuild of current Post rendering and its public projections.

use std::collections::BTreeSet;

use common::ids::PostId;
use common::post_body::PostBody;
use common::post_title::PostTitle;
use common::render::{PostFormat, RenderedHtml, RenderedPostTitle};
use common::tag::Tag;
use common::time::UtcInstant;
use common::username::Username;
use host::feed::FeedPath;
use sqlx::{Encode, Executor, FromRow, Type};

use crate::PostDialect;
use crate::posts::media;
use crate::sql::QueryStorageExt;

#[derive(sqlx::FromRow)]
pub(crate) struct CurrentPostRendering {
    post_id: PostId,
    title: Option<PostTitle>,
    body: PostBody,
    format: PostFormat,
    rendered_html: RenderedHtml,
    rendered_title: Option<RenderedPostTitle>,
}

/// Rebuilds every current Post, including retained Deleted Posts. The caller's
/// queue-row transaction commits this operation and its queue deletion together.
pub(super) async fn rebuild_rendered_posts<DB>(conn: &mut DB::Connection) -> sqlx::Result<()>
where
    DB: PostDialect,
    CurrentPostRendering: for<'r> FromRow<'r, DB::Row>,
    for<'r> (Username,): FromRow<'r, DB::Row>,
    for<'r> Tag: sqlx::Decode<'r, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RenderedHtml: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q RenderedPostTitle>: Encode<'q, DB> + Type<DB>,
    for<'q> FeedPath: Encode<'q, DB> + Type<DB>,
    for<'q> &'q FeedPath: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
{
    let posts: Vec<CurrentPostRendering> = sqlx::query_as(
        "SELECT post_id, title, body, format, rendered_html, rendered_title
         FROM posts ORDER BY post_id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let now = UtcInstant::now();
    let mut affected_feeds = BTreeSet::new();
    for post in posts {
        // SQLx has no generic domain-error variant. Preserve the renderer's
        // typed source while deriving the stored Post projection.
        let rendering = host::render::render_post(post.title, post.body, post.format)
            .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
        let body_changed = rendering.rendered_html() != &post.rendered_html;
        let title_changed = rendering.rendered_title() != post.rendered_title.as_ref();
        if !body_changed && !title_changed {
            continue;
        }
        if body_changed && title_changed {
            sqlx::query(
                "UPDATE posts SET rendered_html = $1, rendered_title = $2 WHERE post_id = $3",
            )
            .bind_storage(rendering.rendered_html())
            .bind_storage(rendering.rendered_title())
            .bind_storage(post.post_id)
            .execute(&mut *conn)
            .await?;
        } else if body_changed {
            sqlx::query("UPDATE posts SET rendered_html = $1 WHERE post_id = $2")
                .bind_storage(rendering.rendered_html())
                .bind_storage(post.post_id)
                .execute(&mut *conn)
                .await?;
        } else {
            sqlx::query("UPDATE posts SET rendered_title = $1 WHERE post_id = $2")
                .bind_storage(rendering.rendered_title())
                .bind_storage(post.post_id)
                .execute(&mut *conn)
                .await?;
        }
        if body_changed {
            media::replace_post_media::<DB>(conn, post.post_id, rendering.media()).await?;
        }
        let author: Option<(Username,)> = sqlx::query_as(
            "SELECT u.username FROM posts p JOIN users u ON u.user_id = p.user_id
             WHERE p.post_id = $1 AND p.deleted_at IS NULL AND p.published_at <= $2
               AND EXISTS (
                   SELECT 1 FROM post_audiences pa
                   JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
                   WHERE pa.post_id = p.post_id AND tk.name = 'public'
               )",
        )
        .bind_storage(post.post_id)
        .bind_storage(now)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((username,)) = author {
            let tags: Vec<Tag> = sqlx::query_scalar::<DB, Tag>(
                "SELECT t.tag_slug FROM post_tags pt
                 JOIN tags t ON t.tag_id = pt.tag_id
                 WHERE pt.post_id = $1 ORDER BY t.tag_slug",
            )
            .bind_storage(post.post_id)
            .fetch_all(&mut *conn)
            .await?;
            affected_feeds.extend(host::feed::affected_feed_urls(&username, &tags));
        }
    }
    for path in affected_feeds {
        sqlx::query("DELETE FROM feed_cache WHERE feed_url = $1")
            .bind_storage(&path)
            .execute(&mut *conn)
            .await?;
        sqlx::query("INSERT INTO feed_events (feed_url) VALUES ($1)")
            .bind_storage(&path)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
