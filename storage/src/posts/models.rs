//! Public post records and persistence bridge values.

use sqlx::{Decode, Result, Row, Type};

use crate::helpers::SerializedPostTags;
use crate::posts::cursors::PostRevisionCursor;
use crate::posts::store::PostTag;
use common::etag::ETag;
use common::idempotency_key::IdempotencyKey;
use common::ids::{PostId, RevisionId, UserId};
use common::media::MediaReference;
use common::org::PublicationState;
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
pub use common::render::{InvalidPostFormat, PostFormat, RenderedHtml};
use common::root_relative_url::RootRelativeUrl;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::AudienceTarget;
use host::render::RenderOutput;

/// The `published_at`-clear flag in an update-post statement.
///
/// This is deliberately a persistence role: [`PublishUpdate`] remains the
/// application-level publication instruction, while the SQL `CASE` needs only
/// its clear-column fact.
#[derive(Clone, Copy, Debug, macros::SqlxBridge)]
pub(crate) struct PostPublicationClear(bool);

impl PostPublicationClear {
    #[must_use]
    pub(crate) const fn for_update(update: PublishUpdate) -> Self {
        Self(matches!(update, PublishUpdate::Unpublish))
    }
}

/// ISO calendar text used by the permalink-date SQL comparison.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct PermalinkDateText(String);

impl From<PermalinkDate> for PermalinkDateText {
    fn from(date: PermalinkDate) -> Self {
        Self(date.to_string())
    }
}

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
    pub rendered_html: RenderedHtml,
    /// When the post was first created.
    pub created_at: UtcInstant,
    /// When the post was last updated.
    pub updated_at: UtcInstant,
    /// When the post was published (None if it is a draft).
    pub published_at: Option<UtcInstant>,
    /// When the post was soft-deleted (None if active).
    pub deleted_at: Option<UtcInstant>,
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
        let timestamp = self.published_at.unwrap_or(self.created_at).value();
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
    /// This label is disposable presentation metadata for an unpublished row, not authored Post
    /// content or historical state; it is derived from the canonical [`PostBody`]. Recomputing it
    /// at read time is deliberate: the bounded draft query already loads the body, while storing
    /// it would need freshness maintenance across body writes and direct backup restores.
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
    PostId: Decode<'r, R::Database> + Type<R::Database>,
    UserId: Decode<'r, R::Database> + Type<R::Database>,
    Username: Decode<'r, R::Database> + Type<R::Database>,
    PostTitle: Decode<'r, R::Database> + Type<R::Database>,
    Slug: Decode<'r, R::Database> + Type<R::Database>,
    PostBody: Decode<'r, R::Database> + Type<R::Database>,
    PostFormat: Decode<'r, R::Database> + Type<R::Database>,
    RenderedHtml: Decode<'r, R::Database> + Type<R::Database>,
    UtcInstant: Decode<'r, R::Database> + Type<R::Database>,
    PostSummary: Decode<'r, R::Database> + Type<R::Database>,
    SerializedPostTags: Decode<'r, R::Database> + Type<R::Database>,
{
    fn from_row(row: &'r R) -> Result<Self> {
        let post_id = row.try_get::<PostId, _>("post_id")?;
        let user_id = row.try_get::<UserId, _>("user_id")?;
        let author_username = row.try_get::<Username, _>("username")?;
        let title = row.try_get::<Option<PostTitle>, _>("title")?;
        let slug = row.try_get::<Slug, _>("slug")?;
        let body = row.try_get::<PostBody, _>("body")?;
        let format = row.try_get::<PostFormat, _>("format")?;
        let rendered_html = row.try_get::<RenderedHtml, _>("rendered_html")?;
        let created_at = row.try_get::<UtcInstant, _>("created_at")?;
        let updated_at = row.try_get::<UtcInstant, _>("updated_at")?;
        let published_at = row.try_get::<Option<UtcInstant>, _>("published_at")?;
        let deleted_at = row.try_get::<Option<UtcInstant>, _>("deleted_at")?;
        let summary = row.try_get::<Option<PostSummary>, _>("summary")?;
        let tags_json = row.try_get::<SerializedPostTags, _>("tags")?;
        let tags = tags_json.into_tags(post_id);

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

/// An immutable complete prior-state snapshot of a Post.
///
/// This read model intentionally has no mutators: product storage creates it as
/// part of a top-level Post mutation, while backup/restore is the only other
/// legitimate whole-store writer (ADR-0136).
#[derive(Clone, Debug)]
pub struct PostRevisionRecord {
    /// Unique identifier for this snapshot.
    pub revision_id: RevisionId,
    /// Durable identity of the Post whose prior state was captured.
    pub post_id: PostId,
    /// Owner copied from the Post at capture time.
    pub user_id: UserId,
    /// Authored title at capture time.
    pub title: Option<PostTitle>,
    /// Authored permalink slug at capture time.
    pub slug: Slug,
    /// Authored source at capture time.
    pub body: PostBody,
    /// Interpretation of the authored source at capture time.
    pub format: PostFormat,
    /// Sanitized rendered representation produced from the captured source.
    pub rendered_html: RenderedHtml,
    /// Optional authored summary at capture time.
    pub summary: Option<PostSummary>,
    /// Original Post creation time, not the capture time.
    pub created_at: UtcInstant,
    /// Prior Post modification time.
    pub updated_at: UtcInstant,
    /// Prior publication time, if the captured state was published or scheduled.
    pub published_at: Option<UtcInstant>,
    /// Prior deletion tombstone time, if the captured state was Deleted.
    pub deleted_at: Option<UtcInstant>,
    /// Time this immutable snapshot was captured.
    pub captured_at: UtcInstant,
    /// Normalized tag state at capture time.
    pub tags: Vec<PostRevisionTag>,
    /// Audience state at capture time.
    pub audiences: Vec<AudienceTarget>,
    /// Exact rendered-media references at capture time.
    pub media: Vec<MediaReference>,
}

/// One normalized tag value belonging to an immutable Post Revision.
#[derive(Clone, Debug)]
pub struct PostRevisionTag {
    /// Normalized slug copied at capture time rather than linked to mutable tags.
    pub tag: Tag,
    /// Display spelling captured with the revision.
    pub display: TagLabel,
}

/// Lifecycle derived from a Post state at the supplied clock, never persisted as
/// a separate mutable flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostLifecycle {
    Draft,
    Scheduled,
    Published,
    Deleted,
}

/// One immutable revision row in a history list.
#[derive(Clone, Debug)]
pub struct PostRevisionMetadata {
    pub revision_id: RevisionId,
    pub post_id: PostId,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub captured_at: UtcInstant,
    /// Lifecycle derived against `captured_at`, so it remains stable.
    pub snapshot_lifecycle: PostLifecycle,
    /// Whether the current durable Post is now Deleted.
    pub current_deleted: bool,
}

/// The non-revision current state heading a per-Post history page.
#[derive(Clone, Debug)]
pub struct CurrentPostRevisionSummary {
    pub post_id: PostId,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub format: PostFormat,
    pub created_at: UtcInstant,
    pub updated_at: UtcInstant,
    pub published_at: Option<UtcInstant>,
    pub deleted_at: Option<UtcInstant>,
    /// Lifecycle derived at request time.
    pub lifecycle: PostLifecycle,
}

/// One owner-only history page.
#[derive(Clone, Debug)]
pub struct PostRevisionPage {
    pub revisions: Vec<PostRevisionMetadata>,
    pub next_cursor: Option<PostRevisionCursor>,
}

/// An owner-only revision detail with its current Post context where applicable.
#[derive(Clone, Debug)]
pub struct PostRevisionDetail {
    pub revision: PostRevisionRecord,
}

/// Non-authoritative metadata an Org ingress expects the stored post to match.
///
/// Storage evaluates this inside the write transaction: create compares after its
/// successful unique-index insert, while update compares the locked pre-write row
/// and its tag projection before creating a revision.
#[derive(Clone, Debug, Default)]
pub struct PostBookkeepingExpectation {
    /// Final collision-resolved slug.
    pub slug: Option<Slug>,
    /// Final stored markup format.
    pub format: Option<PostFormat>,
    /// Final stored publication instant; `Some(None)` expects a draft.
    pub published_at: Option<Option<UtcInstant>>,
    /// Target identity for an update.
    pub post_id: Option<PostId>,
    /// Current pre-write content validator for an update.
    pub content_etag: Option<ETag>,
}

/// Converts normalized Org bookkeeping into the persistence checks that must run
/// inside the post write transaction.
impl From<common::org::OrgBookkeeping> for PostBookkeepingExpectation {
    fn from(bookkeeping: common::org::OrgBookkeeping) -> Self {
        Self {
            slug: bookkeeping.slug,
            format: bookkeeping.format,
            published_at: bookkeeping.date_utc.map(Some),
            post_id: bookkeeping.post_id,
            content_etag: bookkeeping.synced,
        }
    }
}

/// The result of a successful post creation before its enclosing transaction
/// has committed.
pub struct CreatedPost {
    pub record: PostRecord,
    /// Whether the transaction retired an expired mapping before replacing it.
    pub idempotency_key_expired: bool,
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
    pub published_at: Option<UtcInstant>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
    /// Audience targeting for the post. Each entry becomes a `post_audiences`
    /// row; `Private` and an empty vec produce no rows (the post is private).
    pub audiences: Vec<AudienceTarget>,
    /// Tags attached atomically with the new post. Creation has no prior state,
    /// so this never creates a revision.
    pub tags: Vec<TagLabel>,
    /// Non-authoritative Org bookkeeping to compare after the successful row insert.
    pub expectations: PostBookkeepingExpectation,
    /// If `Some`, atomically replay its live `(user_id, key)` mapping or
    /// register the key against the new post. A replay returns
    /// [`CreatePostError::IdempotencyConflict`] carrying the selected Post.
    pub idempotency_key: Option<IdempotencyKey>,
}

/// What an update does to a Post's publication state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishUpdate {
    /// Clear `published_at` back to NULL (draft / unschedule).
    Unpublish,
    /// Publish. `at = Some(t)` sets `published_at = t` (future = scheduled,
    /// past = backdated-live). `at = None` keeps an existing timestamp or
    /// stamps `now` for a previously-unpublished Post.
    Publish { at: Option<UtcInstant> },
}

impl From<PublicationState> for PublishUpdate {
    fn from(state: PublicationState) -> Self {
        match state {
            PublicationState::Draft => Self::Unpublish,
            PublicationState::Scheduled(at) | PublicationState::Published(at) => {
                Self::Publish { at: Some(at) }
            }
        }
    }
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
    /// What this update does to the Post's publication state.
    pub publish: PublishUpdate,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
    /// Audience targeting for the post. On update the existing
    /// `post_audiences` rows are replaced to match this vec; `Private` and an
    /// empty vec produce no rows (the post is private).
    pub audiences: Vec<AudienceTarget>,
    /// Tags replacing the current set inside this content mutation transaction.
    pub tags: Vec<TagLabel>,
    /// The single request clock used when publishing a previously-draft post now.
    pub request_clock: UtcInstant,
    /// Non-authoritative Org bookkeeping to compare under the owner lock.
    pub expectations: PostBookkeepingExpectation,
}
