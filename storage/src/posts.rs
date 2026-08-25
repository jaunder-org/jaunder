//! Content storage for posts, revisions, and tagging.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool, Row};
use thiserror::Error;

use common::feed::FeedPath;
use common::ids::{AudienceId, ChannelId, PostId, RevisionId, TagId, UserId};
use common::media::MediaRef;
use common::pagination::RowLimit;
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::root_relative_url::RootRelativeUrl;
use common::seed::PageCursor;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::{
    AudienceTarget, SubscriberRef, TargetKind, ViewerIdentity, local_subscriber_ref,
};
use host::error::{InternalError, InternalResult};

use crate::backend::Backend;
use crate::helpers::parse_post_tags_json;
pub use common::render::{InvalidPostFormat, PostFormat, RenderOutput, RenderedHtml};

/// The validated calendar date of a public permalink lookup key. Re-exported from
/// `common::time` so storage callers and the trait method name the domain type
/// directly (an impossible date is unrepresentable by construction — #583).
pub use common::time::PermalinkDate;

/// A post record returned by [`PostStorage`] queries.
///
/// `tags` is populated by the same query that loads the rest of the row via
/// a JSON-aggregating subquery, so post and tag state are always read from
/// the same statement-level snapshot. `author_username` is sourced from the
/// `users` table in the same query (via JOIN or correlated subquery), so
/// callers never need a second roundtrip to look up the post's author.
#[derive(Clone, Debug)]
pub struct PostRecord {
    /// Unique internal identifier.
    pub post_id: PostId,
    /// ID of the user who owns the post.
    pub user_id: UserId,
    /// Username of the author
    pub author_username: Username,
    /// Optional title.
    pub title: Option<PostTitle>,
    /// Unique slug (per user, per day).
    pub slug: Slug,
    /// Raw source body (Markdown or Org).
    pub body: PostBody,
    /// Format of the `body`.
    pub format: PostFormat,
    /// HTML produced by `render()` from the `body`, sanitized at that mint point —
    /// safe to emit unescaped (#445).
    // rendered-html-from-trusted:allow post read model carries render-sanitized HTML from storage (#701)
    pub rendered_html: RenderedHtml,
    /// When the post was first created.
    pub created_at: DateTime<Utc>,
    /// When the post was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the post was published (None if it is a draft).
    pub published_at: Option<DateTime<Utc>>,
    /// When the post was soft-deleted (None if active).
    pub deleted_at: Option<DateTime<Utc>>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
    /// The post's tags, ordered by `tag_slug` ascending (byte order).
    ///
    /// Populated by the same query that loaded the rest of the row — every post
    /// SELECT projects [`PostDialect::TAGS_SUBQUERY`] — so reading tags off a
    /// `PostRecord` costs no extra round-trip. The ordering is pinned in that
    /// subquery on both backends (#772); do not rely on insertion order.
    pub tags: Vec<PostTag>,
}

impl PostRecord {
    /// Returns the canonical permalink for this post as a [`RootRelativeUrl`].
    /// Uses the publication timestamp if published; otherwise falls back to the creation timestamp.
    #[must_use]
    pub fn permalink(&self) -> RootRelativeUrl {
        use chrono::Datelike;
        let timestamp = self.published_at.unwrap_or(self.created_at);
        let Ok(url) = format!(
            "/~{}/{:04}/{:02}/{:02}/{}",
            self.author_username,
            timestamp.year(),
            timestamp.month(),
            timestamp.day(),
            self.slug.as_ref()
        )
        .parse::<RootRelativeUrl>() else {
            unreachable!("permalink() builds a valid root-relative path");
        };
        url
    }

    /// Generates a fallback summary from the post's first non-blank body line.
    ///
    /// No title/slug fallbacks: [`PostBody`]'s invariant is *exactly* the condition
    /// [`PostSummary::from_body_line`] relies on — at least one line non-empty after
    /// trimming — so the body always answers (#811, #830, #858).
    #[must_use]
    pub fn fallback_summary_label(&self) -> PostSummary {
        PostSummary::from_body_line(&self.body)
    }
}

/// Decodes the shared post projection directly into its public storage record.
///
/// Every column except the JSON aggregate arrives as its domain type through its `SQLx`
/// bridge. `tags` remains an aggregate boundary: its text is parsed after the post id is
/// available to attach to each decoded tag.
impl<'r, R> sqlx::FromRow<'r, R> for PostRecord
where
    R: Row,
    &'r str: sqlx::ColumnIndex<R>,
    PostId: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    UserId: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    Username: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    PostTitle: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    Slug: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    PostBody: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    PostFormat: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    RenderedHtml: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    DateTime<Utc>: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    PostSummary: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    String: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    fn from_row(row: &'r R) -> sqlx::Result<Self> {
        let post_id = row.try_get::<PostId, _>("post_id")?;
        let user_id = row.try_get::<UserId, _>("user_id")?;
        let author_username = row.try_get::<Username, _>("username")?;
        let title = row.try_get::<Option<PostTitle>, _>("title")?;
        let slug = row.try_get::<Slug, _>("slug")?;
        let body = row.try_get::<PostBody, _>("body")?;
        let format = row.try_get::<PostFormat, _>("format")?;
        // rendered-html-from-trusted:allow post row decodes render-sanitized HTML from rendered_html (#619)
        let rendered_html = row.try_get::<RenderedHtml, _>("rendered_html")?;
        let created_at = row.try_get::<DateTime<Utc>, _>("created_at")?;
        let updated_at = row.try_get::<DateTime<Utc>, _>("updated_at")?;
        let published_at = row.try_get::<Option<DateTime<Utc>>, _>("published_at")?;
        let deleted_at = row.try_get::<Option<DateTime<Utc>>, _>("deleted_at")?;
        let summary = row.try_get::<Option<PostSummary>, _>("summary")?;
        let tags_json = row.try_get::<String, _>("tags")?;
        let tags = parse_post_tags_json(&tags_json, post_id)?;

        Ok(Self {
            post_id,
            user_id,
            author_username,
            title,
            slug,
            body,
            format,
            rendered_html,
            created_at,
            updated_at,
            published_at,
            deleted_at,
            summary,
            tags,
        })
    }
}

/// A post revision record returned by [`PostStorage`] queries.
///
/// Revisions are created automatically whenever a post is updated.
#[derive(Clone, Debug)]
pub struct PostRevisionRecord {
    /// Unique identifier for this revision.
    pub revision_id: RevisionId,
    /// ID of the associated post.
    pub post_id: PostId,
    /// ID of the user who made the edit.
    pub user_id: UserId,
    /// Title at the time of this revision.
    pub title: Option<PostTitle>,
    /// Slug at the time of this revision.
    pub slug: Slug,
    /// Raw source body at the time of this revision.
    pub body: PostBody,
    /// Format at the time of this revision.
    pub format: PostFormat,
    /// HTML produced by `render()` at the time of this revision, sanitized at that
    /// mint point — safe to emit unescaped (#445).
    // rendered-html-from-trusted:allow revision read model carries render-sanitized HTML from storage (#701)
    pub rendered_html: RenderedHtml,
    /// When this revision was created.
    pub edited_at: DateTime<Utc>,
}

/// Errors that can occur when creating a post.
#[derive(Debug, Error)]
pub enum CreatePostError {
    /// A post with the same slug already exists for this user on this day.
    #[error("slug already taken for this user on this date")]
    SlugConflict,
    /// The `(user_id, idempotency_key)` pair has already been used to create a
    /// post; the create is a duplicate of an earlier one.
    #[error("idempotency key already used for this user")]
    IdempotencyConflict,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur when updating a post.
#[derive(Debug, Error)]
pub enum UpdatePostError {
    /// The requested post does not exist.
    #[error("post not found")]
    NotFound,
    /// The user is not authorized to edit this post.
    #[error("not authorized")]
    Unauthorized,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<UpdatePostError> for host::error::InternalError {
    /// Reproduces the former inline `web::posts::mod` mapper
    /// `(kind, class, public_message)`: not-found/unauthorized mask as a 404;
    /// an internal failure is a masked storage error.
    fn from(error: UpdatePostError) -> Self {
        use host::error::InternalError;
        match error {
            UpdatePostError::NotFound | UpdatePostError::Unauthorized => {
                InternalError::not_found("Post")
            }
            UpdatePostError::Internal(e) => InternalError::storage(e),
        }
    }
}

/// Cursor for keyset pagination of post listings.
#[derive(Debug)]
pub struct PostCursor {
    /// Creation timestamp of the last item in the previous page.
    pub created_at: DateTime<Utc>,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: PostId,
}

/// Cursor for keyset pagination of the scheduled-post listing
/// (ordered by `published_at ASC, post_id ASC`).
#[derive(Debug)]
pub struct ScheduledPostCursor {
    /// Publication timestamp of the last item in the previous page.
    pub published_at: DateTime<Utc>,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: PostId,
}

/// Cursor for keyset pagination of the editor-facing per-user collection
/// (ordered by `updated_at DESC, post_id DESC`).
#[derive(Clone, Copy, Debug)]
pub struct CollectionCursor {
    /// Update timestamp of the last item in the previous page.
    pub updated_at: DateTime<Utc>,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: PostId,
}

/// Input for creating a new post.
#[derive(Clone)]
pub struct CreatePostInput {
    pub user_id: UserId,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub body: PostBody,
    pub format: PostFormat,
    /// The rendered body together with the media it references — see [`RenderOutput`],
    /// whose only constructor is rendering, so this input cannot carry a reference set
    /// that disagrees with its HTML (#711).
    pub rendered: RenderOutput,
    /// If Some, the post is created in a published state.
    pub published_at: Option<DateTime<Utc>>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
    /// Audience targeting for the post. Each entry becomes a `post_audiences`
    /// row; `Private` and an empty vec produce no rows (the post is private).
    pub audiences: Vec<AudienceTarget>,
    /// If `Some`, register this idempotency key against the new post in the
    /// same transaction. A `(user_id, key)` collision maps to
    /// [`CreatePostError::IdempotencyConflict`] and rolls the whole create back.
    pub idempotency_key: Option<String>,
}

/// Input for updating an existing post.
#[derive(Clone)]
pub struct UpdatePostInput {
    pub title: Option<PostTitle>,
    /// The new slug. Note: Slugs are typically immutable once published.
    pub slug: Slug,
    pub body: PostBody,
    pub format: PostFormat,
    /// The rendered body together with the media it references — see [`RenderOutput`].
    /// An edit can remove a reference, so the set must always be the one this HTML
    /// implies; deriving it is the only way to build one (#711).
    pub rendered: RenderOutput,
    /// If `true`, clear `published_at` back to NULL (draft / unschedule). Takes
    /// precedence over `explicit_published_at`.
    pub unpublish: bool,
    /// An exact publication instant to store (future = scheduled, past =
    /// backdated). `None` keeps any existing timestamp, or stamps `now` for a
    /// previously-unpublished post. Ignored when `unpublish` is `true`.
    pub explicit_published_at: Option<DateTime<Utc>>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
    /// Audience targeting for the post. On update the existing
    /// `post_audiences` rows are replaced to match this vec; `Private` and an
    /// empty vec produce no rows (the post is private).
    pub audiences: Vec<AudienceTarget>,
}

/// A tag record returned by [`PostStorage`] tag queries.
#[derive(Clone, Debug)]
pub struct TagRecord {
    pub tag_id: TagId,
    pub tag_slug: Tag,
}

/// A post-tag association returned by [`PostStorage`] tag queries.
#[derive(Clone, Debug)]
pub struct PostTag {
    pub post_id: PostId,
    pub tag_id: TagId,
    pub tag_slug: Tag,
    /// The original case-sensitive display name of the tag.
    pub tag_display: TagLabel,
}

/// A post that crossed into "live" within a time window, carrying exactly the
/// data the feed worker needs to compute its affected feed URLs (the author's
/// username and the post's tag slugs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoLivePost {
    pub username: Username,
    pub tag_slugs: Vec<Tag>,
}

/// The post's existing tags, read inside `set_post_tags`' transaction. The SQL is
/// identical on both dialects, so it is shared here rather than duplicated per
/// ADR-0019; only the surrounding transaction shape diverges. `ORDER BY` is not
/// needed for the diff (which is set-based) but keeps the read deterministic,
/// matching [`PostRecord::tags`] (#772).
pub(crate) const SELECT_POST_TAGS: &str = "SELECT pt.post_id, pt.tag_id, t.tag_slug, pt.tag_display
     FROM post_tags pt
     JOIN tags t ON pt.tag_id = t.tag_id
     WHERE pt.post_id = $1
     ORDER BY t.tag_slug";

/// Get-or-create a tag by slug, returning its id in **one** statement.
///
/// The no-op `DO UPDATE` is load-bearing: `DO NOTHING` emits no row for
/// `RETURNING` on the conflict path, which would force a second `SELECT` and
/// open a window in which a concurrently deleted tag yields `RowNotFound`
/// (#883). Rewriting `tag_slug` to the value it already holds makes the id come
/// back on both the insert and the conflict path. #343 landed the same shape
/// for `subscriptions`; both dialects run it.
///
/// Shared rather than per-dialect: `SQLite` accepts `$n` placeholders and
/// `ON CONFLICT … DO UPDATE … RETURNING`.
///
/// **Takes a row lock on the tag until commit**, which is why
/// [`post_tag_diff`] hands additions back in slug order.
///
/// Bind order: `tag_slug`.
pub(crate) const UPSERT_TAG_RETURNING_ID: &str = "INSERT INTO tags (tag_slug) VALUES ($1)
     ON CONFLICT (tag_slug) DO UPDATE SET tag_slug = excluded.tag_slug
     RETURNING tag_id";

/// Attaches a tag to a post, tolerating the row already being there.
///
/// `DO NOTHING`, not `DO UPDATE`: `desired` may carry two labels sharing a slug
/// ([`post_tag_diff`] does not dedupe) and the first occurrence's casing must
/// win, so the existing row is left exactly as it is. Nothing reads a value
/// back, so there is no reason to force a row out of the conflict path here.
///
/// Bind order: `post_id, tag_id, tag_display`.
pub(crate) const INSERT_POST_TAG: &str = "INSERT INTO post_tags
     (post_id, tag_id, tag_display) VALUES ($1, $2, $3)
     ON CONFLICT (post_id, tag_id) DO NOTHING";

/// Drops one tag from a post, by slug, inside `set_post_tags`' transaction. The
/// SQL is identical on both dialects, so it is shared here per ADR-0019.
///
/// `rows_affected` is deliberately never checked by callers: the slug came from
/// the tags read in the same transaction, so "no row deleted" is not an error.
pub(crate) const DELETE_POST_TAG_BY_SLUG: &str = "DELETE FROM post_tags
     WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)";

pub(crate) type PostOwnershipRow = (UserId, Option<DateTime<Utc>>);
pub(crate) type TagListRow = (TagId, Tag);
pub(crate) type PostTagRow = (PostId, TagId, Tag, TagLabel);

/// Maps [`SELECT_POST_TAGS`] rows to [`PostTag`].
///
/// The row tuple's first two positions are `post_id` and `tag_id` — adjacent ids
/// of the same width. Typing them rather than `i64` is what stops a swapped
/// destructuring from compiling (ADR-0063 §2); the SELECT's column order is the
/// only thing that pairs them otherwise.
pub(crate) fn post_tags_from_rows(rows: Vec<PostTagRow>) -> Vec<PostTag> {
    rows.into_iter()
        .map(|(post_id, tag_id, tag_slug, tag_display)| PostTag {
            post_id,
            tag_id,
            tag_slug,
            tag_display,
        })
        .collect()
}

/// The slug-level difference between a post's existing tags and a desired set
/// of display tokens, as computed by [`post_tag_diff`].
///
/// Borrows from both inputs. Applied by `set_post_tags` inside its transaction;
/// no caller performs the writes itself (#771).
pub(crate) struct PostTagDiff<'a> {
    /// Labels to add (their slug is not already present on the post).
    ///
    /// **Slug-ordered, contractually.** [`UPSERT_TAG_RETURNING_ID`] locks each
    /// `tags` row until commit, so applying these in a caller-supplied order lets
    /// two concurrent reconciles deadlock on Postgres (#876). The order is stable,
    /// so two labels sharing a slug keep their input order and the first
    /// occurrence's casing wins. Do not re-sort or re-shuffle at the call site.
    pub to_add: Vec<&'a TagLabel>,
    /// Existing tags to remove (their slug is not in the desired set).
    pub to_remove: Vec<&'a Tag>,
}

/// Diffs a post's `existing` tags against a `desired` set of [`TagLabel`]s.
///
/// Tagging is keyed on slug, so a desired label is "to add" only when no
/// existing tag shares its slug, and an existing tag is "to remove" only when
/// no desired label maps to its slug. Each `desired` label is already valid (its
/// `FromStr` ran at the boundary), so nothing is skipped here. Re-applying an
/// existing tag with different display casing is a no-op (the existing row's
/// casing is preserved by storage).
///
/// This is the pure core of `set_post_tags`, which applies the result inside its
/// own transaction on both dialects (#771).
#[must_use]
pub(crate) fn post_tag_diff<'a>(
    existing: &'a [PostTag],
    desired: &'a [TagLabel],
) -> PostTagDiff<'a> {
    use std::collections::HashSet;

    let existing_slugs: HashSet<Tag> = existing.iter().map(|t| t.tag_slug.clone()).collect();
    let desired_slugs: HashSet<Tag> = desired.iter().map(TagLabel::slug).collect();

    let mut to_add: Vec<&'a TagLabel> = desired
        .iter()
        .filter(|label| !existing_slugs.contains(&label.slug()))
        .collect();
    // Slug order, so every transaction takes `tags` row locks in the same order —
    // caller-supplied order can deadlock concurrent reconciles on Postgres (#876,
    // docs/adr/0125-slug-ordered-tag-lock-acquisition.md).
    //
    // `sort_by_key`, not `sort_unstable_by_key`: `desired` may carry two labels
    // sharing a slug and the FIRST occurrence's casing must still win, which
    // `set_post_tags_is_idempotent_and_absorbs_duplicate_slugs` asserts.
    to_add.sort_by_key(|label| label.slug());
    let to_remove = existing
        .iter()
        .filter(|tag| !desired_slugs.contains(&tag.tag_slug))
        .map(|tag| &tag.tag_slug)
        .collect();

    PostTagDiff { to_add, to_remove }
}

/// Errors that can occur when tagging a post.
#[derive(Debug, Error)]
pub enum TaggingError {
    /// The target post does not exist.
    #[error("post not found")]
    PostNotFound,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<TaggingError> for host::error::InternalError {
    /// Preserves the current wire class of the `set_post_tags` lift:
    /// the former `web` sites used `InternalError::server_message(e.to_string())`
    /// (kind `Internal`, public `"server operation failed"`). Routing through
    /// `server` keeps that projection while carrying the typed `TaggingError`
    /// as the operator-side source instead of stringifying it (A19).
    fn from(error: TaggingError) -> Self {
        host::error::InternalError::server(error)
    }
}

/// Errors that can occur when listing posts by tag.
#[derive(Debug, Error)]
pub enum ListByTagError {
    /// The specified tag does not exist.
    #[error("tag not found")]
    TagNotFound,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

// ---------------------------------------------------------------------------
// Cursor + effectful post orchestration
//
// Cursor (de)serialization plus the effectful read/tag helpers shared by
// `web`'s `#[server]` bodies and the `server` crate's public projector. They
// take `&dyn PostStorage`/`PostRecord`/`PostCursor` — storage abstractions the
// `host` floor cannot name — so they home here in `storage`, returning
// `host::error::InternalError` where fallible.
// ---------------------------------------------------------------------------

/// Projects a [`PostRecord`] onto the keyset [`PostCursor`] that paginates the
/// listing after it.
#[must_use]
pub fn to_post_cursor(post: &PostRecord) -> PostCursor {
    PostCursor {
        created_at: post.created_at,
        post_id: post.post_id,
    }
}

/// Parses the wire cursor pair into a [`PostCursor`]. Both components must be
/// supplied together (an opaque paging token) or both absent (the first page).
///
/// # Errors
///
/// Returns a validation error if only one component is present.
pub fn parse_post_cursor(
    cursor_created_at: Option<DateTime<Utc>>,
    cursor_post_id: Option<PostId>,
) -> InternalResult<Option<PostCursor>> {
    match (cursor_created_at, cursor_post_id) {
        (None, None) => Ok(None),
        (Some(created_at), Some(post_id)) => Ok(Some(PostCursor {
            created_at,
            post_id,
        })),
        _ => Err(InternalError::validation(
            "cursor_created_at and cursor_post_id must be provided together",
        )),
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
        created_at: c.created_at.value(),
        post_id: c.post_id,
    })
}

/// Projects the storage-side [`PostCursor`] back onto the wire [`PageCursor`] a
/// page hands the client as its `next_cursor` — the inverse of
/// [`keyset_cursor`], and kept beside it so the round trip reads as one pair.
#[must_use]
pub fn wire_cursor(cursor: &PostCursor) -> PageCursor {
    PageCursor {
        created_at: UtcInstant::from(cursor.created_at),
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
        published_at: c.created_at.value(),
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
        created_at: UtcInstant::from(cursor.published_at),
        post_id: cursor.post_id,
    }
}

/// The shared public-permalink lookup used by both the `get_post` server fn and
/// the non-reactive public projector.
///
/// Validates the date, then does the visibility-filtered store lookup for
/// `viewer`. The caller maps the record to an `AuthoredPost` with its own
/// `is_author` (the projector always anonymous → `false`; the server fn derives
/// it from the session), so there is one query and no drift between the two
/// public surfaces.
///
/// # Errors
///
/// Returns a storage error if the permalink lookup fails. The date is already a
/// valid calendar date by construction ([`PermalinkDate`]), so there is no
/// in-function date guard.
pub async fn fetch_post_record(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    username: &Username,
    date: PermalinkDate,
    slug: &Slug,
    now: DateTime<Utc>,
) -> InternalResult<Option<PostRecord>> {
    posts
        .get_post_by_permalink(username, date, slug, viewer, now)
        .await
        .map_err(InternalError::storage)
}

/// Applies the `TagNotFound → empty` business rule to a by-tag listing result:
/// a missing tag yields an empty page (not an error), while a real storage
/// failure propagates.
///
/// # Errors
///
/// Returns a storage error if the underlying listing failed for any reason
/// other than a missing tag.
pub fn list_by_tag_rows(
    result: Result<Vec<PostRecord>, ListByTagError>,
) -> InternalResult<Vec<PostRecord>> {
    match result {
        Ok(rows) => Ok(rows),
        Err(ListByTagError::TagNotFound) => Ok(Vec::new()),
        Err(ListByTagError::Internal(e)) => Err(InternalError::storage(e)),
    }
}

/// Async operations on the `posts` and `post_revisions` tables.
///
/// This trait manages the lifecycle of posts, including versioned edits,
/// draft/publish status, soft-deletion, and tagging.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait PostStorage: Send + Sync {
    /// Creates a new post.
    async fn create_post(&self, input: &CreatePostInput) -> Result<PostId, CreatePostError>;

    /// Creates `inputs.len()` posts in a single transaction, returning their new
    /// ids in input order. All-or-nothing: any failure (e.g. a slug conflict on
    /// one row) rolls the whole batch back and nothing persists. An empty slice
    /// is a no-op returning an empty vec without opening a transaction.
    async fn create_posts(
        &self,
        inputs: &[CreatePostInput],
    ) -> Result<Vec<PostId>, CreatePostError>;

    /// Returns the `post_id` a `(user_id, key)` idempotency pair maps to, or
    /// `None` if the key was never used by that user. Used to look up the
    /// original post on an [`CreatePostError::IdempotencyConflict`] retry.
    async fn post_id_for_idempotency_key(
        &self,
        user_id: UserId,
        key: &str,
    ) -> Result<Option<PostId>, sqlx::Error>;

    /// Fetches a post by its ID, applying the viewer-resolution filter: the post
    /// is returned only if `viewer` is the author or a targeted audience admits
    /// them. See ADR-0020.
    async fn get_post_by_id(
        &self,
        post_id: PostId,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Option<PostRecord>>;

    /// Fetches a post by its public permalink components, applying the
    /// viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts: a post with `published_at > now` is
    /// future-dated and stays invisible on this public surface until its time.
    async fn get_post_by_permalink(
        &self,
        username: &Username,
        date: PermalinkDate,
        slug: &Slug,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Option<PostRecord>>;

    /// Fetches an author's own not-yet-live post by its canonical permalink.
    ///
    /// True drafts match the UTC date of `created_at`; scheduled posts match
    /// the UTC date of `published_at`. `now` separates scheduled posts
    /// (`published_at > now`) from posts already live on the public surface.
    async fn get_unpublished_post_by_permalink(
        &self,
        user_id: UserId,
        date: PermalinkDate,
        slug: &Slug,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Option<PostRecord>>;

    /// Updates a post and creates a new revision.
    ///
    /// # Errors
    ///
    /// Returns [`UpdatePostError::NotFound`] if the post doesn't exist, or
    /// [`UpdatePostError::Unauthorized`] if the editor isn't the owner.
    async fn update_post(
        &self,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Publishes a draft: sets `published_at` to now if it is NULL, leaving an
    /// already-published post's timestamp untouched. Changes nothing else — not
    /// the body, rendered HTML, format, slug, summary, audiences or media rows.
    /// Publication is not an edit, so it does not go through `update_post` and
    /// records no revision (#711).
    ///
    /// # Errors
    ///
    /// Returns [`UpdatePostError::NotFound`] if the post does not exist or is
    /// soft-deleted, or [`UpdatePostError::Unauthorized`] if `user_id` does not
    /// own it.
    async fn publish_post(
        &self,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Marks a post as deleted without removing it from the database.
    async fn soft_delete_post(&self, post_id: PostId) -> sqlx::Result<()>;

    /// Reverts a published post to draft status.
    async fn unpublish_post(&self, post_id: PostId) -> sqlx::Result<()>;

    /// Lists published posts for a specific user, ordered by creation date,
    /// applying the viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts (`published_at > now`) off this public
    /// surface until their time.
    ///
    /// The explicit `'a` on the `cursor` reference exists so
    /// `mockall::automock` can mock this trait: automock cannot synthesize a
    /// lifetime for a reference nested inside a generic (here
    /// `Option<&PostCursor>`), so we name it. Behaviour is identical to
    /// lifetime elision — the annotation is purely to satisfy the macro
    /// (ref #245).
    async fn list_published_by_user<'a>(
        &self,
        username: &Username,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists all published posts across the entire site, applying the
    /// viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts (`published_at > now`) off this public
    /// surface until their time.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_published<'a>(
        &self,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists draft posts for a specific user.
    ///
    /// This is the author's "not-yet-live" surface: it returns true drafts
    /// (`published_at IS NULL`) **and** scheduled posts (`published_at > now`),
    /// so a future-dated post — invisible on every public surface until its
    /// time — stays visible to its own author. `now` gates which posts count
    /// as not-yet-live.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_drafts_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists the authenticated author's scheduled posts only.
    ///
    /// Scheduled posts have a non-NULL `published_at` strictly greater than
    /// explicit `now`. True drafts, live posts, soft-deleted posts, and posts
    /// owned by other users are excluded. Results are ordered by
    /// `published_at ASC, post_id ASC`.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_scheduled_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a ScheduledPostCursor>,
        limit: RowLimit,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists all of a user's non-soft-deleted posts (drafts + published)
    /// ordered by `updated_at DESC, post_id DESC` for the `AtomPub` Collection
    /// surface. Tags are hydrated.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_collection_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a CollectionCursor>,
        limit: RowLimit,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Makes the post's tags equal `desired`, in one transaction (#771, ADR-0092).
    ///
    /// The read, the diff and the writes all happen under a single write-lock
    /// acquisition, so a fan-out of N tags costs one acquisition rather than N.
    /// Tags already present with the same slug are left physically untouched, so
    /// the stored `tag_display` casing is preserved; an unchanged set writes
    /// nothing at all.
    ///
    /// An empty `desired` **clears** the post's tags — it is not a no-op.
    ///
    /// `desired` is **unbounded at this layer**: storage stays policy-free, so
    /// the per-post tag cap lives in `common::tag` with the rest of tag policy
    /// rather than being re-asserted here. ADR-0092's "capped by construction"
    /// is therefore enforced at the callers — both production front-ends, web
    /// and `AtomPub`, route their input through `parse_and_validate_tags`, so
    /// no production path can hand this method an unbounded set. A larger set
    /// is executed faithfully; it is simply not reachable outside tests.
    ///
    /// # Errors
    ///
    /// [`TaggingError::PostNotFound`] if the post does not exist. Soft-deleted
    /// posts are tagged normally.
    async fn set_post_tags(
        &self,
        post_id: PostId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError>;

    /// Lists published posts that carry a specific tag, applying the
    /// viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts (`published_at > now`) off this public
    /// surface until their time.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_posts_by_tag<'a>(
        &self,
        tag_slug: &Tag,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<PostRecord>, ListByTagError>;

    /// Lists published posts for a specific user that carry a specific tag,
    /// applying the viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts (`published_at > now`) off this public
    /// surface until their time.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_user_posts_by_tag<'a>(
        &self,
        user_id: UserId,
        tag_slug: &Tag,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<PostRecord>, ListByTagError>;

    /// Returns tag records whose slug begins with `prefix` (case-insensitive
    /// on the slug). An empty / `None` prefix returns all tags, alphabetically,
    /// up to `limit`.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_tags<'a>(
        &self,
        prefix: Option<&'a str>,
        limit: RowLimit,
    ) -> sqlx::Result<Vec<TagRecord>>;

    /// Lists published posts matching `surface`, applying the
    /// [`HybridWindow`](common::feed::HybridWindow) selection rule (union of
    /// "the most recent `min_items` items" and "all items published within the
    /// last `min_days`"). Results are ordered by `published_at DESC`.
    ///
    /// `now` is passed in so callers can supply a deterministic clock in
    /// tests. Posts with `published_at > now` (future-dated) are excluded.
    async fn list_published_in_window(
        &self,
        surface: &common::feed::FeedSurface,
        window: &common::feed::HybridWindow,
        now: DateTime<Utc>,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists posts that crossed into "live" within the window `(after, upto]`
    /// (exclusive lower, inclusive upper): `published_at > after AND
    /// published_at <= upto AND deleted_at IS NULL`. Each [`GoLivePost`] carries
    /// its author username and tag slugs so the feed worker can fan out to the
    /// affected feed surfaces. Drives the steady-state go-live pass.
    async fn list_posts_gone_live_between(
        &self,
        after: DateTime<Utc>,
        upto: DateTime<Utc>,
    ) -> sqlx::Result<Vec<GoLivePost>>;

    /// Returns the URLs of cached feeds whose surface has a live post
    /// (`published_at <= now`, not deleted) strictly newer than the feed's own
    /// `generated_at` — i.e. cached feeds that missed a go-live while the worker
    /// was down. Drives the feed-relative startup catch-up.
    async fn feed_urls_needing_catchup(&self, now: DateTime<Utc>) -> sqlx::Result<Vec<FeedPath>>;

    /// Reads a post's audience targeting as a [`Vec<AudienceTarget>`], for
    /// pre-selecting the editor's audience picker.
    ///
    /// Owner-only: this performs no viewer resolution and is intended to be
    /// called for a post the caller already owns. Maps each `post_audiences`
    /// row back to its [`AudienceTarget`] (`public` → [`AudienceTarget::Public`],
    /// `subscribers` → [`AudienceTarget::Subscribers`], `named` →
    /// [`AudienceTarget::Named`]); a post with no rows yields an empty vec
    /// (equivalent to [`AudienceTarget::Private`]). See ADR-0020.
    async fn get_post_audiences(&self, post_id: PostId) -> sqlx::Result<Vec<AudienceTarget>>;

    /// The ids of `user_id`'s non-soft-deleted posts whose rendered HTML points at
    /// `media`, ascending. An unreferenced item yields an empty vec.
    ///
    /// The read half of `post_media`'s lifecycle, kept in the same trait and module
    /// as [`replace_post_media`], which writes those rows (#711).
    ///
    /// **Deliberately unlimited.** The join answers the question exactly; any cap
    /// would let a reference in an old post past the window leave its media
    /// silently deletable.
    ///
    /// **Deliberately scoped to `user_id`'s own posts.** `media` is keyed per-user,
    /// so another user may hold a row for the same on-disk entry; their posts do not
    /// block this user's delete, and — since the caller shows this list to the
    /// deleting user — must not be disclosed to them (spec D9).
    async fn list_posts_referencing_media(
        &self,
        user_id: UserId,
        media: &MediaRef,
    ) -> sqlx::Result<Vec<PostId>>;
}

/// Backend-specific divergence for [`PostStore`].
///
/// Two consts capture SQL-fragment divergence shared by many methods:
/// [`TAGS_SUBQUERY`][PostDialect::TAGS_SUBQUERY] (`SQLite` `json_group_array`
/// vs Postgres `json_agg`/`::text`) and
/// [`PERMALINK_DATE_CLAUSE`][PostDialect::PERMALINK_DATE_CLAUSE] (`SQLite`
/// `date(COALESCE(...))` vs Postgres
/// `date(COALESCE(...) AT TIME ZONE 'UTC') = $3::date`).
///
/// The two transaction-bearing mutations are monomorphised per backend:
/// [`update_post`][PostDialect::update_post] (Postgres locks the row with
/// `FOR UPDATE`) and [`set_post_tags`][PostDialect::set_post_tags], which
/// diverges twice over — the transaction shape (`SQLite` drives a manual
/// `BEGIN IMMEDIATE`, Postgres locks the post row with `FOR UPDATE`; ADR-0021).
/// That transaction shape is the *only* divergence — the tag write itself is
/// the shared [`UPSERT_TAG_RETURNING_ID`] and [`INSERT_POST_TAG`] (#876).
/// Everything else is shared on [`PostStore`]. See ADR-0019.
#[async_trait]
pub trait PostDialect: Backend {
    /// Correlated JSON tag-aggregation subquery (on `p.post_id`) spelled in
    /// this backend's JSON dialect, yielding a `text` column.
    ///
    /// Both dialects order the aggregate by `t.tag_slug`, which is what makes
    /// [`PostRecord::tags`] slug-ordered (#772). Postgres spells it
    /// `ORDER BY t.tag_slug COLLATE "C"`: its default collation comes from the
    /// cluster locale and disagrees with `SQLite`'s BINARY on the hyphens and
    /// digits in the slug alphabet, so the `COLLATE` is what makes the two
    /// backends agree. Keep the two constants in sync — asserted by
    /// `tags_subquery_pins_slug_ordering_on_both_dialects`.
    const TAGS_SUBQUERY: &'static str;

    /// Predicate matching a post's canonical UTC permalink date —
    /// `COALESCE(published_at, created_at)` — against the bound `YYYY-MM-DD`
    /// string (`$3`), in this backend's date dialect.
    const PERMALINK_DATE_CLAUSE: &'static str;

    /// Deletes every `post_audiences` row for a post. Bind order: `post_id`.
    const DELETE_POST_AUDIENCES: &'static str;
    /// Inserts one `post_audiences` row, resolving the target-kind name to its
    /// `kind_id` via a subquery. Bind order: `post_id, audience_id, kind_name`.
    const INSERT_POST_AUDIENCE: &'static str;

    /// Deletes every `post_media` row for a post. Bind order: `post_id`.
    const DELETE_POST_MEDIA: &'static str;
    /// Inserts one `post_media` row. Bind order:
    /// `post_id, source, sha256, filename`.
    const INSERT_POST_MEDIA: &'static str;

    /// Update a post and record a revision, returning the updated record.
    async fn update_post(
        pool: &Pool<Self>,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Reconcile the post's tags to `desired` in one transaction. Monomorphised
    /// because the **serialization** differs: `SQLite` opens `BEGIN IMMEDIATE`,
    /// Postgres locks the post row with `FOR UPDATE` (ADR-0019, ADR-0021). The
    /// statements it issues are shared, not per-dialect (#876).
    async fn set_post_tags(
        pool: &Pool<Self>,
        post_id: PostId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError>;
}

/// The single definition of "which of `user_id`'s live posts reference this media" —
/// the `FROM`/`JOIN`/`WHERE` half of that question, leaving the projection (and any
/// `ORDER BY`) to whoever splices it in.
///
/// Bind order: `$1` `user_id`, `$2` `source`, `$3` `sha256`, `$4` `filename`.
///
/// Both places that ask the question use this fragment:
/// [`PostStorage::list_posts_referencing_media`], which *reports* the references so a
/// refusal can be explained, and the `NOT EXISTS` guard inside
/// [`try_delete_media`][crate::media::MediaStorage::try_delete_media], which *makes*
/// that refusal atomically (#711, spec D8). They have to agree: spelled separately,
/// widening or narrowing "referenced" in one would leave the guard blocking deletes
/// the message says are unblocked, or vice versa — a disagreement nothing would catch
/// at compile time. So it is spelled once, here, in the module that owns `post_media`.
pub(crate) const POSTS_REFERENCING_MEDIA_FROM_WHERE: &str = "\
     FROM post_media pm \
     JOIN posts p ON p.post_id = pm.post_id \
     WHERE p.user_id = $1 \
       AND p.deleted_at IS NULL \
       AND pm.source = $2 \
       AND pm.sha256 = $3 \
       AND pm.filename = $4";

/// The global version of [`POSTS_REFERENCING_MEDIA_FROM_WHERE`], used by media
/// file reclamation where any live Post naming the on-disk entry keeps the file
/// present, regardless of which user owns the media row being deleted.
///
/// Bind order: `$1` `source`, `$2` `sha256`, `$3` `filename`.
pub(crate) const ANY_POST_REFERENCING_MEDIA_FROM_WHERE: &str = "\
     FROM post_media pm \
     JOIN posts p ON p.post_id = pm.post_id \
     WHERE p.deleted_at IS NULL \
       AND pm.source = $1 \
       AND pm.sha256 = $2 \
       AND pm.filename = $3";

/// Generic [`PostStorage`] backed by any [`PostDialect`] database.
///
/// Every read and the non-transactional shared mutations live here, splicing
/// [`PostDialect::TAGS_SUBQUERY`] / [`PostDialect::PERMALINK_DATE_CLAUSE`] into
/// otherwise-identical SQL; the transaction-bearing mutations delegate to
/// [`PostDialect`]. See ADR-0019.
pub struct PostStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> PostStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> PostStorage for PostStore<DB>
where
    DB: PostDialect,
    PostRecord: for<'r> sqlx::FromRow<'r, DB::Row>,
    (PostId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (bool,): for<'r> sqlx::FromRow<'r, DB::Row>,
    PostTagRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    TagListRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    (TargetKind, Option<AudienceId>): for<'r> sqlx::FromRow<'r, DB::Row>,
    (DateTime<Utc>,): for<'r> sqlx::FromRow<'r, DB::Row>,
    // `feed_urls_needing_catchup` reads `feed_cache` a row at a time (a bad `feed_url`
    // must not fail the scan), so it needs the column-decode bounds directly rather than
    // a `FromRow` tuple. `FeedPath` decodes as itself via the ADR-0071 bridge.
    for<'r> FeedPath: sqlx::Decode<'r, DB> + sqlx::Type<DB>,
    for<'r> DateTime<Utc>: sqlx::Decode<'r, DB> + sqlx::Type<DB>,
    for<'r> &'r str: sqlx::ColumnIndex<DB::Row>,
    // Not residue: the ADR-0071 bridge *delegates* to `i64`, so this pair is what
    // makes every id newtype bind on a generic backend.
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q str>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<String>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // The viewer-resolution binds are NULL-able (`ResolutionBinds::bind_onto`).
    for<'q> Option<UserId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<ChannelId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q SubscriberRef: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q SubscriberRef>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Slug`/`Tag`/`Username` bind and decode as themselves via the ADR-0071 sqlx
    // into their newtypes). The `Option<&PostTitle>` bound is the nullable `title`
    // bind, forwarded from `write_post_in_tx` (create paths).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> Option<&'q PostTitle>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `summary` binds as `Option<&PostSummary>` via the ADR-0071 sqlx bridge on
    // the create paths, mirroring the `Option<&PostTitle>` bound above.
    for<'q> Option<&'q PostSummary>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<AudienceId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `RowLimit` binds as itself via the ADR-0071 sqlx bridge (delegates to `i64`) —
    // every listing's `LIMIT` placeholder (#696).
    for<'q> RowLimit: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<DateTime<Utc>>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.posts.create",
        skip(self, input),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn create_post(&self, input: &CreatePostInput) -> Result<PostId, CreatePostError> {
        let mut tx = self.pool.begin().await?;
        // On any error the `?` drops `tx`, which sqlx rolls back. (`&mut tx`
        // coerces to `&mut DB::Connection` for the helper.)
        let post_id = write_post_in_tx::<DB>(&mut tx, input).await?;
        tx.commit().await?;
        Ok(post_id)
    }

    #[tracing::instrument(
        name = "storage.posts.create_batch",
        skip(self, inputs),
        fields(db.system = DB::DB_SYSTEM, count = inputs.len())
    )]
    async fn create_posts(
        &self,
        inputs: &[CreatePostInput],
    ) -> Result<Vec<PostId>, CreatePostError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.pool.begin().await?;
        let mut ids = Vec::with_capacity(inputs.len());
        for input in inputs {
            // `?` drops `tx` on error → whole-batch rollback (atomic seed).
            ids.push(write_post_in_tx::<DB>(&mut tx, input).await?);
        }
        tx.commit().await?;
        Ok(ids)
    }

    #[tracing::instrument(
        name = "storage.posts.post_id_for_idempotency_key",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn post_id_for_idempotency_key(
        &self,
        user_id: UserId,
        key: &str,
    ) -> Result<Option<PostId>, sqlx::Error> {
        let post_id = sqlx::query_scalar::<_, PostId>(
            "SELECT post_id FROM idempotency_keys WHERE user_id = $1 AND key = $2",
        )
        .bind(user_id)
        // sqlx-newtype-bind:allow deferred-newtype #1086 — idempotency keys are tracked for a domain value.
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(post_id)
    }

    #[tracing::instrument(
        name = "storage.posts.get_by_id",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_post_by_id(
        &self,
        post_id: PostId,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Option<PostRecord>> {
        let (resolution, binds, _) = resolution_where(viewer, 2);
        let sql = format!(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                    {tags} AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE p.post_id = $1
               AND {resolution}",
            tags = DB::TAGS_SUBQUERY,
        );
        let query = sqlx::query_as::<_, PostRecord>(&sql).bind(post_id);
        Ok(binds.bind_onto(query).fetch_optional(&self.pool).await?)
    }

    #[tracing::instrument(
        name = "storage.posts.get_audiences",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_post_audiences(&self, post_id: PostId) -> sqlx::Result<Vec<AudienceTarget>> {
        // Owner-only: no viewer resolution. `ORDER BY` makes the result
        // deterministic so callers can compare vecs directly.
        let rows: Vec<(TargetKind, Option<AudienceId>)> = sqlx::query_as(
            "SELECT tk.name, pa.audience_id \
             FROM post_audiences pa \
             JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id \
             WHERE pa.post_id = $1 \
             ORDER BY tk.name, pa.audience_id",
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, audience_id)| audience_target_from_row(kind, audience_id))
            .collect())
    }

    #[tracing::instrument(
        name = "storage.posts.list_referencing_media",
        skip(self, media),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_posts_referencing_media(
        &self,
        user_id: UserId,
        media: &MediaRef,
    ) -> sqlx::Result<Vec<PostId>> {
        // Identical on both backends, so it stays here rather than becoming a
        // `PostDialect` const (ADR-0019). No `LIMIT`: see the trait doc. The predicate
        // itself is `POSTS_REFERENCING_MEDIA_FROM_WHERE`, shared with the delete guard.
        //
        // Decodes straight into `PostId` rather than `i64`-then-convert: an id column
        // decodes as its newtype (ADR-0085, #715), which is also what keeps the `i64`
        // decode bound off this impl.
        let sql =
            format!("SELECT pm.post_id {POSTS_REFERENCING_MEDIA_FROM_WHERE} ORDER BY pm.post_id");
        sqlx::query_scalar::<_, PostId>(&sql)
            .bind(user_id)
            .bind(media.source)
            .bind(&media.sha256)
            .bind(&media.filename)
            .fetch_all(&self.pool)
            .await
    }

    #[tracing::instrument(
        name = "storage.posts.get_by_permalink",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_post_by_permalink(
        &self,
        username: &Username,
        date: PermalinkDate,
        slug: &Slug,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Option<PostRecord>> {
        // `PermalinkDate`'s Display is ISO `YYYY-MM-DD` — the exact string the
        // `PERMALINK_DATE_CLAUSE` binds.
        let date_str = date.to_string();
        let (resolution, binds, _) = resolution_where(viewer, 5);
        // `published_at <= $4` hides scheduled (future-dated) posts until due.
        let sql = format!(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                    {tags} AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE u.username = $1
               AND p.slug = $2
               AND p.published_at IS NOT NULL
               AND p.published_at <= $4
               AND p.deleted_at IS NULL
               AND {date_clause}
               AND {resolution}",
            tags = DB::TAGS_SUBQUERY,
            date_clause = DB::PERMALINK_DATE_CLAUSE,
        );
        let query = sqlx::query_as::<_, PostRecord>(&sql)
            .bind(username)
            .bind(slug)
            // sqlx-newtype-bind:allow permanent-primitive — permalink date is a formatted URL path part, not a domain value.
            .bind(date_str.as_str())
            .bind(now);
        Ok(binds.bind_onto(query).fetch_optional(&self.pool).await?)
    }

    #[tracing::instrument(
        name = "storage.posts.get_unpublished_by_permalink",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_unpublished_post_by_permalink(
        &self,
        user_id: UserId,
        date: PermalinkDate,
        slug: &Slug,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Option<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let date_clause = DB::PERMALINK_DATE_CLAUSE;
        let date_str = date.to_string();
        let sql = format!(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                    {tags} AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE p.user_id = $1
               AND p.slug = $2
               AND {date_clause}
               AND (p.published_at IS NULL OR p.published_at > $4)
               AND p.deleted_at IS NULL"
        );
        let row = sqlx::query_as::<_, PostRecord>(&sql)
            .bind(user_id)
            .bind(slug)
            // sqlx-newtype-bind:allow permanent-primitive — permalink date is a formatted URL path part, not a domain value.
            .bind(date_str.as_str())
            .bind(now)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    #[tracing::instrument(
        name = "storage.posts.update",
        skip(self, input),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn update_post(
        &self,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        DB::update_post(&self.pool, post_id, editor_user_id, input).await
    }

    #[tracing::instrument(
        name = "storage.posts.publish",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn publish_post(
        &self,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<PostRecord, UpdatePostError> {
        // No dialect split (ADR-0019): ownership and liveness are the UPDATE's own
        // predicate rather than a preceding SELECT, so there is no check-then-write
        // window for `update_post`'s `FOR UPDATE` / `BEGIN IMMEDIATE` locking to
        // close, and one statement writes the single column publication touches.
        let published = sqlx::query_scalar::<_, PostId>(
            "UPDATE posts
                SET published_at = COALESCE(published_at, $1),
                    updated_at = $1
              WHERE post_id = $2 AND user_id = $3 AND deleted_at IS NULL
          RETURNING post_id",
        )
        .bind(Utc::now())
        .bind(post_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        if published.is_none() {
            // Nothing matched, so the post is either gone or someone else's — a live
            // row here is necessarily another user's, or the UPDATE would have
            // matched. One pure-existence read (`post_id`, never the owner's
            // identity) tells the two apart for the caller's error; nothing was
            // written, so there is no state to unwind. Decoding an id column into
            // its newtype rather than `i64` is the ADR-0085 convention (#715).
            let live = sqlx::query_scalar::<_, PostId>(
                "SELECT post_id FROM posts WHERE post_id = $1 AND deleted_at IS NULL",
            )
            .bind(post_id)
            .fetch_optional(&self.pool)
            .await?;
            return Err(if live.is_some() {
                UpdatePostError::Unauthorized
            } else {
                UpdatePostError::NotFound
            });
        }

        // Re-read through the record projection so `tags` and `author_username` come
        // back populated. Owner-only, so no viewer resolution.
        let sql = format!(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                    {tags} AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE p.post_id = $1",
            tags = DB::TAGS_SUBQUERY,
        );
        let row = sqlx::query_as::<_, PostRecord>(&sql)
            .bind(post_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row)
    }

    #[tracing::instrument(
        name = "storage.posts.soft_delete",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn soft_delete_post(&self, post_id: PostId) -> sqlx::Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE posts SET deleted_at = $1 WHERE post_id = $2")
            .bind(now)
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.posts.unpublish",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn unpublish_post(&self, post_id: PostId) -> sqlx::Result<()> {
        sqlx::query("UPDATE posts SET published_at = NULL WHERE post_id = $1")
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.posts.list_published_by_user",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_published_by_user<'a>(
        &self,
        username: &Username,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1 username, $2/$3 cursor, $4 post_id, $5 now, then the
            // resolution fragment from $6 — 3 or 5 placeholders depending on the
            // viewer variant — and the limit at the returned `limit_idx`.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 6);
            // `published_at <= $5` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $5
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(username)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id)
                .bind(now);
            binds
                .bind_onto(query)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 username, $2 now, then the variant-sized resolution
            // fragment from $3 and the limit at the returned `limit_idx`.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 3);
            // `published_at <= $2` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $2
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(username)
                .bind(now);
            binds
                .bind_onto(query)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };
        Ok(rows)
    }

    #[tracing::instrument(
        name = "storage.posts.list_published",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_published<'a>(
        &self,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1/$2 cursor, $3 post_id, $4 now, then the variant-sized
            // resolution fragment from $5 and the limit at `limit_idx`.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 5);
            // `published_at <= $4` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.published_at <= $4
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $1 OR (p.created_at = $2 AND p.post_id < $3))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id)
                .bind(now);
            binds
                .bind_onto(query)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 now, then the variant-sized resolution fragment from $2
            // and the limit at the returned `limit_idx`.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 2);
            // `published_at <= $1` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.published_at <= $1
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRecord>(&sql).bind(now);
            binds
                .bind_onto(query)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };
        Ok(rows)
    }

    #[tracing::instrument(
        name = "storage.posts.list_drafts_by_user",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_drafts_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // `published_at IS NULL OR published_at > $5` surfaces both true
            // drafts and scheduled (future-dated) posts to the author.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND (p.published_at IS NULL OR p.published_at > $5)
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $6"
            );
            sqlx::query_as::<_, PostRecord>(&sql)
                .bind(user_id)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id)
                .bind(now)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        } else {
            // `published_at IS NULL OR published_at > $2` surfaces both true
            // drafts and scheduled (future-dated) posts to the author.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND (p.published_at IS NULL OR p.published_at > $2)
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $3"
            );
            sqlx::query_as::<_, PostRecord>(&sql)
                .bind(user_id)
                .bind(now)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };
        Ok(rows)
    }

    #[tracing::instrument(
        name = "storage.posts.list_scheduled_by_user",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_scheduled_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a ScheduledPostCursor>,
        limit: RowLimit,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at > $5
                   AND p.deleted_at IS NULL
                   AND (p.published_at > $2 OR (p.published_at = $3 AND p.post_id > $4))
                 ORDER BY p.published_at ASC, p.post_id ASC
                 LIMIT $6"
            );
            sqlx::query_as::<_, PostRecord>(&sql)
                .bind(user_id)
                .bind(cursor.published_at)
                .bind(cursor.published_at)
                .bind(cursor.post_id)
                .bind(now)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        } else {
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at > $2
                   AND p.deleted_at IS NULL
                 ORDER BY p.published_at ASC, p.post_id ASC
                 LIMIT $3"
            );
            sqlx::query_as::<_, PostRecord>(&sql)
                .bind(user_id)
                .bind(now)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };
        Ok(rows)
    }

    #[tracing::instrument(
        name = "storage.posts.list_collection_by_user",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_collection_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a CollectionCursor>,
        limit: RowLimit,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.deleted_at IS NULL
                   AND (p.updated_at, p.post_id) < ($2, $3)
                 ORDER BY p.updated_at DESC, p.post_id DESC
                 LIMIT $4"
            );
            sqlx::query_as::<_, PostRecord>(&sql)
                .bind(user_id)
                .bind(cursor.updated_at)
                .bind(cursor.post_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        } else {
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.deleted_at IS NULL
                 ORDER BY p.updated_at DESC, p.post_id DESC
                 LIMIT $2"
            );
            sqlx::query_as::<_, PostRecord>(&sql)
                .bind(user_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };
        Ok(rows)
    }

    #[tracing::instrument(
        name = "storage.posts.set_post_tags",
        skip(self, desired),
        fields(db.system = DB::DB_SYSTEM, tag_count = desired.len())
    )]
    async fn set_post_tags(
        &self,
        post_id: PostId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError> {
        DB::set_post_tags(&self.pool, post_id, desired).await
    }

    #[tracing::instrument(
        name = "storage.posts.list_posts_by_tag",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_posts_by_tag<'a>(
        &self,
        tag_slug: &Tag,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug)
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1 tag, $2/$3 cursor, $4 post_id, $5 now, then the
            // variant-sized resolution fragment from $6 and the limit at
            // the returned `limit_idx`.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 6);
            // `published_at <= $5` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $5
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(tag_slug)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id)
                .bind(now);
            binds
                .bind_onto(query)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 tag, $2 now, then the variant-sized resolution fragment
            // from $3 and the limit at the returned `limit_idx`.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 3);
            // `published_at <= $2` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $2
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(tag_slug)
                .bind(now);
            binds
                .bind_onto(query)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };

        Ok(rows)
    }

    #[tracing::instrument(
        name = "storage.posts.list_user_posts_by_tag",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_user_posts_by_tag<'a>(
        &self,
        user_id: UserId,
        tag_slug: &Tag,
        cursor: Option<&'a PostCursor>,
        limit: RowLimit,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug)
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1 user_id, $2 tag, $3/$4 cursor, $5 post_id, $6 now, then
            // the variant-sized resolution fragment from $7 and the limit at
            // the returned `limit_idx`.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 7);
            // `published_at <= $6` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $6
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $3 OR (p.created_at = $4 AND p.post_id < $5))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(user_id)
                .bind(tag_slug)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id)
                .bind(now);
            binds
                .bind_onto(query)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 user_id, $2 tag, $3 now, then the variant-sized
            // resolution fragment from $4 and the limit at `limit_idx`.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 4);
            // `published_at <= $3` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $3
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(user_id)
                .bind(tag_slug)
                .bind(now);
            binds
                .bind_onto(query)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
        };

        Ok(rows)
    }

    #[tracing::instrument(
        name = "storage.posts.list_tags",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_tags<'a>(
        &self,
        prefix: Option<&'a str>,
        limit: RowLimit,
    ) -> sqlx::Result<Vec<TagRecord>> {
        let normalized = prefix
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_ascii_lowercase);
        let pattern = normalized.as_deref().map(|p| format!("{p}%"));

        let rows = match pattern {
            Some(ref like) => {
                sqlx::query_as::<_, TagListRow>(
                    "SELECT tag_id, tag_slug FROM tags
                     WHERE tag_slug LIKE $1
                     ORDER BY tag_slug
                     LIMIT $2",
                )
                // sqlx-newtype-bind:allow permanent-primitive — LIKE prefix pattern is derived query text, not a stored domain value.
                .bind(like.as_str())
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, TagListRow>(
                    "SELECT tag_id, tag_slug FROM tags
                     ORDER BY tag_slug
                     LIMIT $1",
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };

        // `tag_slug` decodes straight into `Tag` via the sqlx bridge (#438), so a
        // malformed stored value is rejected as a column-decode error above.
        Ok(rows
            .into_iter()
            .map(|(tag_id, tag_slug)| TagRecord { tag_id, tag_slug })
            .collect())
    }

    #[tracing::instrument(
        name = "storage.posts.list_published_in_window",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_published_in_window(
        &self,
        surface: &common::feed::FeedSurface,
        window: &common::feed::HybridWindow,
        now: DateTime<Utc>,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Vec<PostRecord>> {
        // ROW_NUMBER() identifies the top `min_items` posts; OR-combining with
        // `published_at >= cutoff` produces the hybrid-window union in a single
        // query. Only the JSON tag aggregation differs per backend, so the SQL
        // is shared via `DB::TAGS_SUBQUERY`.
        let cutoff = window.cutoff_date(now);
        let rows = list_published_in_window_rows::<DB>(
            &self.pool,
            surface,
            now,
            cutoff,
            window.min_items,
            viewer,
        )
        .await?;
        Ok(rows)
    }

    #[tracing::instrument(
        name = "storage.posts.list_posts_gone_live_between",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_posts_gone_live_between(
        &self,
        after: DateTime<Utc>,
        upto: DateTime<Utc>,
    ) -> sqlx::Result<Vec<GoLivePost>> {
        // `published_at > $1 AND published_at <= $2` selects exactly the posts
        // that crossed into "live" within the half-open window `(after, upto]`.
        // The standard post projection (incl. the JSON tag subquery) is reused so the row
        // decodes directly into `PostRecord`; we then keep only the username + tag slugs
        // the feed fan-out needs. No viewer filter: go-live regeneration is independent of
        // any reader's audience.
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
             ORDER BY p.published_at ASC, p.post_id ASC"
        );
        let rows = sqlx::query_as::<_, PostRecord>(&sql)
            .bind(after)
            .bind(upto)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|rec| GoLivePost {
                username: rec.author_username,
                tag_slugs: rec.tags.into_iter().map(|t| t.tag_slug).collect(),
            })
            .collect())
    }

    #[tracing::instrument(
        name = "storage.posts.feed_urls_needing_catchup",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn feed_urls_needing_catchup(&self, now: DateTime<Utc>) -> sqlx::Result<Vec<FeedPath>> {
        // Cached feeds live in the same database, so they are enumerated here
        // and, for each, the newest live post on that surface is compared
        // against the feed's own `generated_at`. Feed count is small, so a
        // per-feed check is simpler than a set-based join.
        //
        // Rows are read one at a time rather than via `query_as` so a single bad
        // `feed_url` cannot fail the whole scan — see the skip below.
        let rows = sqlx::query("SELECT feed_url, generated_at FROM feed_cache")
            .fetch_all(&self.pool)
            .await?;
        let mut needing = Vec::new();
        let mut decode_reported = false;
        for row in rows {
            let generated_at: DateTime<Utc> = row.try_get("generated_at")?;
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
                continue;
            };
            if let Some(max) = max_published_at_for_surface::<DB>(&self.pool, &surface, now).await?
                && max > generated_at
            {
                needing.push(feed_path);
            }
        }
        Ok(needing)
    }
}

/// The viewer-resolution binds folded into a read query's `WHERE`, in the exact
/// left-to-right order their placeholders appear in [`resolution_where`]'s
/// fragment. `subref` (and, where it is bound at all, `channel`) repeats —
/// subscribers branch, then named branch — because each occurrence gets its own
/// placeholder; see [`resolution_where`].
///
/// The enum mirrors [`ViewerIdentity`] rather than carrying three independent
/// `Option`s, because **bind arity is per-variant**: a `Local` viewer's channel
/// is resolved in SQL and so is not bound at all. Recovering that from three
/// `Option`s would mean reading `(Some, None, Some)` as "local" — an implicit
/// encoding of exactly the fact the variant already states.
enum ResolutionBinds {
    /// No viewer: all five placeholders bind SQL NULL, which makes every
    /// comparison *unknown* rather than true — see [`resolution_where`] for why
    /// that is what "this branch cannot match" means here.
    Anonymous,
    /// A local viewer: three binds — `author_id, subref, subref`. The channel is
    /// the seeded `local` row, resolved by subquery instead of bound.
    Local {
        /// `p.user_id = $author_id` — the author branch fires for this and only
        /// this variant.
        user_id: UserId,
        /// `s.subscriber_ref` for the subscribers/named branches: the viewer's
        /// user id in decimal, the form `subscribe_to` stores.
        subref: SubscriberRef,
    },
    /// A non-local viewer: five binds — `NULL, channel, subref, channel, subref`.
    /// The author placeholder binds NULL, so the author branch cannot fire (#6).
    Remote {
        /// `s.channel_id` for the subscribers/named `EXISTS` branches.
        channel: ChannelId,
        /// `s.subscriber_ref` for the subscribers/named branches.
        subref: SubscriberRef,
    },
}

/// The viewer-resolution predicate and its binds, for folding into a read
/// query's `WHERE`. A post is returned to `viewer` only if the viewer is the
/// author OR some targeted audience admits them. See ADR-0020, Task 13.
///
/// The fragment is emitted in full for every viewer; `Anonymous` is handled by
/// binding NULL for all three values, so it reduces to "public posts only"
/// without a second query shape. A NULL comparison is *unknown*, never true:
/// `p.user_id = NULL` cannot admit a post, and the `EXISTS` subqueries match no
/// row, so `EXISTS` is false. The fragment contains no `NOT`, and the caller
/// `AND`s it into a `WHERE`, where unknown filters the row out exactly as false
/// would — so NULL kills every non-`public` branch.
///
/// NULL binds make the unreachable branches follow from SQL comparison
/// semantics rather than from reserving sentinel IDs or subscriber references.
/// The predicate therefore stays correct independently of which concrete
/// values the live schema permits.
///
/// `start` is the next free `$n` index, and **the placeholder count is
/// per-variant**, so callers must thread the returned `next` rather than assume
/// `start + 5`:
///
/// - `Local` uses THREE (`$start`..`$start+2`): `author, subref, subref`. Its
///   channel is not a bind — a local viewer's channel is always the seeded
///   `local` row, so both subscription branches resolve it inline with an
///   uncorrelated subquery (`channels.name` is `NOT NULL UNIQUE`, so it yields at
///   most one row).
/// - `Anonymous` and `Remote` use FIVE (`$start`..`$start+4`):
///   `author, channel, subref, channel, subref`.
///
/// Either way the `channel`/`subref` pair appears once in the subscribers branch
/// and again in the named branch, and each bound occurrence gets its own number
/// so the binds are positional on both backends (`SQLite` accepts `$n` and binds
/// by position; see ADR-0019) — which is why the returned [`ResolutionBinds`]
/// carries `subref` once but the caller binds it **twice**. Returns
/// `(sql, binds, next)` where `next` is the first free index after the fragment.
fn resolution_where(viewer: &ViewerIdentity, start: usize) -> (String, ResolutionBinds, usize) {
    /// The seeded `local` channel, resolved in SQL rather than bound — see the
    /// doc above and ADR-0020.
    const LOCAL_CHANNEL: &str = "(SELECT channel_id FROM channels WHERE name = 'local')";

    let binds = match viewer {
        ViewerIdentity::Anonymous => ResolutionBinds::Anonymous,
        // Only a local viewer can be the author. Its channel is not carried at
        // all, because it can only ever be the `local` row, which the SQL
        // resolves for itself.
        ViewerIdentity::Local { user_id } => ResolutionBinds::Local {
            user_id: *user_id,
            subref: local_subscriber_ref(*user_id),
        },
        // A remote viewer is never the author, whatever its ref parses as: the
        // author bind stays NULL, so `p.user_id = NULL` is unknown and admits
        // nothing (#6). It can still be admitted by a subscription branch.
        ViewerIdentity::Remote {
            channel_id,
            subscriber_ref,
        } => ResolutionBinds::Remote {
            channel: *channel_id,
            subref: subscriber_ref.clone(),
        },
    };
    let author = start;
    // The channel slots are *expressions*, not necessarily placeholders, and the
    // ref slots renumber accordingly — hence the per-variant `next`.
    let (sub_channel, sub_refnum, named_channel, named_refnum, next) = match binds {
        ResolutionBinds::Local { .. } => (
            LOCAL_CHANNEL.to_owned(),
            format!("${}", start + 1),
            LOCAL_CHANNEL.to_owned(),
            format!("${}", start + 2),
            start + 3,
        ),
        ResolutionBinds::Anonymous | ResolutionBinds::Remote { .. } => (
            format!("${}", start + 1),
            format!("${}", start + 2),
            format!("${}", start + 3),
            format!("${}", start + 4),
            start + 5,
        ),
    };
    let sql = format!(
        "( p.user_id = ${author}
  OR EXISTS (
    SELECT 1 FROM post_audiences pa
    JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
    WHERE pa.post_id = p.post_id AND (
         tk.name = 'public'
      OR (tk.name = 'subscribers' AND EXISTS (
            SELECT 1 FROM subscriptions s JOIN subscription_statuses st ON st.status_id = s.status_id
            WHERE s.author_user_id = p.user_id AND s.channel_id = {sub_channel}
              AND s.subscriber_ref = {sub_refnum} AND st.name = 'active'))
      OR (tk.name = 'named' AND EXISTS (
            SELECT 1 FROM audience_members am
            JOIN subscriptions s ON s.subscription_id = am.subscription_id
            JOIN subscription_statuses st ON st.status_id = s.status_id
            WHERE am.audience_id = pa.audience_id AND s.channel_id = {named_channel}
              AND s.subscriber_ref = {named_refnum} AND st.name = 'active'))
  ))
)"
    );
    (sql, binds, next)
}

impl ResolutionBinds {
    /// Binds this variant's resolution placeholders onto `query` in the exact
    /// fragment order — `author_id, channel, subref, channel, subref`, minus the
    /// two channel binds for [`ResolutionBinds::Local`], whose channel is
    /// resolved in SQL. The caller must have already bound everything to the left
    /// of the fragment, and must bind the query's trailing binds (e.g. `LIMIT`)
    /// afterward, at the index [`resolution_where`] returned.
    fn bind_onto<'q, DB>(
        &'q self,
        query: sqlx::query::QueryAs<'q, DB, PostRecord, DB::Arguments<'q>>,
    ) -> sqlx::query::QueryAs<'q, DB, PostRecord, DB::Arguments<'q>>
    where
        DB: Database,
        i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        &'q SubscriberRef: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        Option<&'q SubscriberRef>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        // sqlx implements `Encode for Option<T>` per concrete database (the
        // `impl_encode_for_option!` macro), not blanket over a generic `DB`, so
        // each NULL-able bind's type has to be restated here — and, per ADR-0019,
        // again on every caller.
        Option<UserId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        Option<ChannelId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        Option<&'q str>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    {
        match self {
            Self::Anonymous => query
                .bind(None::<UserId>)
                .bind(None::<ChannelId>)
                .bind(None::<&SubscriberRef>)
                .bind(None::<ChannelId>)
                .bind(None::<&SubscriberRef>),
            Self::Local { user_id, subref } => query
                .bind(Some(*user_id))
                .bind(Some(subref))
                .bind(Some(subref)),
            Self::Remote { channel, subref } => query
                .bind(None::<UserId>)
                .bind(Some(*channel))
                .bind(Some(subref))
                .bind(Some(*channel))
                .bind(Some(subref)),
        }
    }
}

/// Maps an [`AudienceTarget`] to its `post_audiences` row shape:
/// `(target kind, audience_id)`. `Private` produces no row.
fn audience_target_row(target: &AudienceTarget) -> Option<(TargetKind, Option<AudienceId>)> {
    match target {
        AudienceTarget::Public => Some((TargetKind::Public, None)),
        AudienceTarget::Subscribers => Some((TargetKind::Subscribers, None)),
        AudienceTarget::Named(id) => Some((TargetKind::Named, Some(*id))),
        AudienceTarget::Private => None,
    }
}

/// Maps a `post_audiences` row `(target_kind name, audience_id)` back to its
/// [`AudienceTarget`] — the inverse of [`audience_target_row`], used by
/// [`PostStorage::get_post_audiences`].
///
/// `public` → [`AudienceTarget::Public`], `subscribers` →
/// [`AudienceTarget::Subscribers`], `named` (with an id) →
/// [`AudienceTarget::Named`].
///
/// **Returns `Option`, for one reason only.** A `named` row whose `audience_id` is
/// NULL has no target to build, so it is dropped — asserted below. An unrecognised
/// kind name never reaches this function: the column decodes as `TargetKind`
/// (#728), so it is a `ColumnDecode` error at the query boundary rather than a
/// silent drop here.
fn audience_target_from_row(
    kind: TargetKind,
    audience_id: Option<AudienceId>,
) -> Option<AudienceTarget> {
    match kind {
        TargetKind::Public => Some(AudienceTarget::Public),
        TargetKind::Subscribers => Some(AudienceTarget::Subscribers),
        TargetKind::Named => audience_id.map(AudienceTarget::Named),
    }
}

/// Maps an error from the idempotency-key `INSERT`. A `(user_id, key)` unique
/// violation is a [`CreatePostError::IdempotencyConflict`] (a duplicate create),
/// distinct from the post `INSERT`'s `SlugConflict` — attribution is by which
/// statement's mapper runs. Any other error passes through as `Internal`.
fn map_idempotency_insert_error(e: sqlx::Error) -> CreatePostError {
    match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            CreatePostError::IdempotencyConflict
        }
        e => CreatePostError::Internal(e),
    }
}

/// Writes one post row and its audience rows onto a caller-supplied transaction
/// connection, so it joins whatever transaction is open.
///
/// This is the single place that knows the post `INSERT` and the
/// unique-violation → [`CreatePostError::SlugConflict`] mapping: both
/// `create_post` (write one) and `create_posts` (write many in one transaction)
/// are pure transaction orchestration over it, so the row-write logic lives once
/// rather than being duplicated per arity.
pub(crate) async fn write_post_in_tx<DB>(
    conn: &mut DB::Connection,
    input: &CreatePostInput,
) -> Result<PostId, CreatePostError>
where
    DB: PostDialect,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<AudienceId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q str>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<String>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<DateTime<Utc>>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Slug`/`PostBody` bind as themselves and `PostTitle` as `Option<&PostTitle>`
    // via the ADR-0071 sqlx bridge (the `Option<&…>` pair covers the nullable
    // `title` bind).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> Option<&'q PostTitle>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `summary` binds as `Option<&PostSummary>` via the ADR-0071 sqlx bridge on
    // the create paths, mirroring the `Option<&PostTitle>` bound above.
    for<'q> Option<&'q PostSummary>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    (PostId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let now = Utc::now();

    let post_id = sqlx::query_scalar::<_, PostId>(
        "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at, summary)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING post_id",
    )
    .bind(input.user_id)
    // `Option::as_ref` → `Option<&PostTitle>` (a typed newtype bind, not an
    // `AsRef<str>` strip); the sqlx bridge encodes `Option<&PostTitle>`.
    .bind(input.title.as_ref())
    .bind(&input.slug)
    .bind(&input.body)
    .bind(input.format)
    .bind(input.rendered.html())
    .bind(now)
    .bind(now)
    .bind(input.published_at)
    // `Option::as_ref` → `Option<&PostSummary>` (a typed newtype bind via the
    // ADR-0071 sqlx bridge, not an `AsRef<str>` strip); the `sqlx-newtype-bind`
    // gate forbids stripping to `&str` here.
    .bind(input.summary.as_ref())
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => CreatePostError::SlugConflict,
        e => CreatePostError::Internal(e),
    })?;

    replace_post_audiences::<DB>(conn, post_id, &input.audiences).await?;
    replace_post_media::<DB>(conn, post_id, input.rendered.media()).await?;

    // Register the idempotency key in the same transaction as the post. This
    // INSERT has its own unique-violation mapping — a `(user_id, key)` clash is
    // an `IdempotencyConflict` (a duplicate create), distinct from the post
    // INSERT's `SlugConflict` above. Attribution is by which statement's
    // `map_err` fires, not by inspecting the constraint name.
    if let Some(key) = input.idempotency_key.as_deref() {
        sqlx::query("INSERT INTO idempotency_keys (user_id, key, post_id) VALUES ($1, $2, $3)")
            .bind(input.user_id)
            .bind(key)
            .bind(post_id)
            .execute(&mut *conn)
            .await
            .map_err(map_idempotency_insert_error)?;
    }

    Ok(post_id)
}

/// Replaces a post's `post_audiences` rows to exactly match `audiences`.
///
/// Deletes every existing row for `post_id`, then inserts one row per targeting
/// entry (`Public`/`Subscribers` carry a NULL `audience_id`; `Named(id)` carries
/// the id; `Private` and an empty vec leave the post with no rows). Runs on the
/// caller's executor so it shares the create/update transaction. See ADR-0020.
pub(crate) async fn replace_post_audiences<DB>(
    conn: &mut DB::Connection,
    post_id: PostId,
    audiences: &[AudienceTarget],
) -> sqlx::Result<()>
where
    DB: PostDialect,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<AudienceId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> TargetKind: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    sqlx::query(DB::DELETE_POST_AUDIENCES)
        .bind(post_id)
        .execute(&mut *conn)
        .await?;
    for target in audiences {
        if let Some((kind, audience_id)) = audience_target_row(target) {
            sqlx::query(DB::INSERT_POST_AUDIENCE)
                .bind(post_id)
                .bind(audience_id)
                .bind(kind)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// Replaces a post's `post_media` rows to exactly match `media`.
///
/// Deletes every existing row for `post_id`, then inserts one per reference, so an
/// edit that *removes* an embed removes its row. Runs on the caller's executor so it
/// shares the create/update transaction — the sibling of [`replace_post_audiences`],
/// and kept beside it at every call site because they are one concern: a post's child
/// rows (#711).
///
/// `media` is [`RenderOutput::media`](common::render::RenderOutput::media), which is
/// already deduplicated and sorted, so the composite primary key can never be violated
/// and no dialect-divergent conflict handling is needed.
pub(crate) async fn replace_post_media<DB>(
    conn: &mut DB::Connection,
    post_id: PostId,
    media: &[MediaRef],
) -> sqlx::Result<()>
where
    DB: PostDialect,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `MediaSource`/`ContentHash`/`Filename` all bind as themselves through the shared
    // sqlx bridge (ADR-0071), which is what these bounds make available on the generic
    // backend. The `sqlx-newtype-bind` gate forbids stripping any of them to `&str` here.
    //
    // The newtypes delegate `Encode` to `String`; `MediaSource` delegates to `&'q str`,
    // because a `#[text_enum]` token is a `&'static str` and encoding it needs no
    // allocation (#746 D4). Hence both pairs — the `&'q str` one is the same bound the
    // other generic binders in this module already carry.
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    sqlx::query(DB::DELETE_POST_MEDIA)
        .bind(post_id)
        .execute(&mut *conn)
        .await?;
    for reference in media {
        sqlx::query(DB::INSERT_POST_MEDIA)
            .bind(post_id)
            .bind(reference.source)
            .bind(&reference.sha256)
            .bind(&reference.filename)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Runs the hybrid-window query for `surface`, returning [`PostRecord`]s.
///
/// Shared across backends: the four `FeedSurface` variants differ only in the
/// ranked-CTE source/predicate and bind list, and the JSON tag aggregation is
/// supplied by [`PostDialect::TAGS_SUBQUERY`].
async fn list_published_in_window_rows<DB>(
    pool: &Pool<DB>,
    surface: &common::feed::FeedSurface,
    now: DateTime<Utc>,
    cutoff: DateTime<Utc>,
    min_items: common::feed::FeedMinItems,
    viewer: &ViewerIdentity,
) -> sqlx::Result<Vec<PostRecord>>
where
    DB: PostDialect,
    PostRecord: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> common::feed::FeedMinItems: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // The viewer-resolution binds are NULL-able (`ResolutionBinds::bind_onto`).
    for<'q> Option<UserId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<ChannelId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q SubscriberRef: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q SubscriberRef>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q str>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Username`/`Tag` bind as themselves via the ADR-0071 sqlx bridge, for the
    // surface `username`/`tag` binds.
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    use common::feed::FeedSurface;
    let tags = DB::TAGS_SUBQUERY;
    match surface {
        FeedSurface::Site => {
            // Binds: $1 now, $2 min_items, $3 cutoff, then the variant-sized
            // resolution fragment from $4. `window_sql` places it last, so
            // nothing binds after it and the returned `next` is discarded.
            let (resolution, binds, _) = resolution_where(viewer, 4);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(now)
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::User { username } => {
            // Binds: $1 now, $2 username, $3 min_items, $4 cutoff, then the
            // variant-sized resolution fragment last, from $5.
            let (resolution, binds, _) = resolution_where(viewer, 5);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(now)
                .bind(username)
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::SiteTag { tag } => {
            // Binds: $1 now, $2 tag, $3 min_items, $4 cutoff, then the
            // variant-sized resolution fragment last, from $5.
            let (resolution, binds, _) = resolution_where(viewer, 5);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(now)
                .bind(tag)
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::UserTag { username, tag } => {
            // Binds: $1 now, $2 username, $3 tag, $4 min_items, $5 cutoff, then
            // the variant-sized resolution fragment last, from $6.
            let (resolution, binds, _) = resolution_where(viewer, 6);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind(now)
                .bind(username)
                .bind(tag)
                .bind(min_items)
                .bind(cutoff);
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
 )
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $2 OR r.published_at >= $3)
   AND {resolution}
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
 )
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $3 OR r.published_at >= $4)
   AND {resolution}
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
 )
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $3 OR r.published_at >= $4)
   AND {resolution}
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
 )
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $4 OR r.published_at >= $5)
   AND {resolution}
 ORDER BY p.published_at DESC, p.post_id DESC"
        ),
    }
}

/// The most recent `published_at` of a *live* post (`published_at <= now`, not
/// deleted) on `surface`, or `None` when the surface has no live post. Each
/// surface variant adds exactly the joins/predicates that define its post set,
/// mirroring the window query's surface filters. Used by
/// [`PostStorage::feed_urls_needing_catchup`] to detect a cached feed that is
/// stale relative to a go-live the worker may have missed while down.
async fn max_published_at_for_surface<DB>(
    pool: &Pool<DB>,
    surface: &common::feed::FeedSurface,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<DateTime<Utc>>>
where
    DB: PostDialect,
    (DateTime<Utc>,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Username`/`Tag` bind as themselves via the ADR-0071 sqlx bridge, for the
    // surface `username`/`tag` binds.
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    use common::feed::FeedSurface;
    let row: Option<(DateTime<Utc>,)> = match surface {
        FeedSurface::Site => {
            sqlx::query_as(
                "SELECT p.published_at FROM posts p
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL
                 ORDER BY p.published_at DESC LIMIT 1",
            )
            .bind(now)
            .fetch_optional(pool)
            .await?
        }
        FeedSurface::User { username } => {
            sqlx::query_as(
                "SELECT p.published_at FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL AND u.username = $2
                 ORDER BY p.published_at DESC LIMIT 1",
            )
            .bind(now)
            .bind(username)
            .fetch_optional(pool)
            .await?
        }
        FeedSurface::SiteTag { tag } => {
            sqlx::query_as(
                "SELECT p.published_at FROM posts p
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL AND t.tag_slug = $2
                 ORDER BY p.published_at DESC LIMIT 1",
            )
            .bind(now)
            .bind(tag)
            .fetch_optional(pool)
            .await?
        }
        FeedSurface::UserTag { username, tag } => {
            sqlx::query_as(
                "SELECT p.published_at FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL AND u.username = $2 AND t.tag_slug = $3
                 ORDER BY p.published_at DESC LIMIT 1",
            )
            .bind(now)
            .bind(username)
            .bind(tag)
            .fetch_optional(pool)
            .await?
        }
    };
    Ok(row.map(|(published_at,)| published_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed_cache::FeedCacheRow;
    use crate::test_support::{
        Backend, CloseablePool, MEDIA_TEST_SHA256, SeedRawPost, SeedUser, TestEnv, UpdateRawPost,
        backends, create_draft_via_service, create_post_via_service, fetch_post_media, fp,
        media_ref_for, media_row_exists, media_url_for, seed_media, seed_users,
        update_post_body_via_service,
    };
    use common::test_support::{
        parse_content_type, parse_etag, parse_post_body, parse_post_summary, parse_post_title,
        parse_row_limit, parse_slug, parse_tag, parse_tag_label, parse_username,
    };
    use rstest::*;
    use rstest_reuse::*;
    use std::sync::Arc;
    use std::time::Duration;

    /// Guards the two dialect constants against drifting apart — the failure mode
    /// where one is edited and the other forgotten (#772; the rationale for the
    /// Postgres `COLLATE "C"` lives on [`PostDialect::TAGS_SUBQUERY`]).
    ///
    /// Deliberately a *sync* check, not a semantic one: it proves both constants
    /// carry the clause, not that either is positioned correctly inside the
    /// aggregate. Placement is proven behaviourally, on both backends, by
    /// `post_record_carries_tags` and
    /// `regenerated_json_feed_carries_slug_ordered_tags`.
    #[test]
    fn tags_subquery_pins_slug_ordering_on_both_dialects() {
        let sqlite = <sqlx::Sqlite as PostDialect>::TAGS_SUBQUERY;
        let postgres = <sqlx::Postgres as PostDialect>::TAGS_SUBQUERY;
        assert!(
            sqlite.contains("ORDER BY t.tag_slug"),
            "sqlite TAGS_SUBQUERY must order by slug: {sqlite}"
        );
        assert!(
            postgres.contains("ORDER BY t.tag_slug COLLATE \"C\""),
            "postgres TAGS_SUBQUERY must order by slug under C collation: {postgres}"
        );
    }

    /// A local viewer's channel is not a bind: both subscription branches resolve
    /// the seeded `local` row inline, so the fragment spends three placeholders
    /// (author, ref, ref) and `next` is `start + 3` — the property every call site
    /// depends on by threading the returned index rather than assuming `+5` (#6).
    #[test]
    fn resolution_where_resolves_the_local_channel_in_sql_for_a_local_viewer() {
        let viewer = ViewerIdentity::Local {
            user_id: UserId::from(7),
        };
        let (sql, binds, next) = resolution_where(&viewer, 2);
        assert!(matches!(binds, ResolutionBinds::Local { .. }));
        assert_eq!(next, 5, "three placeholders consumed from $2: {sql}");
        assert_eq!(
            sql.matches("(SELECT channel_id FROM channels WHERE name = 'local')")
                .count(),
            2,
            "both the subscribers and named branches resolve the channel: {sql}"
        );
        assert!(sql.contains("p.user_id = $2"), "{sql}");
        assert!(sql.contains("s.subscriber_ref = $3"), "{sql}");
        assert!(sql.contains("s.subscriber_ref = $4"), "{sql}");
        assert!(
            !sql.contains("$5"),
            "no fourth placeholder is emitted: {sql}"
        );
        assert!(
            !sql.contains("99"),
            "the carried channel id is ignored: {sql}"
        );
    }

    /// The counterpart: `Anonymous` and `Remote` keep the five-placeholder shape,
    /// binding the channel rather than resolving it.
    #[rstest]
    #[case::anonymous(ViewerIdentity::Anonymous)]
    #[case::remote(ViewerIdentity::Remote {
        channel_id: ChannelId::from(2),
        subscriber_ref: "7".parse().unwrap(),
    })]
    fn resolution_where_binds_the_channel_for_a_non_local_viewer(#[case] viewer: ViewerIdentity) {
        let (sql, _binds, next) = resolution_where(&viewer, 2);
        assert_eq!(next, 7, "five placeholders consumed from $2: {sql}");
        assert!(
            !sql.contains("name = 'local'"),
            "a non-local viewer never resolves the local channel: {sql}"
        );
        assert!(sql.contains("s.channel_id = $3"), "{sql}");
        assert!(sql.contains("s.subscriber_ref = $4"), "{sql}");
        assert!(sql.contains("s.channel_id = $5"), "{sql}");
        assert!(sql.contains("s.subscriber_ref = $6"), "{sql}");
    }

    /// Physical row identity for the post's `post_tags` rows: `ctid` on Postgres,
    /// `rowid` on `SQLite`. Column values cannot serve — a DELETE+INSERT
    /// reproduces `tag_id`/`tag_display` exactly, which is exactly what the
    /// no-write-when-unchanged test must detect.
    async fn physical_row_ids(env: &TestEnv, post_id: PostId) -> Vec<String> {
        match env.base.pool() {
            CloseablePool::Postgres(pool) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT ctid::text FROM post_tags WHERE post_id = $1 ORDER BY tag_id",
                )
                .bind(post_id)
                .fetch_all(pool)
                .await
            }
            CloseablePool::Sqlite(pool) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT CAST(rowid AS TEXT) FROM post_tags WHERE post_id = $1 ORDER BY tag_id",
                )
                .bind(post_id)
                .fetch_all(pool)
                .await
            }
        }
        .expect("read physical row ids")
    }

    /// The post's tag slugs, slug-ordered, read through the normal post read path.
    async fn slugs_of(posts: &dyn PostStorage, post_id: PostId) -> Vec<String> {
        posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .expect("read post")
            .expect("post exists")
            .tags
            .iter()
            .map(|t| t.tag_slug.to_string())
            .collect()
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_adds_removes_and_clears(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        posts
            .set_post_tags(post, &[parse_tag_label("rust"), parse_tag_label("web")])
            .await
            .expect("set initial tags");
        assert_eq!(slugs_of(posts, post).await, vec!["rust", "web"]);

        // Reconcile: "web" drops, "nix" arrives, "rust" stays.
        posts
            .set_post_tags(post, &[parse_tag_label("rust"), parse_tag_label("nix")])
            .await
            .expect("reconcile tags");
        assert_eq!(slugs_of(posts, post).await, vec!["nix", "rust"]);

        // An empty desired set clears; it is deliberately NOT a no-op, unlike
        // `enqueue_many`'s empty-input early return (#771).
        posts.set_post_tags(post, &[]).await.expect("clear tags");
        assert!(slugs_of(posts, post).await.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_preserves_existing_display_casing(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        posts
            .set_post_tags(post, &[parse_tag_label("Rust")])
            .await
            .expect("initial casing");
        // Same slug, different casing: the stored row is left untouched, so the
        // original casing survives.
        posts
            .set_post_tags(post, &[parse_tag_label("rUsT")])
            .await
            .expect("re-apply with new casing");

        let record = posts
            .get_post_by_id(post, &ViewerIdentity::Anonymous)
            .await
            .expect("read post")
            .expect("post exists");
        assert_eq!(record.tags.len(), 1);
        assert_eq!(record.tags[0].tag_display, "Rust");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_is_idempotent_and_absorbs_duplicate_slugs(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        let desired = [parse_tag_label("rust"), parse_tag_label("web")];
        posts.set_post_tags(post, &desired).await.expect("first");
        posts.set_post_tags(post, &desired).await.expect("second");
        assert_eq!(slugs_of(posts, post).await, vec!["rust", "web"]);

        // `post_tag_diff` does not dedupe its input, so two labels sharing a slug
        // both reach the insert; the conflict-tolerant insert absorbs the second
        // and the first occurrence's casing wins.
        posts
            .set_post_tags(post, &[parse_tag_label("Nix"), parse_tag_label("nix")])
            .await
            .expect("duplicate slug in desired");
        let record = posts
            .get_post_by_id(post, &ViewerIdentity::Anonymous)
            .await
            .expect("read post")
            .expect("post exists");
        assert_eq!(record.tags.len(), 1);
        assert_eq!(record.tags[0].tag_display, "Nix");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_rejects_missing_post_but_allows_soft_deleted(#[case] backend: Backend) {
        let env = backend.setup().await;
        let posts = &*env.state.posts;

        let err = posts
            .set_post_tags(PostId::from(999_999), &[parse_tag_label("rust")])
            .await
            .expect_err("missing post must be rejected");
        assert!(matches!(err, TaggingError::PostNotFound));

        // Soft-deleted posts stay taggable: the exists-check deliberately carries
        // no `deleted_at` filter.
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        posts.soft_delete_post(post).await.expect("soft delete");
        posts
            .set_post_tags(post, &[parse_tag_label("rust")])
            .await
            .expect("tagging a soft-deleted post still succeeds");
    }

    /// #339: `set_post_tags` must take its write lock **before** snapshotting the
    /// tags it diffs against, and hold it through the writes — so two writers on
    /// one post serialize and the committed result is exactly the desired set.
    ///
    /// The interleave is forced, not raced: the test holds the same lock
    /// `set_post_tags` takes and acts as a rival writer. A hopeful two-task race
    /// would pass or fail on scheduling and prove nothing.
    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_locks_before_snapshotting(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;

        env.state
            .posts
            .set_post_tags(post, &[parse_tag_label("alpha")])
            .await
            .expect("seed tags");

        // The rival writer: holds the post write lock and adds "beta", uncommitted.
        let mut rival = env
            .base
            .pool()
            .lock_post_for_write(post)
            .await
            .expect("take post write lock");
        rival
            .add_tag(&parse_tag_label("beta"))
            .await
            .expect("rival adds a tag");

        // Two pooled connections are live at once — this one and the spawned
        // call's — so the pool must allow >= 2. sqlx's default max_connections is
        // 10 and neither backend overrides it; at 1 this would deadlock, not fail.
        //
        // This is also safe under the current-thread runtime `#[tokio::test]`
        // defaults to: sqlx-sqlite runs each connection on its own OS thread
        // (docs/adr/0126-sqlx-sqlite-busy-handler-threading.md).
        let posts = Arc::clone(&env.state.posts);
        let mut racer =
            tokio::spawn(
                async move { posts.set_post_tags(post, &[parse_tag_label("gamma")]).await },
            );

        // PRECONDITION, not the regression guard: this proves mutual exclusion
        // exists at all. A read-then-lock implementation still blocks here on its
        // writes, so this assertion alone does not catch it — the final one does.
        //
        // 300ms sits well inside SQLite's 5s busy_timeout
        // (storage/src/sqlite/mod.rs), so a correct implementation is still
        // retrying — not failing with SQLITE_BUSY — when the lock is released below.
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut racer)
                .await
                .is_err(),
            "set_post_tags completed while another writer held the post write lock; \
             its read-diff-write is not serialized (#339)"
        );

        rival.commit().await.expect("rival commits");
        racer
            .await
            .expect("racer task panicked")
            .expect("set_post_tags failed");

        // THE REGRESSION GUARD. A correct implementation snapshots after the lock
        // is granted, so it sees {alpha, beta}, puts both in `to_remove`, and
        // leaves exactly {gamma}. A read-then-lock implementation snapshots
        // {alpha} before the rival commits, never removes "beta", and leaves
        // {beta, gamma}.
        assert_eq!(slugs_of(&*env.state.posts, post).await, vec!["gamma"]);
    }

    /// #883: the upsert returns the tag id on its **conflict** path, not just when
    /// it inserts. A `DO UPDATE` → `DO NOTHING` regression makes `RETURNING` emit
    /// no row, so `fetch_one` fails — and this is the test that says why.
    ///
    /// Cross-post deliberately: the second post's tag already exists in `tags`, so
    /// the upsert can only take the conflict path.
    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_reuses_an_existing_tag_across_posts(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let first = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let second = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        posts
            .set_post_tags(first, &[parse_tag_label("rust")])
            .await
            .expect("first post takes the insert path");
        posts
            .set_post_tags(second, &[parse_tag_label("rust")])
            .await
            .expect("second post takes the conflict path");

        assert_eq!(slugs_of(posts, first).await, vec!["rust"]);
        assert_eq!(slugs_of(posts, second).await, vec!["rust"]);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_with_unchanged_set_writes_nothing(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        let desired = [parse_tag_label("rust"), parse_tag_label("web")];
        posts
            .set_post_tags(post, &desired)
            .await
            .expect("seed tags");

        // Decoy: seeded second, so on SQLite its post_tags rows occupy HIGHER
        // rowids. Without it, `max(rowid)+1` would hand the target's rows their
        // original rowids back after a delete-and-reinsert and this test would
        // pass against the very implementation it exists to reject.
        let decoy = SeedRawPost::new(user).seed(&env.state).await.post_id;
        posts
            .set_post_tags(
                decoy,
                &[parse_tag_label("decoy-a"), parse_tag_label("decoy-b")],
            )
            .await
            .expect("seed decoy");

        let before = physical_row_ids(&env, post).await;
        posts
            .set_post_tags(post, &desired)
            .await
            .expect("re-apply the identical set");
        let after = physical_row_ids(&env, post).await;

        assert_eq!(
            before, after,
            "rows were rewritten; set_post_tags must leave unchanged tags physically untouched"
        );
    }

    #[test]
    fn map_idempotency_insert_error_passes_non_unique_errors_through() {
        // A unique violation becomes IdempotencyConflict (covered by the create
        // dedup integration test); any other error passes through as Internal.
        let mapped = map_idempotency_insert_error(sqlx::Error::PoolClosed);
        assert!(matches!(mapped, CreatePostError::Internal(_)));
    }

    #[test]
    fn audience_target_from_row_maps_every_kind() {
        // Each lookup-table kind maps to its target; `named` carries the id.
        assert_eq!(
            audience_target_from_row(TargetKind::Public, None),
            Some(AudienceTarget::Public)
        );
        assert_eq!(
            audience_target_from_row(TargetKind::Subscribers, None),
            Some(AudienceTarget::Subscribers)
        );
        assert_eq!(
            audience_target_from_row(TargetKind::Named, Some(AudienceId::from(7))),
            Some(AudienceTarget::Named(AudienceId::from(7)))
        );
        // A `named` row missing its id is dropped — the only reason this returns
        // `Option`.
        assert_eq!(audience_target_from_row(TargetKind::Named, None), None);
        // An unrecognised kind name is not expressible here: the parameter is a
        // `TargetKind`, so a bad name cannot get this far.
        // `get_post_audiences_rejects_an_unknown_target_kind` covers it at the
        // boundary where it surfaces.
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_post_audiences_rejects_an_unknown_target_kind(#[case] backend: Backend) {
        // Decoding `tk.name` as `TargetKind` surfaces an unrecognised value as a query
        // error (#728). It must not be silently dropped — that would lose an audience
        // row with no error and no log.
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let post = SeedRawPost::new(author)
            .audiences(vec![AudienceTarget::Public])
            .seed(state)
            .await;
        assert_eq!(
            state.posts.get_post_audiences(post.post_id).await.unwrap(),
            vec![AudienceTarget::Public],
            "precondition: the audience reads back before tampering"
        );

        // Only reachable by DB tampering or a migration that renames a lookup row.
        env.base
            .pool()
            .execute("UPDATE target_kinds SET name = 'bogus' WHERE name = 'public'")
            .await
            .unwrap();

        let err = state
            .posts
            .get_post_audiences(post.post_id)
            .await
            .unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "an unrecognised kind must surface as a decode error, not a shorter list: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn continuation_reporting_feed_urls_needing_catchup_skips_a_row_whose_feed_url_no_longer_parses(
        #[case] backend: Backend,
    ) {
        // A `feed_url` that will not decode into a `FeedPath` must cost only its own
        // row (docs/adr/0122-one-bad-row-must-not-stop-the-scan.md).
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let now = Utc::now();
        SeedRawPost::new(author).published_at(now).seed(state).await;

        // Two stale cached feeds, both older than the post above, so both would need
        // catch-up if they were readable.
        let stale = now - chrono::Duration::hours(1);
        for url in ["/feed.rss", "/feed.atom"] {
            state
                .feed_cache
                .upsert(FeedCacheRow {
                    feed_path: fp(url),
                    body: "<rss/>".into(),
                    etag: parse_etag("\"sha256-deadbeef\""),
                    content_type: parse_content_type("application/rss+xml"),
                    updated_at: stale,
                    generated_at: stale,
                })
                .await
                .unwrap();
        }
        // Only reachable by DB tampering or a row written under a looser grammar:
        // `FeedPath`'s validating bridge rejects this on read.
        env.base
            .pool()
            .execute(
                "UPDATE feed_cache SET feed_url = 'not-a-feed-path' WHERE feed_url = '/feed.atom'",
            )
            .await
            .unwrap();
        env.base
            .pool()
            .execute(
                "INSERT INTO feed_cache \
                 (feed_url, body, etag, content_type, updated_at, generated_at) \
                 SELECT 'also-not-a-feed-path', body, etag, content_type, updated_at, generated_at \
                 FROM feed_cache WHERE feed_url = 'not-a-feed-path'",
            )
            .await
            .unwrap();

        let (needing, trace) = crate::helpers::swallowed_test::capture_async(
            state.posts.feed_urls_needing_catchup(now),
        )
        .await;
        let needing = needing.unwrap();

        assert_eq!(
            needing,
            vec![fp("/feed.rss")],
            "the readable stale feed is still reported, and the corrupt row is skipped \
             rather than failing the whole scan"
        );
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.feed_cache.decode_feed_path",
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn continuation_reporting_post_not_found_results_survive_injected_rollback_failures(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let author = SeedUser::new().seed(&env.state).await.user_id;
        let missing_post = PostId::from(999_999);
        let update_input = UpdateRawPost::new("missing").build();

        let update_primary = env
            .state
            .posts
            .update_post(missing_post, author, &update_input)
            .await;
        assert!(matches!(&update_primary, Err(UpdatePostError::NotFound)));
        let (update, trace) = crate::helpers::swallowed_test::capture(|| match backend {
            Backend::Sqlite => crate::sqlite::posts::finish_post_update(
                update_primary,
                Err(sqlx::Error::PoolClosed),
            ),
            Backend::Postgres => crate::postgres::posts::finish_post_update_rejection(
                update_primary,
                Err(sqlx::Error::PoolClosed),
            ),
        });
        assert!(matches!(update, Err(UpdatePostError::NotFound)));
        let update_context = match backend {
            Backend::Sqlite => "storage.sqlite.post_update.rollback",
            Backend::Postgres => "storage.postgres.post_update.rollback_domain_rejection",
        };
        crate::helpers::swallowed_test::assert_one_report(&trace, update_context);

        let tags_primary = env
            .state
            .posts
            .set_post_tags(missing_post, &[parse_tag_label("rust")])
            .await;
        assert!(matches!(&tags_primary, Err(TaggingError::PostNotFound)));
        let (tags, trace) = crate::helpers::swallowed_test::capture(|| match backend {
            Backend::Sqlite => {
                crate::sqlite::posts::finish_post_tags(tags_primary, Err(sqlx::Error::PoolClosed))
            }
            Backend::Postgres => crate::postgres::posts::finish_post_tags_not_found(
                tags_primary,
                Err(sqlx::Error::PoolClosed),
            ),
        });
        assert!(matches!(tags, Err(TaggingError::PostNotFound)));
        let tags_context = match backend {
            Backend::Sqlite => "storage.sqlite.post_tags.rollback",
            Backend::Postgres => "storage.postgres.post_tags.rollback_not_found",
        };
        crate::helpers::swallowed_test::assert_one_report(&trace, tags_context);
    }

    fn post_tag(slug: &str, display: &str) -> PostTag {
        PostTag {
            post_id: PostId::from(1),
            tag_id: TagId::from(0),
            tag_slug: parse_tag(slug),
            tag_display: parse_tag_label(display),
        }
    }

    #[test]
    fn post_tag_diff_adds_removes_keeps() {
        let existing = vec![post_tag("rust", "Rust"), post_tag("leptos", "Leptos")];
        let desired: Vec<TagLabel> = vec![
            // Same slug as an existing tag (different casing): kept, not re-added.
            parse_tag_label("Rust"),
            // New slug: added.
            parse_tag_label("wasm"),
        ];

        let diff = post_tag_diff(&existing, &desired);

        let added: Vec<String> = diff.to_add.iter().map(ToString::to_string).collect();
        assert_eq!(added, vec!["wasm".to_string()]);
        let removed: Vec<String> = diff.to_remove.iter().map(ToString::to_string).collect();
        assert_eq!(removed, vec!["leptos".to_string()]);
    }

    /// `to_add` comes back slug-ordered so concurrent reconciles take `tags` row locks
    /// in a consistent order (#876) — and the sort is **stable**, so two labels sharing
    /// a slug keep their input order and the first occurrence's casing survives to the
    /// insert.
    #[test]
    fn post_tag_diff_orders_additions_by_slug_stably() {
        let existing: Vec<PostTag> = vec![];
        // Deliberately unordered, with a duplicate slug in the middle: if the sort were
        // unstable, "NIX" could overtake "Nix" and the wrong casing would be stored.
        let desired: Vec<TagLabel> = vec![
            parse_tag_label("wasm"),
            parse_tag_label("Nix"),
            parse_tag_label("NIX"),
            parse_tag_label("actix"),
        ];

        let diff = post_tag_diff(&existing, &desired);

        let added: Vec<String> = diff.to_add.iter().map(ToString::to_string).collect();
        assert_eq!(
            added,
            vec![
                "actix".to_string(),
                "Nix".to_string(),
                "NIX".to_string(),
                "wasm".to_string(),
            ],
            "additions are slug-ordered, and the duplicate slug keeps its input order"
        );
    }

    #[test]
    fn tagging_error_display_post_not_found() {
        let err = TaggingError::PostNotFound;
        assert_eq!(err.to_string(), "post not found");
    }

    #[test]
    fn tagging_error_debug() {
        let err = TaggingError::PostNotFound;
        let debug_str = format!("{err:?}");
        assert!(debug_str.contains("PostNotFound"));

        let err2 = TaggingError::Internal(sqlx::Error::RowNotFound);
        let debug_str2 = format!("{err2:?}");
        assert!(debug_str2.contains("Internal"));
    }

    #[test]
    fn list_by_tag_error_display_tag_not_found() {
        let err = ListByTagError::TagNotFound;
        assert_eq!(err.to_string(), "tag not found");
    }

    #[test]
    fn list_by_tag_error_debug() {
        let err = ListByTagError::TagNotFound;
        let debug_str = format!("{err:?}");
        assert!(debug_str.contains("TagNotFound"));
    }

    #[test]
    fn fallback_summary_label_uses_the_first_non_blank_body_line() {
        let post = PostRecord {
            post_id: PostId::from(1),
            user_id: UserId::from(1),
            author_username: parse_username("author"),
            title: Some(parse_post_title("My Title")),
            slug: parse_slug("my-slug"),
            body: parse_post_body(
                "\n\n   The first non-empty line of the body is here. \n\n Another line.",
            ),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted(
                "<p>The first non-empty line of the body is here.</p>",
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            published_at: None,
            deleted_at: None,
            summary: None,
            tags: vec![],
        };

        assert_eq!(
            post.fallback_summary_label(),
            "The first non-empty line of the body is here."
        );

        // A `PostBody` always has a non-blank line (#811), so the body rung always
        // answers and `fallback_summary_label` needs no title/slug fallbacks — they
        // would be dead compensation.
    }

    #[test]
    fn permalink_formats_username_date_and_slug() {
        use chrono::TimeZone;
        let post = PostRecord {
            post_id: PostId::from(1),
            user_id: UserId::from(1),
            author_username: parse_username("author"),
            title: Some(parse_post_title("My Title")),
            slug: parse_slug("hello-world"),
            body: parse_post_body("My body"),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>My body</p>"),
            created_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            published_at: Some(Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap()),
            deleted_at: None,
            summary: None,
            tags: vec![],
        };

        assert_eq!(post.permalink().as_ref(), "/~author/2026/04/12/hello-world");
    }

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
            rendered_html: RenderedHtml::from_trusted("<p>My body</p>"),
            created_at: Utc::now(),
            updated_at: Utc::now(),
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

    #[apply(backends)]
    #[tokio::test]
    async fn create_post_persists_summary(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .summary(parse_post_summary("the summary"))
            .seed(&env.state)
            .await
            .post_id;
        let post = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(post.summary, Some(parse_post_summary("the summary")));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn update_post_persists_and_clears_summary(#[case] backend: Backend) {
        // `update_post` writes the `summary` column — omitting it from the SET clause
        // would silently drop an edited summary. An edit replaces the value; `None`
        // clears it. The returned record reflects the RETURNING row.
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;

        // Seed with an initial summary so the first edit exercises replace-an-existing
        // value (not set-from-none); the second edit then clears it.
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .summary(parse_post_summary("original summary"))
            .seed(&env.state)
            .await
            .post_id;

        let update = |summary: Option<PostSummary>| {
            UpdateRawPost::new("summary-edit")
                .title("Test Title")
                .body(parse_post_body("Test body"))
                .summary(summary)
                .build()
        };

        // An edit replaces the summary.
        let changed = posts
            .update_post(
                post_id,
                user_id,
                &update(Some(parse_post_summary("edited summary"))),
            )
            .await
            .unwrap();
        assert_eq!(changed.summary, Some(parse_post_summary("edited summary")));

        // `None` clears it.
        let cleared = posts
            .update_post(post_id, user_id, &update(None))
            .await
            .unwrap();
        assert_eq!(cleared.summary, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publish_post_changes_only_the_publication_timestamp(#[case] backend: Backend) {
        // Publishing is not an edit (#711): it stamps `published_at` and touches
        // nothing else — body, rendered HTML, format, slug, title, summary, tags and
        // audience targeting all survive. Routing publication through `update_post`
        // would rewrite the whole row and clobber its child rows.
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let posts = &*env.state.posts;
        let seeded = SeedRawPost::new(user.user_id)
            .draft()
            .summary(parse_post_summary("the summary"))
            .tags(["Rust"])
            .seed(&env.state)
            .await;
        let before = posts
            .get_post_by_id(seeded.post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        let audiences_before = posts.get_post_audiences(seeded.post_id).await.unwrap();

        let after = posts
            .publish_post(seeded.post_id, user.user_id)
            .await
            .expect("publish succeeds");

        assert!(after.published_at.is_some(), "the draft is now published");
        assert_eq!(after.author_username, user.username);
        assert_eq!(after.title, before.title);
        assert_eq!(after.slug, before.slug);
        assert_eq!(after.body, before.body);
        assert_eq!(after.format, before.format);
        assert_eq!(after.rendered_html, before.rendered_html);
        assert_eq!(after.summary, before.summary);
        assert_eq!(after.created_at, before.created_at);
        assert_eq!(after.tags.len(), 1);
        assert_eq!(after.tags[0].tag_slug, "rust");
        assert_eq!(
            posts.get_post_audiences(seeded.post_id).await.unwrap(),
            audiences_before
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publish_post_keeps_an_already_published_timestamp(#[case] backend: Backend) {
        // COALESCE, not overwrite: the permalink is derived from `published_at`, so
        // re-publishing must not restamp it.
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;

        let first = posts.publish_post(post_id, user_id).await.unwrap();
        let second = posts.publish_post(post_id, user_id).await.unwrap();

        assert!(first.published_at.is_some());
        assert_eq!(
            first.published_at, second.published_at,
            "republishing must not restamp"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publish_post_rejects_a_missing_foreign_or_deleted_post(#[case] backend: Backend) {
        // The ownership/liveness guard `update_post` applies, applied to publication:
        // a post that is gone reads as NotFound, someone else's live post as
        // Unauthorized (both mask as a 404 at the web boundary).
        let env = backend.setup().await;
        let [owner, stranger] = seed_users::<2>(&env.state).await;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(owner)
            .draft()
            .seed(&env.state)
            .await
            .post_id;

        assert!(matches!(
            posts.publish_post(PostId::from(999_999), owner).await,
            Err(UpdatePostError::NotFound)
        ));
        assert!(matches!(
            posts.publish_post(post_id, stranger).await,
            Err(UpdatePostError::Unauthorized)
        ));
        // The rejected publish wrote nothing: the post is still a draft.
        assert!(
            posts
                .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
                .await
                .unwrap()
                .unwrap()
                .published_at
                .is_none()
        );

        posts.soft_delete_post(post_id).await.unwrap();
        assert!(matches!(
            posts.publish_post(post_id, owner).await,
            Err(UpdatePostError::NotFound)
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publish_post_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env
            .state
            .posts
            .publish_post(PostId::from(1), UserId::from(1))
            .await;
        assert!(matches!(result, Err(UpdatePostError::Internal(_))));
    }

    // -----------------------------------------------------------------------
    // post_media: what a post's rendered HTML points a reader at (#711)
    // -----------------------------------------------------------------------

    #[apply(backends)]
    #[tokio::test]
    async fn create_post_writes_its_media_rows(#[case] backend: Backend) {
        // A11, and the web half of A14: `create_post_via_service` is the entry point
        // `web::posts::create` uses, so this drives render -> extract -> write through
        // the product's own path rather than a synthetic input.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let uploaded = seed_media(&env.state, user, "photo.jpg").await;
        let body = format!("<img src=\"{}\">", media_url_for("photo.jpg"));

        let post_id = create_post_via_service(&env.state, user, parse_post_body(&body)).await;

        assert_eq!(
            fetch_post_media(&env.base, post_id).await,
            vec![media_ref_for("photo.jpg")]
        );
        // The recorded triple names the entry the `media` table holds — the join a
        // reference guard reads in the other direction.
        assert!(media_row_exists(&env.state, user, &uploaded).await);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_post_records_a_raw_filename_and_a_member_url(#[case] backend: Backend) {
        // A2, A3 at the persistence level — the issue's two headline spellings become
        // rows: a URL bearing the name a person types resolves to the stored encoded
        // spelling, and the AtomPub member layout (which carries no source segment)
        // is recognised too.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let raw = media_url_for("my photo.jpg").replace("%20", " ");
        let member = format!("/atompub/alice/media/{MEDIA_TEST_SHA256}/photo.jpg");
        let body = format!("<img src=\"{raw}\"><a href=\"{member}\">doc</a>");

        let post_id = create_post_via_service(&env.state, user, parse_post_body(&body)).await;

        let names: Vec<String> = fetch_post_media(&env.base, post_id)
            .await
            .into_iter()
            .map(|media| media.filename.to_string())
            .collect();
        assert!(
            names.contains(&"my%20photo.jpg".to_owned()),
            "raw spelling must be canonicalised: {names:?}"
        );
        assert!(
            names.contains(&"photo.jpg".to_owned()),
            "member URL must be recorded: {names:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn a_post_referencing_nothing_writes_no_media_rows(#[case] backend: Backend) {
        // A13 — no false positives: prose that names no file writes no rows.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;

        let post_id =
            create_post_via_service(&env.state, user, parse_post_body("just some prose")).await;

        assert!(fetch_post_media(&env.base, post_id).await.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn updating_a_post_replaces_its_media_rows(#[case] backend: Backend) {
        // A12, both directions — an edit that swaps one embed for another removes the
        // old row and adds the new one, and an edit that removes every embed empties
        // the set. This is why `replace_post_media` deletes before inserting.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let a = media_url_for("a.jpg");
        let b = media_url_for("b.jpg");
        let post_id = create_post_via_service(
            &env.state,
            user,
            parse_post_body(&format!("<img src=\"{a}\">")),
        )
        .await;

        update_post_body_via_service(
            &env.state,
            post_id,
            user,
            parse_post_body(&format!("<img src=\"{b}\">")),
        )
        .await;

        let rows = fetch_post_media(&env.base, post_id).await;
        assert_eq!(rows.len(), 1, "the removed reference is gone: {rows:?}");
        assert_eq!(
            rows[0],
            media_ref_for("b.jpg"),
            "the added reference is present"
        );

        update_post_body_via_service(
            &env.state,
            post_id,
            user,
            parse_post_body("no media at all"),
        )
        .await;

        assert!(fetch_post_media(&env.base, post_id).await.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publishing_a_draft_preserves_its_media_rows(#[case] backend: Backend) {
        // A15 and the row half of A15b — publication is not an edit, so it must leave
        // the child rows alone. This fails against any design that routes publication
        // through `update_post`, which would rewrite them from whatever input the
        // publish call happened to carry. It lives here rather than beside the other
        // `publish_post` tests because it needs `post_media` to exist.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let body = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
        let post_id = create_draft_via_service(&env.state, user, parse_post_body(&body)).await;
        let before = fetch_post_media(&env.base, post_id).await;
        assert_eq!(
            before.len(),
            1,
            "precondition: the draft records its reference"
        );

        env.state
            .posts
            .publish_post(post_id, user)
            .await
            .expect("publish succeeds");

        assert_eq!(
            fetch_post_media(&env.base, post_id).await,
            before,
            "rows survive publication"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_posts_referencing_media_scopes_and_orders(#[case] backend: Backend) {
        // A16.
        let env = backend.setup().await;
        let [owner, stranger] = seed_users::<2>(&env.state).await;
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));

        let first = create_post_via_service(&env.state, owner, parse_post_body(&embed)).await;
        let second = create_post_via_service(&env.state, owner, parse_post_body(&embed)).await;
        let deleted = create_post_via_service(&env.state, owner, parse_post_body(&embed)).await;
        let foreign = create_post_via_service(&env.state, stranger, parse_post_body(&embed)).await;
        let unrelated =
            create_post_via_service(&env.state, owner, parse_post_body("no media")).await;
        env.state
            .posts
            .soft_delete_post(deleted)
            .await
            .expect("soft delete succeeds");

        let found = env
            .state
            .posts
            .list_posts_referencing_media(owner, &media_ref_for("photo.jpg"))
            .await
            .expect("listing succeeds");

        assert_eq!(found, vec![first, second], "own, non-deleted, ascending");
        assert!(
            !found.contains(&deleted),
            "a soft-deleted post does not block a delete"
        );
        assert!(
            !found.contains(&foreign),
            "another user's post is not reported (spec D9)"
        );
        assert!(!found.contains(&unrelated));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_posts_referencing_media_reports_every_reference_past_the_old_scan_window(
        #[case] backend: Backend,
    ) {
        // A17 — the truncation half. The old code paged the user's posts at
        // `RowLimit::at_most(1000)` and scanned their bodies, so a reference in post
        // 1001 went unseen and its media stayed silently deletable.
        //
        // Every seeded post embeds the *same* media, which is stronger than seeding
        // 1200 unrelated fillers plus one needle: with only one matching row a `LIMIT
        // 1000` on the new join would pass unnoticed, since the filter runs first.
        // Here the cap has 1201 rows to truncate, so the absence of a limit is what
        // the assertions actually rest on.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let body = parse_post_body(&format!("<img src=\"{}\">", media_url_for("needle.jpg")));

        // One batched transaction, not 1201 round trips. `create_posts` shares
        // `write_post_in_tx` with `create_post`, so each row's `post_media` is written
        // too, and the ids come back in input order.
        let inputs: Vec<CreatePostInput> = (0..1201)
            .map(|_| SeedRawPost::new(user).body(body.clone()).build())
            .collect();
        let ids = env
            .state
            .posts
            .create_posts(&inputs)
            .await
            .expect("batch seed succeeds");

        let found = env
            .state
            .posts
            .list_posts_referencing_media(user, &media_ref_for("needle.jpg"))
            .await
            .expect("listing succeeds");

        assert!(
            found == ids,
            "every reference, ascending and untruncated: got {} of {}",
            found.len(),
            ids.len()
        );
        assert_eq!(
            found.last(),
            ids.last(),
            "the reference past the old 1000-row window is returned"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_posts_referencing_media_returns_empty_for_unreferenced_media(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        create_post_via_service(&env.state, user, parse_post_body("no media")).await;

        let found = env
            .state
            .posts
            .list_posts_referencing_media(user, &media_ref_for("absent.jpg"))
            .await
            .expect("listing succeeds");

        assert!(found.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn reading_post_with_overlong_summary_in_db_errors(#[case] backend: Backend) {
        // A pre-existing row whose summary exceeds MAX_POST_SUMMARY_CHARS (the
        // column is unbounded TEXT) must surface as an error at the strict read
        // boundary — never a panic — because the validating sqlx `Decode` fails
        // closed through `PostSummary`'s `FromStr`. The over-cap value is
        // unconstructible via the newtype, so it is forced in with raw SQL.
        // Mirrors `users.rs`'s overlong-display-name fail-closed test.
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;

        let overlong = "a".repeat(common::post_summary::MAX_POST_SUMMARY_CHARS + 1);
        let sql = format!(
            "UPDATE posts SET summary='{overlong}' WHERE post_id={}",
            i64::from(post_id)
        );
        env.base.pool().execute(sql.as_str()).await.unwrap();

        let result = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn reading_post_with_blank_title_in_db_errors(#[case] backend: Backend) {
        // The title column is nullable TEXT with no CHECK, so a blank row is
        // representable in the database even though `PostTitle` can no longer
        // construct one (#830). It must fail closed at the strict read boundary
        // through the validating `Decode`, never decode to an empty title. Forced in
        // with raw SQL for the same reason as the overlong-summary test above.
        //
        // This also pins that `PostTitle` is on the *validating* sqlx bridge at all:
        // a blank row can only fail here if `Decode` routes through `FromStr`. That
        // bridge decodes a borrowed `&'r str` without allocating (`macros`'
        // `validating_bridge_decodes_a_borrowed_str_without_allocating`, #758).
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;

        let sql = format!(
            "UPDATE posts SET title='' WHERE post_id={}",
            i64::from(post_id)
        );
        env.base.pool().execute(sql.as_str()).await.unwrap();

        let result = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn reading_post_with_blank_body_in_db_errors(#[case] backend: Backend) {
        // ADR-0105 §1 claims a blank body is "unrepresentable" partly because a blank
        // one already in the database fails to *decode*. The serde half of that claim
        // is pinned by `post_body_deserialize_rejects_blank`; this is the sqlx half.
        // The body column is plain TEXT with no CHECK, so the row is representable at
        // the database level — it is forced past `PostBody` with raw SQL, exactly as
        // the blank-title test above does. Whitespace (not "") is used deliberately:
        // it pins the newtype's *blank* rule rather than a mere emptiness check.
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;

        let sql = format!(
            "UPDATE posts SET body='   ' WHERE post_id={}",
            i64::from(post_id)
        );
        env.base.pool().execute(sql.as_str()).await.unwrap();

        let result = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_post_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = SeedRawPost::new(UserId::from(1))
            .draft()
            .create(&env.state)
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_post_by_id_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env
            .state
            .posts
            .get_post_by_id(PostId::from(1), &ViewerIdentity::Anonymous)
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_published_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env
            .state
            .posts
            .list_published(
                None,
                parse_row_limit("10"),
                &ViewerIdentity::Anonymous,
                Utc::now(),
            )
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_insert_error_returns_internal(#[case] backend: Backend) {
        let env = backend.setup().await;
        let uid = SeedUser::new().seed(&env.state).await.user_id;
        let post_id = SeedRawPost::new(uid).draft().seed(&env.state).await.post_id;

        // Break the post_tags statements inside the transaction (but not the
        // post-existence check, which reads `posts`) so they return a plain
        // Database error: exercises the catch-all Internal arm and the
        // BEGIN IMMEDIATE / FOR UPDATE rollback path on an unexpected failure.
        env.base
            .pool()
            .execute("ALTER TABLE post_tags RENAME COLUMN tag_display TO tag_display_x")
            .await
            .unwrap();

        let result = env
            .state
            .posts
            .set_post_tags(post_id, &[parse_tag_label("rust")])
            .await;
        assert!(matches!(result, Err(TaggingError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_collection_by_user_orders_by_updated_at_desc_and_excludes_deleted(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let uid = SeedUser::new().seed(&env.state).await.user_id;
        let now = Utc::now();

        let mk = |slug: &str, published: bool| {
            let builder = SeedRawPost::new(uid).slug(slug);
            if published {
                builder.published_at(now - chrono::Duration::minutes(30))
            } else {
                builder.draft()
            }
        };

        // Post 1: draft. Post 2: published. Post 3: soft-deleted (excluded).
        let post1_id = mk("draft-post", false).seed(&env.state).await.post_id;
        let post2_id = mk("published-post", true).seed(&env.state).await.post_id;
        let post3_id = mk("deleted-post", true).seed(&env.state).await.post_id;

        // Give distinct updated_at (post2 more recent than post1) and soft-delete post3.
        // ISO-8601 literals inlined so both backends accept the raw statement.
        let t_older = (now - chrono::Duration::hours(2)).to_rfc3339();
        let t_newer = (now - chrono::Duration::hours(1)).to_rfc3339();
        let now_str = now.to_rfc3339();
        env.base
            .pool()
            .execute(&format!(
                "UPDATE posts SET updated_at = '{t_older}' WHERE post_id = {post1_id}"
            ))
            .await
            .unwrap();
        env.base
            .pool()
            .execute(&format!(
                "UPDATE posts SET updated_at = '{t_newer}' WHERE post_id = {post2_id}"
            ))
            .await
            .unwrap();
        env.base
            .pool()
            .execute(&format!(
                "UPDATE posts SET deleted_at = '{now_str}' WHERE post_id = {post3_id}"
            ))
            .await
            .unwrap();

        let results = env
            .state
            .posts
            .list_collection_by_user(uid, None, parse_row_limit("10"))
            .await
            .unwrap();

        // The collection is the author's view: drafts and published in, deleted out,
        // ordered by updated_at DESC (post2 updated more recently).
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].post_id, post2_id);
        assert_eq!(results[1].post_id, post1_id);
        assert!(
            results
                .iter()
                .any(|p| p.post_id == post1_id && p.published_at.is_none())
        );
        assert!(
            results
                .iter()
                .any(|p| p.post_id == post2_id && p.published_at.is_some())
        );
        assert!(!results.iter().any(|p| p.post_id == post3_id));
    }

    // Not-found/unauthorized mask as a 404; internal is a masked storage failure.
    #[test]
    fn from_update_post_error_maps_variants() {
        use host::error::{ErrorKind, InternalError};

        let not_found: InternalError = UpdatePostError::NotFound.into();
        assert_eq!(not_found.kind(), ErrorKind::NotFound);
        assert_eq!(not_found.public_message(), "Post not found");

        let unauthorized: InternalError = UpdatePostError::Unauthorized.into();
        assert_eq!(unauthorized.kind(), ErrorKind::NotFound);
        assert_eq!(unauthorized.public_message(), "Post not found");

        let internal: InternalError = UpdatePostError::Internal(sqlx::Error::PoolClosed).into();
        assert_eq!(internal.kind(), ErrorKind::Storage);
        assert_eq!(internal.public_message(), "storage operation failed");
    }

    // The `set_post_tags` lift masks as a server error
    // (`"server operation failed"`, kind `Internal`) while the typed
    // `TaggingError` is preserved on the operator side rather than stringified.
    #[test]
    fn from_tagging_error_maps_to_server() {
        use host::error::{ErrorKind, InternalError};

        let error: InternalError = TaggingError::PostNotFound.into();
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(error.public_message(), "server operation failed");
        // The typed source is preserved (not flattened to the wire message).
        assert!(error.operator_message().contains("post not found"));
    }

    // -- Cursor + effectful helper tests (Cluster C push-down, #334) --

    #[test]
    fn to_post_cursor_round_trips_through_parse() {
        use chrono::TimeZone;
        let post = PostRecord {
            post_id: PostId::from(42),
            user_id: UserId::from(1),
            author_username: parse_username("author"),
            title: None,
            slug: parse_slug("hello-world"),
            body: parse_post_body("hello world"),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>hello world</p>"),
            created_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            published_at: None,
            deleted_at: None,
            summary: None,
            tags: vec![],
        };

        let cursor = to_post_cursor(&post);
        let parsed = parse_post_cursor(Some(cursor.created_at), Some(cursor.post_id))
            .unwrap()
            .expect("both components present yields a cursor");
        assert_eq!(parsed.created_at, post.created_at);
        assert_eq!(parsed.post_id, post.post_id);
    }

    #[test]
    fn parse_post_cursor_accepts_empty_cursor() {
        assert!(parse_post_cursor(None, None).unwrap().is_none());
    }

    #[test]
    fn parse_post_cursor_rejects_half_a_cursor() {
        use chrono::TimeZone;
        assert!(
            parse_post_cursor(
                Some(Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap()),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn list_by_tag_rows_maps_each_arm() {
        assert!(list_by_tag_rows(Ok(vec![])).is_ok());

        let tag_not_found = list_by_tag_rows(Err(ListByTagError::TagNotFound));
        assert!(matches!(tag_not_found, Ok(rows) if rows.is_empty()));

        let internal = list_by_tag_rows(Err(ListByTagError::Internal(sqlx::Error::PoolClosed)));
        assert!(internal.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn fetch_post_record_returns_seeded_post_and_none_for_missing(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let ids = crate::test_support::seed_posts(&env.state, user_id, 1, true).await;
        let record = posts
            .get_post_by_id(ids[0], &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        let date = PermalinkDate::from(record.created_at.date_naive());

        // A published, public post is visible to an anonymous viewer at its permalink.
        let found = fetch_post_record(
            posts,
            &ViewerIdentity::Anonymous,
            &record.author_username,
            date,
            &record.slug,
            Utc::now(),
        )
        .await
        .unwrap();
        assert_eq!(found.map(|p| p.post_id), Some(record.post_id));

        // A permalink with no matching post resolves to None (not an error).
        let missing = fetch_post_record(
            posts,
            &ViewerIdentity::Anonymous,
            &record.author_username,
            date,
            &parse_slug("no-such-slug"),
            Utc::now(),
        )
        .await
        .unwrap();
        assert!(missing.is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_round_trips_slug_and_label(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;

        // Tagging with a case-preserving label stores the canonical slug and the
        // author's casing; both read back intact on either backend.
        posts
            .set_post_tags(post_id, &[parse_tag_label("Rust")])
            .await
            .unwrap();

        let tags = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .expect("get_post_by_id failed")
            .expect("post exists")
            .tags;
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].tag_slug, "rust"); // canonical slug (lowercased)
        assert_eq!(tags[0].tag_display, "Rust"); // author casing preserved
    }

    #[apply(backends)]
    #[tokio::test]
    async fn post_round_trips_slug_title_body_username_and_tag(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let user_id = user.user_id;
        let posts = &*env.state.posts;

        // `create_post` binds a typed `Slug`, `Option<&PostTitle>`, and `&PostBody`;
        // `set_post_tags` binds a `TagLabel`. The read decodes the `slug`/`title`/`body`/
        // author-`username` columns and the JSON `tag_slug`/`tag_display` straight
        // back into their newtypes — exercising both bridge directions (#438).
        let body = parse_post_body("the round-trip body");
        let post = SeedRawPost::new(user_id)
            .draft()
            .body(body.clone())
            .seed(&env.state)
            .await;
        let post_id = post.post_id;
        posts
            .set_post_tags(post_id, &[parse_tag_label("Rust")])
            .await
            .unwrap();

        let record = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.slug, post.slug);
        assert_eq!(record.title, Some(post.title));
        assert_eq!(record.body, body);
        assert_eq!(record.author_username, user.username);
        assert_eq!(record.tags.len(), 1);
        assert_eq!(record.tags[0].tag_slug, "rust");
        assert_eq!(record.tags[0].tag_display, "Rust");

        // A post with no title exercises the `None` decode path for
        // `Option<PostTitle>`.
        let untitled_body = parse_post_body("body");
        let untitled_id = posts
            .create_post(&CreatePostInput {
                user_id,
                title: None,
                slug: parse_slug("no-title"),
                body: untitled_body.clone(),
                format: PostFormat::Markdown,
                rendered: RenderOutput::render(&untitled_body, &PostFormat::Markdown),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .unwrap();
        let untitled = posts
            .get_post_by_id(untitled_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(untitled.title, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_post_rejects_malformed_aggregated_tags_as_decode_error(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;
        posts
            .set_post_tags(post_id, &[parse_tag_label("Rust")])
            .await
            .expect("seed tag");

        // The tags aggregate reads `tag_slug` from `tags`. Land a value the `Tag`
        // serde bridge rejects with a raw bind, because a typed bind cannot create it.
        let sql = "UPDATE tags SET tag_slug = $1
                   WHERE tag_id = (SELECT tag_id FROM post_tags WHERE post_id = $2)";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind("not a slug")
                .bind(post_id)
                .execute(pool)
                .await
                .expect("corrupt tag slug");
        });

        // Exercise the public read boundary: malformed aggregate state must become
        // the decode error from PostRecord's JSON aggregate decoder.
        let err = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .expect_err("malformed aggregated tag must fail the read");
        assert!(
            matches!(err, sqlx::Error::Decode(_)),
            "expected an aggregate decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_post_rejects_a_malformed_slug_column(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;

        // Overwrite the `slug` column with a value `Slug::from_str` rejects (a space
        // is not a valid slug character), binding it as a raw `&str` so the bad
        // value actually lands in the column — the typed bind could not produce it.
        let sql = "UPDATE posts SET slug = $1 WHERE post_id = $2";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind("not a slug")
                .bind(post_id)
                .execute(pool)
                .await
                .unwrap();
        });

        // The read decodes the `slug` column into `Slug` via the sqlx bridge, which
        // validates through `FromStr`; the malformed value surfaces as a
        // column-decode error rather than being silently admitted (covers the
        // bridge's `Decode` error arm).
        let err = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn post_format_column_round_trips_all_variants(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;

        // Org and Html exercise the `PostFormat` bridge Encode (write) + Decode (read)
        // for the non-default variants; Markdown is covered by the round-trip tests.
        for fmt in [PostFormat::Org, PostFormat::Html] {
            let post_id = SeedRawPost::new(user_id)
                .draft()
                .format(fmt)
                .seed(&env.state)
                .await
                .post_id;
            let record = posts
                .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(record.format, fmt);
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_post_rejects_a_malformed_format_column(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;

        // Land a bogus token in `format` via a raw bind (the typed bind could not
        // produce it), then assert the read fails at column-decode — the bridge's
        // `Decode` error arm (`parse()` → `InvalidPostFormat`).
        let sql = "UPDATE posts SET format = $1 WHERE post_id = $2";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind("bogus")
                .bind(post_id)
                .execute(pool)
                .await
                .unwrap();
        });
        let err = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_unpublished_post_by_permalink_matches_canonical_date_and_scope(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let posts = &*env.state.posts;
        let now = Utc::now();
        let scheduled_at = now + chrono::Duration::days(30);
        let author = SeedUser::new().seed(&env.state).await.user_id;
        let other = SeedUser::new().seed(&env.state).await.user_id;

        let draft = SeedRawPost::new(author).draft().seed(&env.state).await;
        let scheduled = SeedRawPost::new(author)
            .published_at(scheduled_at)
            .seed(&env.state)
            .await;
        let live_at_boundary = SeedRawPost::new(author)
            .published_at(now)
            .seed(&env.state)
            .await;
        let deleted = SeedRawPost::new(author)
            .published_at(scheduled_at)
            .seed(&env.state)
            .await;
        posts
            .soft_delete_post(deleted.post_id)
            .await
            .expect("soft delete");

        let draft_record = posts
            .get_post_by_id(draft.post_id, &ViewerIdentity::Local { user_id: author })
            .await
            .unwrap()
            .expect("author can read seeded draft");
        let draft_date = PermalinkDate::from(draft_record.created_at.date_naive());
        let scheduled_date = PermalinkDate::from(scheduled_at.date_naive());

        let found_draft = posts
            .get_unpublished_post_by_permalink(author, draft_date, &draft.slug, now)
            .await
            .unwrap();
        assert_eq!(found_draft.map(|post| post.post_id), Some(draft.post_id));

        let found_scheduled = posts
            .get_unpublished_post_by_permalink(author, scheduled_date, &scheduled.slug, now)
            .await
            .unwrap();
        assert_eq!(
            found_scheduled.map(|post| post.post_id),
            Some(scheduled.post_id)
        );

        let missing = parse_slug("missing");
        for (user_id, date, slug) in [
            (other, scheduled_date, &scheduled.slug),
            (author, scheduled_date, &missing),
            (
                author,
                PermalinkDate::from(now.date_naive()),
                &live_at_boundary.slug,
            ),
            (author, scheduled_date, &deleted.slug),
        ] {
            assert!(
                posts
                    .get_unpublished_post_by_permalink(user_id, date, slug, now)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }
}
