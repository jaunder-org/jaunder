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

/// Cursor for keyset pagination of published web Post timelines.
#[derive(Debug)]
pub struct PostCursor {
    /// Publication timestamp of the last item in the previous page.
    pub published_at: UtcInstant,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: PostId,
    /// Direction that produced this cursor.
    pub order: common::seed::TimelineOrder,
}

/// Borrowed pagination inputs shared by every published web timeline query.
///
/// Keeping order, cursor, and fetch limit cohesive prevents a caller from
/// accidentally pairing a cursor with the wrong direction.
#[derive(Debug)]
pub struct PublishedPageRequest<'a> {
    cursor: Option<&'a PostCursor>,
    order: common::seed::TimelineOrder,
    limit: common::pagination::RowLimit,
}

impl<'a> PublishedPageRequest<'a> {
    /// Start a timeline walk in `order`.
    #[must_use]
    pub const fn first(
        order: common::seed::TimelineOrder,
        limit: common::pagination::RowLimit,
    ) -> Self {
        Self {
            cursor: None,
            order,
            limit,
        }
    }

    /// Continue a timeline walk, deriving its order from the cursor.
    #[must_use]
    pub const fn after(cursor: &'a PostCursor, limit: common::pagination::RowLimit) -> Self {
        Self {
            cursor: Some(cursor),
            order: cursor.order,
            limit,
        }
    }

    #[must_use]
    pub const fn limit(&self) -> common::pagination::RowLimit {
        self.limit
    }

    pub(super) const fn into_parts(
        self,
    ) -> (
        Option<&'a PostCursor>,
        common::seed::TimelineOrder,
        common::pagination::RowLimit,
    ) {
        (self.cursor, self.order, self.limit)
    }
}

/// Cursor for the author-only draft listing, which retains creation ordering.
#[derive(Debug)]
pub struct DraftPostCursor {
    pub created_at: UtcInstant,
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

/// Projects a published [`PostRecord`] onto the keyset [`PostCursor`] that
/// paginates after it.
///
/// A published timeline row without its publication time violates the storage
/// query's `published_at IS NOT NULL` invariant, so never manufacture a cursor.
///
/// # Errors
///
/// Returns an internal error if a published timeline query projects an impossible row.
pub fn to_post_cursor(
    post: &PostRecord,
    order: common::seed::TimelineOrder,
) -> InternalResult<PostCursor> {
    let Some(published_at) = post.published_at else {
        return Err(InternalError::server_message(
            "published timeline row missing published_at",
        ));
    };
    Ok(PostCursor {
        published_at,
        post_id: post.post_id,
        order,
    })
}

/// Projects a wire timeline cursor onto its storage-side form.
#[must_use]
pub fn timeline_keyset_cursor(cursor: Option<common::seed::TimelineCursor>) -> Option<PostCursor> {
    cursor.map(|cursor| PostCursor {
        published_at: cursor.published_at,
        post_id: cursor.post_id,
        order: cursor.order,
    })
}

/// Projects a storage timeline cursor back onto its wire form.
#[must_use]
pub fn wire_cursor(cursor: &PostCursor) -> common::seed::TimelineCursor {
    common::seed::TimelineCursor {
        published_at: cursor.published_at,
        post_id: cursor.post_id,
        order: cursor.order,
    }
}

/// Projects a wire [`PageCursor`] onto the draft-list storage cursor.
#[must_use]
pub fn keyset_cursor(cursor: Option<PageCursor>) -> Option<DraftPostCursor> {
    cursor.map(|cursor| DraftPostCursor {
        created_at: cursor.created_at,
        post_id: cursor.post_id,
    })
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
    use super::{
        PostCursor, PublishedPageRequest, ScheduledPostCursor, scheduled_keyset_cursor,
        timeline_keyset_cursor, to_post_cursor, to_scheduled_post_cursor, wire_cursor,
        wire_scheduled_cursor,
    };
    use crate::posts::models::{PostFormat, PostRecord};
    use common::ids::{PostId, UserId};
    use common::seed::TimelineOrder;
    use common::test_support::{
        parse_post_body, parse_post_title, parse_row_limit, parse_slug, parse_username,
        rendered_html,
    };
    use common::time::UtcInstant;

    fn unpublished_post() -> PostRecord {
        PostRecord {
            author_display_name: None,
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
        }
    }

    #[test]
    fn published_cursor_rejects_row_without_publish_time() {
        let err = to_post_cursor(&unpublished_post(), TimelineOrder::Newest).unwrap_err();
        assert_eq!(
            err.operator_message(),
            "published timeline row missing published_at"
        );
    }

    #[test]
    fn scheduled_cursor_rejects_row_without_publish_time() {
        let err = to_scheduled_post_cursor(&unpublished_post()).unwrap_err();
        assert_eq!(
            err.operator_message(),
            "scheduled listing row missing published_at"
        );
    }

    #[test]
    fn post_cursor_round_trips_through_wire_cursor_with_its_order() {
        let cursor = PostCursor {
            published_at: "2026-04-12T08:30:00.123456Z".parse().unwrap(),
            post_id: PostId::from(42),
            order: TimelineOrder::Oldest,
        };

        let round_trip = timeline_keyset_cursor(Some(wire_cursor(&cursor))).unwrap();
        assert_eq!(round_trip.published_at, cursor.published_at);
        assert_eq!(round_trip.post_id, cursor.post_id);
        assert_eq!(round_trip.order, TimelineOrder::Oldest);
    }

    #[test]
    fn continuation_request_derives_order_from_its_cursor() {
        let cursor = PostCursor {
            published_at: UtcInstant::now(),
            post_id: PostId::from(42),
            order: TimelineOrder::Oldest,
        };

        let request = PublishedPageRequest::after(&cursor, parse_row_limit("10"));
        let (_, order, _) = request.into_parts();
        assert_eq!(order, TimelineOrder::Oldest);
    }

    #[test]
    fn scheduled_cursor_round_trips_through_wire_cursor() {
        let cursor = ScheduledPostCursor {
            published_at: "2026-04-12T08:30:00.123456Z".parse().unwrap(),
            post_id: PostId::from(42),
        };

        assert_eq!(
            scheduled_keyset_cursor(Some(wire_scheduled_cursor(&cursor)))
                .unwrap()
                .published_at,
            cursor.published_at
        );
    }
}
