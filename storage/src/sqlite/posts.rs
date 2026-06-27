use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Pool, Sqlite};

use crate::helpers::{post_record_from_row, PostRow};
use crate::{PostDialect, PostRecord, PostStore, TaggingError, UpdatePostError, UpdatePostInput};
use common::tag::Tag;

/// SQLite-backed post storage.
pub type SqlitePostStorage = PostStore<Sqlite>;

#[async_trait]
impl PostDialect for Sqlite {
    const TAGS_SUBQUERY: &'static str = "COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]')";

    const PERMALINK_DATE_CLAUSE: &'static str = "date(p.published_at) = $3";

    const DELETE_POST_AUDIENCES: &'static str = "DELETE FROM post_audiences WHERE post_id = ?";

    // Bind order: post_id, audience_id, kind_name (matches `replace_post_audiences`).
    const INSERT_POST_AUDIENCE: &'static str = "INSERT INTO post_audiences \
         (post_id, audience_id, target_kind_id) \
         VALUES (?, ?, (SELECT kind_id FROM target_kinds WHERE name = ?))";

    async fn update_post(
        pool: &Pool<Sqlite>,
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        // ADR-0021: take the write lock up front with BEGIN IMMEDIATE rather than a
        // deferred BEGIN, so the SELECT->INSERT step performs no shared->reserved lock
        // upgrade (the SQLITE_BUSY-on-upgrade failure mode). sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw connection,
        // mirroring create_user_with_invite / sqlite/backup.rs.
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        let now = Utc::now();

        let result: Result<PostRow, UpdatePostError> = async {
            let existing = sqlx::query_as::<_, (i64, Option<DateTime<Utc>>)>(
                "SELECT user_id, deleted_at FROM posts WHERE post_id = $1",
            )
            .bind(post_id)
            .fetch_optional(&mut *conn)
            .await?;

            match existing {
                None => return Err(UpdatePostError::NotFound),
                Some((owner_id, deleted_at))
                    if owner_id != editor_user_id || deleted_at.is_some() =>
                {
                    return Err(UpdatePostError::Unauthorized);
                }
                Some(_) => {}
            }

            sqlx::query(
                "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html, edited_at)
                 SELECT post_id, user_id, title, slug, body, format, rendered_html, $1
                 FROM posts WHERE post_id = $2",
            )
            .bind(now)
            .bind(post_id)
            .execute(&mut *conn)
            .await?;

            let row = sqlx::query_as::<_, PostRow>(
                "UPDATE posts
                 SET title = $1,
                     slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END,
                     body = $3,
                     format = $4,
                     rendered_html = $5,
                     published_at = CASE WHEN $6 THEN COALESCE(published_at, $7) ELSE NULL END,
                     updated_at = $8
                 WHERE post_id = $9
                 RETURNING post_id, user_id,
                           (SELECT username FROM users WHERE user_id = posts.user_id) AS username,
                           title, slug, body, format, rendered_html,
                           created_at, updated_at, published_at, deleted_at, summary,
                           COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags",
            )
            .bind(&input.title)
            .bind(input.slug.as_str())
            .bind(&input.body)
            .bind(input.format.to_string())
            .bind(&input.rendered_html)
            .bind(input.publish)
            .bind(now)
            .bind(now)
            .bind(post_id)
            .fetch_one(&mut *conn)
            .await?;

            crate::posts::replace_post_audiences::<Sqlite>(&mut *conn, post_id, &input.audiences)
                .await?;

            Ok(row)
        }
        .await;

        match result {
            Ok(row) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                post_record_from_row(row).map_err(UpdatePostError::Internal)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    async fn tag_post(
        pool: &Pool<Sqlite>,
        post_id: i64,
        tag_display: &str,
    ) -> Result<(), TaggingError> {
        let tag: Tag = tag_display.parse().map_err(|_| {
            TaggingError::Internal(sqlx::Error::Decode("invalid tag format".into()))
        })?;

        // ADR-0021: take the write lock up front with BEGIN IMMEDIATE rather than a
        // deferred BEGIN, so the SELECT->INSERT step performs no shared->reserved lock
        // upgrade (the SQLITE_BUSY-on-upgrade failure mode). sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw connection,
        // mirroring update_post / create_user_with_invite / sqlite/backup.rs.
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), TaggingError> = async {
            let post_exists: bool =
                sqlx::query_scalar("SELECT COUNT(*) > 0 FROM posts WHERE post_id = $1")
                    .bind(post_id)
                    .fetch_one(&mut *conn)
                    .await?;

            if !post_exists {
                return Err(TaggingError::PostNotFound);
            }

            sqlx::query("INSERT OR IGNORE INTO tags (tag_slug) VALUES ($1)")
                .bind(tag.as_str())
                .execute(&mut *conn)
                .await?;

            let tag_id: i64 =
                sqlx::query_scalar::<_, i64>("SELECT tag_id FROM tags WHERE tag_slug = $1")
                    .bind(tag.as_str())
                    .fetch_one(&mut *conn)
                    .await?;

            match sqlx::query(
                "INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES ($1, $2, $3)",
            )
            .bind(post_id)
            .bind(tag_id)
            .bind(tag_display)
            .execute(&mut *conn)
            .await
            {
                Ok(_) => Ok(()),
                Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                    Err(TaggingError::AlreadyTagged)
                }
                Err(e) => Err(TaggingError::Internal(e)),
            }
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    async fn untag_post(
        pool: &Pool<Sqlite>,
        post_id: i64,
        tag_slug: &Tag,
    ) -> Result<(), TaggingError> {
        let rows_deleted = sqlx::query(
            "DELETE FROM post_tags
             WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)",
        )
        .bind(post_id)
        .bind(tag_slug.as_str())
        .execute(pool)
        .await?
        .rows_affected();

        if rows_deleted == 0 {
            Err(TaggingError::TagNotFound)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;
    use crate::PostStorage;

    #[tokio::test]
    async fn create_post_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqlitePostStorage::new(pool.clone());
        pool.close().await;
        let input = crate::CreatePostInput {
            user_id: 1,
            title: Some("Test".to_string()),
            slug: "test-post".parse().unwrap(),
            body: "body".to_string(),
            format: crate::PostFormat::Markdown,
            rendered_html: "<p>body</p>".to_string(),
            published_at: None,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        };
        let result = storage.create_post(&input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_post_by_id_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqlitePostStorage::new(pool.clone());
        pool.close().await;
        let result = storage
            .get_post_by_id(1, &common::visibility::ViewerIdentity::Anonymous)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_published_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqlitePostStorage::new(pool.clone());
        pool.close().await;
        let result = storage
            .list_published(
                None,
                10,
                &common::visibility::ViewerIdentity::Anonymous,
                Utc::now(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn tag_post_insert_error_returns_internal() {
        let pool = sqlite_pool().await;
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator) VALUES (?, ?, ?, ?)",
        )
        .bind("tagger")
        .bind("hash")
        .bind(Utc::now())
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();
        let storage = SqlitePostStorage::new(pool.clone());
        let post_id = storage
            .create_post(&crate::CreatePostInput {
                user_id: 1,
                title: Some("Post".to_string()),
                slug: "post".parse().unwrap(),
                body: "body".to_string(),
                format: crate::PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
                published_at: None,
                summary: None,
                audiences: vec![common::visibility::AudienceTarget::Public],
            })
            .await
            .unwrap();

        // Break the post_tags INSERT (but not the existence check or tag insert) so it
        // returns a non-unique Database error: exercises the catch-all Internal arm and
        // the BEGIN IMMEDIATE rollback path on an unexpected failure.
        sqlx::query("ALTER TABLE post_tags RENAME COLUMN tag_display TO tag_display_x")
            .execute(&pool)
            .await
            .unwrap();

        let result = storage.tag_post(post_id, "rust").await;
        assert!(matches!(result, Err(TaggingError::Internal(_))));
    }

    #[tokio::test]
    async fn list_collection_by_user_orders_by_updated_at_desc_and_excludes_deleted() {
        let pool = sqlite_pool().await;
        let storage = SqlitePostStorage::new(pool.clone());

        // Create a test user
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator) VALUES (?, ?, ?, ?)",
        )
        .bind("testuser")
        .bind("hash")
        .bind(Utc::now())
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();

        let user_id = 1i64;
        let now = Utc::now();

        // Create 3 posts with different updated_at times
        // Post 1: draft (published_at = NULL)
        let post1_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at, summary)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING post_id",
        )
        .bind(user_id)
        .bind("Draft Post")
        .bind("draft-post")
        .bind("draft content")
        .bind("markdown")
        .bind("<p>draft content</p>")
        .bind(now - chrono::Duration::hours(3))
        .bind(now - chrono::Duration::hours(2))
        .bind(None::<chrono::DateTime<Utc>>)
        .bind(None::<String>)
        .fetch_one(&pool)
        .await
        .unwrap();

        // Post 2: published
        let post2_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at, summary)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING post_id",
        )
        .bind(user_id)
        .bind("Published Post")
        .bind("published-post")
        .bind("published content")
        .bind("markdown")
        .bind("<p>published content</p>")
        .bind(now - chrono::Duration::hours(2))
        .bind(now - chrono::Duration::hours(1))
        .bind(Some(now - chrono::Duration::minutes(30)))
        .bind(None::<String>)
        .fetch_one(&pool)
        .await
        .unwrap();

        // Post 3: soft-deleted (will be excluded)
        let post3_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at, deleted_at, summary)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING post_id",
        )
        .bind(user_id)
        .bind("Deleted Post")
        .bind("deleted-post")
        .bind("deleted content")
        .bind("markdown")
        .bind("<p>deleted content</p>")
        .bind(now - chrono::Duration::hours(1))
        .bind(now)
        .bind(Some(now - chrono::Duration::minutes(10)))
        .bind(Some(Utc::now()))
        .bind(None::<String>)
        .fetch_one(&pool)
        .await
        .unwrap();

        // List all posts in collection
        let results = storage
            .list_collection_by_user(user_id, None, 10)
            .await
            .unwrap();

        // Should have 2 posts (draft and published, not deleted)
        assert_eq!(results.len(), 2);

        // Check they are ordered by updated_at DESC (post2 updated more recently)
        assert_eq!(results[0].post_id, post2_id);
        assert_eq!(results[1].post_id, post1_id);

        // Verify draft is included
        assert!(results
            .iter()
            .any(|p| p.post_id == post1_id && p.published_at.is_none()));

        // Verify published is included
        assert!(results
            .iter()
            .any(|p| p.post_id == post2_id && p.published_at.is_some()));

        // Verify deleted is not included
        assert!(!results.iter().any(|p| p.post_id == post3_id));
    }
}
