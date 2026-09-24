//! Offline repair of legacy current-Post Media references.

use sqlx::{Encode, Executor, FromRow, Type};

use crate::PostDialect;
use crate::posts::media;
use crate::posts::models::RenderedHtml;
use common::ids::PostId;
use common::media::MediaReference;

pub(super) async fn backfill_post_media_references<DB>(
    conn: &mut DB::Connection,
) -> sqlx::Result<()>
where
    DB: PostDialect,
    (PostId, RenderedHtml): for<'r> FromRow<'r, DB::Row>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    // Under the offline lock and within the queue-row transaction, extraction
    // and replacement see the same snapshot without a second stale-read guard.
    let posts: Vec<(PostId, RenderedHtml)> = sqlx::query_as(
        "SELECT p.post_id, p.rendered_html
         FROM posts p
         WHERE EXISTS (
             SELECT 1 FROM post_media pm
             WHERE pm.post_id = p.post_id AND pm.reference_kind = 'legacy'
         )
         ORDER BY p.post_id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let candidates: Vec<(PostId, Vec<MediaReference>)> = posts
        .into_iter()
        .map(|(post_id, rendered_html)| {
            (
                post_id,
                host::render::extract_media_refs(rendered_html.as_ref()),
            )
        })
        .collect();
    if !candidates.is_empty() {
        media::replace_legacy_post_media::<DB>(conn, &candidates).await?;
    }
    Ok(())
}
