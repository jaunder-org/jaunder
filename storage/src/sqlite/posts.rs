use async_trait::async_trait;
use sqlx::{Row, SqlitePool};

use crate::helpers::{post_record_from_row, PostRow};
use crate::{
    CreatePostError, CreatePostInput, ListByTagError, PostCursor, PostRecord, PostStorage, PostTag,
    TaggingError, UpdatePostError, UpdatePostInput,
};
use chrono::Utc;
use common::slug::Slug;
use common::tag::Tag;
use common::username::Username;

pub struct SqlitePostStorage {
    pool: SqlitePool,
}

impl SqlitePostStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PostStorage for SqlitePostStorage {
    async fn create_post(&self, input: &CreatePostInput) -> Result<i64, CreatePostError> {
        let now = Utc::now();

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING post_id",
        )
        .bind(input.user_id)
        .bind(&input.title)
        .bind(input.slug.as_str())
        .bind(&input.body)
        .bind(input.format.to_string())
        .bind(&input.rendered_html)
        .bind(now)
        .bind(now)
        .bind(input.published_at)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(id) => Ok(id),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(CreatePostError::SlugConflict)
            }
            Err(e) => Err(CreatePostError::Internal(e)),
        }
    }

    async fn get_post_by_id(&self, post_id: i64) -> sqlx::Result<Option<PostRecord>> {
        let row = sqlx::query_as::<_, PostRow>(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at,
                    COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE p.post_id = $1",
        )
        .bind(post_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(post_record_from_row).transpose()?)
    }

    async fn get_post_by_permalink(
        &self,
        username: &Username,
        year: i32,
        month: u32,
        day: u32,
        slug: &Slug,
    ) -> sqlx::Result<Option<PostRecord>> {
        let date_str = format!("{year:04}-{month:02}-{day:02}");
        let row = sqlx::query_as::<_, PostRow>(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at,
                    COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE u.username = $1
               AND p.slug = $2
               AND p.published_at IS NOT NULL
               AND p.deleted_at IS NULL
               AND date(p.published_at) = $3",
        )
        .bind(username.as_str())
        .bind(slug.as_str())
        .bind(&date_str)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(post_record_from_row).transpose()?)
    }

    async fn update_post(
        &self,
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        let mut tx = self.pool.begin().await?;
        let now = Utc::now();

        let existing = sqlx::query_as::<_, (i64, Option<chrono::DateTime<Utc>>)>(
            "SELECT user_id, deleted_at FROM posts WHERE post_id = $1",
        )
        .bind(post_id)
        .fetch_optional(&mut *tx)
        .await?;

        match existing {
            None => {
                tx.rollback().await.ok();
                return Err(UpdatePostError::NotFound);
            }
            Some((owner_id, deleted_at)) if owner_id != editor_user_id || deleted_at.is_some() => {
                tx.rollback().await.ok();
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
        .execute(&mut *tx)
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
                       created_at, updated_at, published_at, deleted_at,
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
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        post_record_from_row(row).map_err(UpdatePostError::Internal)
    }

    async fn soft_delete_post(&self, post_id: i64) -> sqlx::Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE posts SET deleted_at = $1 WHERE post_id = $2")
            .bind(now)
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn unpublish_post(&self, post_id: i64) -> sqlx::Result<()> {
        sqlx::query("UPDATE posts SET published_at = NULL WHERE post_id = ?")
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_published_by_user(
        &self,
        username: &Username,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $5",
            )
            .bind(username.as_str())
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2",
            )
            .bind(username.as_str())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    async fn list_published(
        &self,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $1 OR (p.created_at = $2 AND p.post_id < $3))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $4",
            )
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $1",
            )
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    async fn list_drafts_by_user(
        &self,
        user_id: i64,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.published_at IS NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $5",
            )
            .bind(user_id)
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.published_at IS NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2",
            )
            .bind(user_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    async fn tag_post(&self, post_id: i64, tag_display: &str) -> Result<(), TaggingError> {
        let tag: Tag = tag_display.parse().map_err(|_| {
            TaggingError::Internal(sqlx::Error::Decode("invalid tag format".into()))
        })?;

        let mut tx = self.pool.begin().await?;

        let post_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM posts WHERE post_id = $1")
                .bind(post_id)
                .fetch_one(&mut *tx)
                .await?;

        if !post_exists {
            tx.rollback().await.ok();
            return Err(TaggingError::PostNotFound);
        }

        sqlx::query("INSERT OR IGNORE INTO tags (tag_slug) VALUES ($1)")
            .bind(tag.as_str())
            .execute(&mut *tx)
            .await?;

        let tag_id: i64 =
            sqlx::query_scalar::<_, i64>("SELECT tag_id FROM tags WHERE tag_slug = $1")
                .bind(tag.as_str())
                .fetch_one(&mut *tx)
                .await?;

        let result =
            sqlx::query("INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES ($1, $2, $3)")
                .bind(post_id)
                .bind(tag_id)
                .bind(tag_display)
                .execute(&mut *tx)
                .await;

        match result {
            Ok(_) => {
                tx.commit().await?;
                Ok(())
            }
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                tx.rollback().await.ok();
                Err(TaggingError::AlreadyTagged)
            }
            Err(e) => {
                tx.rollback().await.ok();
                Err(TaggingError::Internal(e))
            }
        }
    }

    async fn untag_post(&self, post_id: i64, tag_slug: &Tag) -> Result<(), TaggingError> {
        let rows_deleted = sqlx::query(
            "DELETE FROM post_tags
             WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)",
        )
        .bind(post_id)
        .bind(tag_slug.as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows_deleted == 0 {
            Err(TaggingError::TagNotFound)
        } else {
            Ok(())
        }
    }

    async fn get_tags_for_post(&self, post_id: i64) -> sqlx::Result<Vec<PostTag>> {
        let rows = sqlx::query(
            "SELECT pt.post_id, pt.tag_id, t.tag_slug, pt.tag_display
             FROM post_tags pt
             JOIN tags t ON pt.tag_id = t.tag_id
             WHERE pt.post_id = $1
             ORDER BY t.tag_slug",
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let tag_slug_str: String = row.get("tag_slug");
                let tag_slug: Tag = tag_slug_str
                    .parse()
                    .map_err(|_| sqlx::Error::Decode("invalid tag format".into()))?;
                Ok(PostTag {
                    post_id: row.get("post_id"),
                    tag_id: row.get("tag_id"),
                    tag_slug,
                    tag_display: row.get("tag_display"),
                })
            })
            .collect()
    }

    async fn list_posts_by_tag(
        &self,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $5",
            )
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2",
            )
            .bind(tag_slug.as_str())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(post_record_from_row)
            .collect::<sqlx::Result<_>>()
            .map_err(ListByTagError::Internal)
    }

    async fn list_user_posts_by_tag(
        &self,
        user_id: i64,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $3 OR (p.created_at = $4 AND p.post_id < $5))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $6",
            )
            .bind(user_id)
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $3",
            )
            .bind(user_id)
            .bind(tag_slug.as_str())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(post_record_from_row)
            .collect::<sqlx::Result<_>>()
            .map_err(ListByTagError::Internal)
    }

    async fn list_tags(
        &self,
        prefix: Option<&str>,
        limit: u32,
    ) -> sqlx::Result<Vec<crate::TagRecord>> {
        let normalized = prefix
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_ascii_lowercase);
        let pattern = normalized.as_deref().map(|p| format!("{p}%"));
        let limit_i64 = i64::from(limit);

        let rows = match pattern {
            Some(ref like) => {
                sqlx::query(
                    "SELECT tag_id, tag_slug FROM tags
                     WHERE tag_slug LIKE $1
                     ORDER BY tag_slug
                     LIMIT $2",
                )
                .bind(like)
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT tag_id, tag_slug FROM tags
                     ORDER BY tag_slug
                     LIMIT $1",
                )
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        rows.into_iter()
            .map(|row| {
                let tag_slug_str: String = row.get("tag_slug");
                let tag_slug: Tag = tag_slug_str
                    .parse()
                    .map_err(|_| sqlx::Error::Decode("invalid tag format".into()))?;
                Ok(crate::TagRecord {
                    tag_id: row.get("tag_id"),
                    tag_slug,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;

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
        };
        let result = storage.create_post(&input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_post_by_id_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqlitePostStorage::new(pool.clone());
        pool.close().await;
        let result = storage.get_post_by_id(1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_published_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqlitePostStorage::new(pool.clone());
        pool.close().await;
        let result = storage.list_published(None, 10).await;
        assert!(result.is_err());
    }
}
