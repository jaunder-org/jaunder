//! Post keyset cursors and wire projections.

use common::seed::PageCursor;
use host::error::{InternalError, InternalResult};

use crate::posts::models::PostRecord;
use common::ids::{PostId, RevisionId};
use common::time::UtcInstant;

/// Immutable-ID cursor for newest-first revision history pagination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostRevisionCursor {
    pub revision_id: RevisionId,
}

/// Cursor for keyset pagination of post listings.
#[derive(Debug)]
pub struct PostCursor {
    /// Creation timestamp of the last item in the previous page.
    pub created_at: UtcInstant,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: PostId,
}

/// Cursor for keyset pagination of the scheduled-post listing
/// (ordered by `published_at ASC, post_id ASC`).
#[derive(Debug)]
pub struct ScheduledPostCursor {
    /// Publication timestamp of the last item in the previous page.
    pub published_at: UtcInstant,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: PostId,
}

/// Cursor for keyset pagination of the editor-facing per-user collection
/// (ordered by `updated_at DESC, post_id DESC`).
#[derive(Clone, Copy, Debug)]
pub struct CollectionCursor {
    /// Update timestamp of the last item in the previous page.
    pub updated_at: UtcInstant,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: PostId,
}

/// Projects a [`PostRecord`] onto the keyset [`PostCursor`] that paginates the
/// listing after it.
#[must_use]
pub fn to_post_cursor(post: &PostRecord) -> PostCursor {
    PostCursor {
        created_at: post.created_at,
        post_id: post.post_id,
    }
}

/// Projects a wire [`PageCursor`] onto the storage-side [`PostCursor`].
///
/// Infallible by construction, not by omission: the boundary parse ADR-0063 §4
/// asks for has already happened one layer out, at the `#[server]` argument —
/// `PageCursor` bundles the keyset components, so arg-decode rejects a half
/// cursor before any handler body runs. Nothing is left here to reject, which is
/// the whole point of taking the pair as one type rather than two `Option`s.
#[must_use]
pub fn keyset_cursor(cursor: Option<PageCursor>) -> Option<PostCursor> {
    cursor.map(|c| PostCursor {
        created_at: c.created_at,
        post_id: c.post_id,
    })
}

/// Projects the storage-side [`PostCursor`] back onto the wire [`PageCursor`] a
/// page hands the client as its `next_cursor` — the inverse of
/// [`keyset_cursor`], and kept beside it so the round trip reads as one pair.
#[must_use]
pub fn wire_cursor(cursor: &PostCursor) -> PageCursor {
    PageCursor {
        created_at: cursor.created_at,
        post_id: cursor.post_id,
    }
}

/// Projects a wire [`PageCursor`] onto the storage-side scheduled-post cursor.
///
/// The existing wire cursor shape is reused for author-only post lists; on the
/// scheduled surface its timestamp component carries the `published_at` key, not
/// the creation timestamp.
#[must_use]
pub fn scheduled_keyset_cursor(cursor: Option<PageCursor>) -> Option<ScheduledPostCursor> {
    cursor.map(|c| ScheduledPostCursor {
        published_at: c.created_at,
        post_id: c.post_id,
    })
}

/// Projects a scheduled row onto the keyset cursor that paginates after it.
///
/// The storage query that feeds this helper selects only `published_at IS NOT
/// NULL` rows. Returning a typed error instead of silently dropping the cursor
/// keeps a broken query projection from turning pagination into a duplicate page.
///
/// # Errors
///
/// Returns an internal error if a row from the scheduled-post listing lacks
/// `published_at`, which would make the next-page cursor undefined.
pub fn to_scheduled_post_cursor(post: &PostRecord) -> InternalResult<ScheduledPostCursor> {
    let Some(published_at) = post.published_at else {
        return Err(InternalError::server_message(
            "scheduled listing row missing published_at",
        ));
    };
    Ok(ScheduledPostCursor {
        published_at,
        post_id: post.post_id,
    })
}

/// Projects the storage-side scheduled cursor back onto the shared wire cursor.
#[must_use]
pub fn wire_scheduled_cursor(cursor: &ScheduledPostCursor) -> PageCursor {
    PageCursor {
        created_at: cursor.published_at,
        post_id: cursor.post_id,
    }
}

#[cfg(test)]
mod tests {
    use super::to_scheduled_post_cursor;
    use crate::posts::models::{PostFormat, PostRecord};
    use common::ids::{PostId, UserId};
    use common::test_support::{
        parse_post_body, parse_post_title, parse_slug, parse_username, rendered_html,
    };
    use common::time::UtcInstant;

    #[test]
    fn scheduled_cursor_rejects_row_without_publish_time() {
        let post = PostRecord {
            post_id: PostId::from(1),
            user_id: UserId::from(1),
            author_username: parse_username("author"),
            title: Some(parse_post_title("My Title")),
            slug: parse_slug("hello-world"),
            body: parse_post_body("My body"),
            format: PostFormat::Markdown,
            rendered_html: rendered_html("<p>My body</p>"),
            created_at: UtcInstant::now(),
            updated_at: UtcInstant::now(),
            published_at: None,
            deleted_at: None,
            summary: None,
            tags: vec![],
        };

        let err = to_scheduled_post_cursor(&post).unwrap_err();
        assert_eq!(
            err.operator_message(),
            "scheduled listing row missing published_at"
        );
    }
}
