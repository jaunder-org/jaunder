//! Generic storage implementation for posts.

use std::collections::BTreeSet;

use async_trait::async_trait;
use chrono::Duration;
use sha2::{Digest, Sha256};
use sqlx::{Database, Decode, Encode, Executor, Pool, Result, Row, Type};

use crate::InstanceId;
use crate::backend::Backend;
use crate::posts::cursors::{
    CollectionCursor, PostCursor, PostRevisionCursor, ScheduledPostCursor,
};
use crate::posts::errors::{CreatePostError, ListByTagError, TaggingError, UpdatePostError};
use crate::posts::media;
use crate::posts::media::{
    MediaReferenceEvidence, MediaReferenceSnapshot, PersistedMediaReference, PersistedMediaSubject,
    PostMediaReferenceBackfill,
};
use crate::posts::models::{
    CreatePostInput, CreatedPost, CurrentPostRevisionSummary, PermalinkDate, PermalinkDateText,
    PostLifecycle, PostRecord, PostRevisionDetail, PostRevisionMetadata, PostRevisionPage,
    PostRevisionRecord, PostRevisionTag, PublishUpdate, RenderedHtml, UpdatePostInput,
};
use crate::posts::tags;
use crate::posts::tags::{PostTag, TagRecord};
use crate::sql::{Exists, QueryStorageExt, RowCount};
use crate::write_scope::WriteTransaction;
use common::idempotency_key::IdempotencyKey;
use common::ids::{AudienceId, ChannelId, PostId, RevisionId, UserId};
use common::media::{
    ContentHash, Filename, MediaRef, MediaReferenceForm, MediaReferenceKind, MediaSource,
};
use common::pagination::{PageSize, RowLimit};
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::render::PostFormat;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::{self, AudienceTarget, SubscriberRef, TargetKind, ViewerIdentity};
use host::{
    error::{InternalError, InternalResult},
    etag,
    feed::{FeedMinItems, FeedPath},
    metrics,
    retention::Domain,
};

/// A post that crossed into "live" within a time window, carrying exactly the
/// data the feed worker needs to compute its affected feed URLs (the author's
/// username and the post's tag slugs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoLivePost {
    pub username: Username,
    pub tag_slugs: Vec<Tag>,
}

const IDEMPOTENCY_REPLAY_WINDOW_HOURS: i64 = 1;

fn idempotency_replay_cutoff(now: UtcInstant) -> UtcInstant {
    UtcInstant::from(now.value() - Duration::hours(IDEMPOTENCY_REPLAY_WINDOW_HOURS))
}

/// Captures every scalar field belonging to a complete immutable prior Post
/// state. Child snapshot writes intentionally remain with Task 2's transaction.
/// Bind order: `captured_at, post_id`.
pub(crate) const INSERT_COMPLETE_POST_REVISION: &str = "INSERT INTO post_revisions
     (post_id, user_id, title, slug, body, format, rendered_html, summary,
      created_at, updated_at, published_at, deleted_at, captured_at)
     SELECT post_id, user_id, title, slug, body, format, rendered_html, summary,
            created_at, updated_at, published_at, deleted_at, $1
     FROM posts WHERE post_id = $2
     RETURNING revision_id";

/// The locked pre-write columns needed for final-state and content expectations.
#[derive(sqlx::FromRow)]
pub(crate) struct PostBookkeepingRow {
    pub user_id: UserId,
    pub deleted_at: Option<UtcInstant>,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub body: PostBody,
    pub format: PostFormat,
    pub rendered_html: RenderedHtml,
    pub summary: Option<PostSummary>,
    pub published_at: Option<UtcInstant>,
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
    now: UtcInstant,
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

#[cfg_attr(any(test, feature = "test-utils"), mockall::automock)]
#[async_trait]
pub trait PostStorage: Send + Sync {
    /// Creates a new post at `now`.
    ///
    /// A keyed create uses `now` both to retire an expired mapping in this
    /// transaction and to establish the replacement mapping's replay window.
    /// The returned expiry observation is not telemetry until the caller's
    /// transaction has been confirmed committed.
    async fn create_post(
        &self,
        transaction: &mut WriteTransaction,
        input: &CreatePostInput,
        now: UtcInstant,
    ) -> Result<CreatedPost, CreatePostError>;

    /// Creates `inputs.len()` posts in an existing write transaction, returning their new
    /// ids in input order. All-or-nothing: any failure (e.g. a slug conflict on
    /// one row) rolls the whole batch back and nothing persists. An empty slice
    /// is a no-op.
    async fn create_posts(
        &self,
        transaction: &mut WriteTransaction,
        inputs: &[CreatePostInput],
    ) -> Result<Vec<PostId>, CreatePostError>;

    /// Returns the unexpired `post_id` a `(user_id, key)` idempotency pair maps
    /// to. A mapping created one hour or more before `now` never replays, even
    /// if physical cleanup has not run.
    async fn post_id_for_idempotency_key(
        &self,
        user_id: UserId,
        key: &IdempotencyKey,
        now: UtcInstant,
    ) -> Result<Option<PostId>, sqlx::Error>;

    /// Physically removes every idempotency mapping expired at `now`, in
    /// fixed-size statements. Each completed statement releases its connection
    /// before the next one, so an accumulated backlog never extends one lock.
    async fn prune_expired_idempotency_keys(&self, now: UtcInstant) -> Result<u64, sqlx::Error>;

    /// Fetches a post by its ID, applying the viewer-resolution filter: the post
    /// is returned only if `viewer` is the author or a targeted audience admits
    /// them. See ADR-0020.
    async fn get_post_by_id(
        &self,
        post_id: PostId,
        viewer: &ViewerIdentity,
    ) -> Result<Option<PostRecord>>;

    /// Lists immutable owner history across every owned Post, newest revision ID
    /// first. The owner bind is part of the storage query so this cannot become a
    /// web-only authorization check.
    async fn list_owned_revision_history(
        &self,
        user_id: UserId,
        cursor: Option<PostRevisionCursor>,
        page_size: PageSize,
    ) -> Result<PostRevisionPage>;

    /// Lists immutable owner history for one Post, including a Deleted Post.
    /// A missing or foreign Post returns `None`; callers deliberately map that
    /// absence to the same public error as a missing revision.
    async fn list_post_revision_history(
        &self,
        user_id: UserId,
        post_id: PostId,
        cursor: Option<PostRevisionCursor>,
        page_size: PageSize,
    ) -> Result<Option<PostRevisionPage>>;

    /// Returns the current owner-visible history heading, including a Deleted
    /// Post. Lifecycle is derived against the supplied request clock.
    async fn get_current_revision_summary(
        &self,
        user_id: UserId,
        post_id: PostId,
        now: UtcInstant,
    ) -> Result<Option<CurrentPostRevisionSummary>>;

    /// Returns one complete immutable snapshot only when both the Post and
    /// revision belong to `user_id`. The exact triple is bound in SQL.
    async fn get_post_revision_detail(
        &self,
        user_id: UserId,
        post_id: PostId,
        revision_id: RevisionId,
    ) -> Result<Option<PostRevisionDetail>>;

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
        now: UtcInstant,
    ) -> Result<Option<PostRecord>>;

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
        now: UtcInstant,
    ) -> Result<Option<PostRecord>>;

    /// Updates a post and creates a new revision.
    ///
    /// # Errors
    ///
    /// Returns [`UpdatePostError::NotFound`] if the post doesn't exist, or
    /// [`UpdatePostError::Unauthorized`] if the editor isn't the owner.
    async fn update_post(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Publishes a draft through the owner-checked revision transaction. A
    /// post that is already published is returned unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`UpdatePostError::NotFound`] if the post does not exist or is
    /// soft-deleted, or [`UpdatePostError::Unauthorized`] if `user_id` does not
    /// own it.
    async fn publish_post(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Marks an owned live post as deleted through the revision transaction.
    ///
    /// # Errors
    ///
    /// Returns [`UpdatePostError::NotFound`] if the post does not exist or is
    /// already soft-deleted, or [`UpdatePostError::Unauthorized`] if `user_id`
    /// does not own it.
    async fn soft_delete_post(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<(), UpdatePostError>;

    /// Reverts a live post owned by `user_id` to draft status, returning the row
    /// written by the guarded mutation.
    ///
    /// # Errors
    ///
    /// Returns [`UpdatePostError::NotFound`] if the post does not exist or is
    /// soft-deleted, or [`UpdatePostError::Unauthorized`] if `user_id` does not
    /// own its live row.
    async fn unpublish_post(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<PostRecord, UpdatePostError>;

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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>>;

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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>>;

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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>>;

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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>>;

    /// Lists all of a user's non-soft-deleted posts (drafts + published)
    /// ordered by `updated_at DESC, post_id DESC` for the `AtomPub` Collection
    /// surface. Tags are hydrated.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_collection_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a CollectionCursor>,
        limit: RowLimit,
    ) -> Result<Vec<PostRecord>>;

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
    /// [`TaggingError::PostNotFound`] if the post does not exist or is
    /// soft-deleted, and [`TaggingError::Unauthorized`] if `user_id` does not
    /// own it.
    async fn set_post_tags(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
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
        now: UtcInstant,
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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>, ListByTagError>;

    /// Returns tag records whose slug begins with `prefix` (case-insensitive
    /// on the slug). An empty / `None` prefix returns all tags, alphabetically,
    /// up to `limit`.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_tags<'a>(
        &self,
        prefix: Option<&'a str>,
        limit: RowLimit,
    ) -> Result<Vec<TagRecord>>;

    /// Lists published posts matching `surface`, applying the
    /// [`HybridWindow`](host::feed::HybridWindow) selection rule (union of
    /// "the most recent `min_items` items" and "all items published within the
    /// last `min_days`"). Results are ordered by `published_at DESC`.
    ///
    /// `now` is passed in so callers can supply a deterministic clock in
    /// tests. Posts with `published_at > now` (future-dated) are excluded.
    async fn list_published_in_window(
        &self,
        surface: &common::feed::FeedSurface,
        window: &host::feed::HybridWindow,
        now: UtcInstant,
        viewer: &ViewerIdentity,
    ) -> Result<Vec<PostRecord>>;

    /// Lists posts that crossed into "live" within the window `(after, upto]`
    /// (exclusive lower, inclusive upper): `published_at > after AND
    /// published_at <= upto AND deleted_at IS NULL`. Each [`GoLivePost`] carries
    /// its author username and tag slugs so the feed worker can fan out to the
    /// affected feed surfaces. Drives the steady-state go-live pass.
    async fn list_posts_gone_live_between(
        &self,
        after: UtcInstant,
        upto: UtcInstant,
    ) -> Result<Vec<GoLivePost>>;

    /// Returns the URLs of cached feeds whose surface has a live post
    /// (`published_at <= now`, not deleted) strictly newer than the feed's own
    /// `generated_at` — i.e. cached feeds that missed a go-live while the worker
    /// was down. Drives the feed-relative startup catch-up.
    async fn feed_urls_needing_catchup(&self, now: UtcInstant) -> Result<Vec<FeedPath>>;

    /// Reads a post's audience targeting as a [`Vec<AudienceTarget>`], for
    /// pre-selecting the editor's audience picker.
    ///
    /// Owner-only: this performs no viewer resolution and is intended to be
    /// called for a post the caller already owns. Maps each `post_audiences`
    /// row back to its [`AudienceTarget`] (`public` → [`AudienceTarget::Public`],
    /// `subscribers` → [`AudienceTarget::Subscribers`], `named` →
    /// [`AudienceTarget::Named`]); a post with no rows yields an empty vec
    /// (equivalent to [`AudienceTarget::Private`]). See ADR-0020.
    async fn get_post_audiences(&self, post_id: PostId) -> Result<Vec<AudienceTarget>>;

    /// Loads a bounded exact-reference snapshot for a media identity.
    ///
    /// `has_unexamined_references` reports the sentinel row, which remains live
    /// because it receives no foreign evidence.
    async fn list_media_references(&self, media: &MediaRef) -> Result<MediaReferenceSnapshot>;

    async fn list_posts_referencing_media(
        &self,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<Vec<PostId>>;
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
/// [`unpublish_post`][PostDialect::unpublish_post] is likewise dialect-specific
/// only for its complete `UPDATE ... RETURNING` JSON tag aggregate. Everything
/// else is shared on [`PostStore`]. See ADR-0019.
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

    /// Acquires the backend's transaction-scoped media locks in the stable order
    /// represented by `media`. `SQLite` already holds its single writer lock.
    async fn lock_media_references(
        conn: &mut Self::Connection,
        media: &BTreeSet<MediaRef>,
    ) -> Result<()>;

    /// Serializes this `(user_id, key)` with competing creates and returns its
    /// live mapping under a row lock. `SQLite` already holds its writer lock;
    /// `PostgreSQL` additionally takes a transaction-scoped advisory lock so an
    /// absent mapping is serialized too.
    async fn lock_live_idempotency_mapping(
        conn: &mut Self::Connection,
        user_id: UserId,
        key: &IdempotencyKey,
        cutoff: UtcInstant,
    ) -> sqlx::Result<Option<PostId>>;

    /// Deletes every `post_media` row for a post. Bind order: `post_id`.
    const DELETE_POST_MEDIA: &'static str;

    /// Updates a post and records a revision, returning the updated record.
    async fn update_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Publishes a live owner's post through the backend's lifecycle revision
    /// transaction. A matching current state is returned unchanged.
    async fn publish_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<Option<PostRecord>, sqlx::Error>;

    /// Soft-deletes a live owner's row under the backend's lifecycle revision
    /// transaction. Returns false when ownership or liveness did not match.
    async fn soft_delete_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<bool, sqlx::Error>;

    /// Reverts a live owner's publication state under the backend's lifecycle
    /// transaction. A draft is returned unchanged.
    async fn unpublish_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<Option<PostRecord>, sqlx::Error>;

    /// Reconcile an active owner's post tags to `desired` in one transaction.
    /// Monomorphised because the **serialization** differs: `SQLite` opens
    /// `BEGIN IMMEDIATE`, `PostgreSQL` locks the post row with `FOR UPDATE`
    /// (ADR-0019, ADR-0021). The statements it issues are shared, not
    /// per-dialect (#876).
    async fn set_post_tags(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError>;
    /// Atomically installs references re-derived outside the writer lock.
    ///
    /// The backend rejects the batch if an authoritative HTML snapshot changed after
    /// derivation, so startup fails safely and a later open derives a fresh batch.
    async fn apply_post_media_reference_backfill(
        pool: &Pool<Self>,
        candidates: &[PostMediaReferenceBackfill],
    ) -> Result<()>;

    /// Inserts a deduplicated `post_media` batch in one statement.
    async fn insert_post_media_rows(
        conn: &mut Self::Connection,
        rows: BTreeSet<(PostId, MediaRef, MediaReferenceKind, MediaReferenceForm)>,
    ) -> Result<()>;
    /// Lists the owner's retained references after excluding evidence proved foreign for
    /// `current_instance_id`. Dynamic evidence binds require a concrete `SQLx` dialect.
    async fn list_posts_referencing_media(
        pool: &Pool<Self>,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<Vec<PostId>>;
}

/// This is decoded explicitly rather than through a positional `SQLx` tuple so
/// persisted values always cross the storage boundary as domain types.
struct RevisionDetailRow {
    revision_id: RevisionId,
    post_id: PostId,
    user_id: UserId,
    title: Option<PostTitle>,
    slug: Slug,
    body: PostBody,
    format: PostFormat,
    rendered_html: RenderedHtml,
    summary: Option<PostSummary>,
    created_at: UtcInstant,
    updated_at: UtcInstant,
    published_at: Option<UtcInstant>,
    deleted_at: Option<UtcInstant>,
    captured_at: UtcInstant,
}

/// The typed columns required to render one immutable revision in a history list.
struct RevisionMetadataRow {
    revision_id: RevisionId,
    post_id: PostId,
    title: Option<PostTitle>,
    slug: Slug,
    captured_at: UtcInstant,
    deleted_at: Option<UtcInstant>,
    published_at: Option<UtcInstant>,
    current_deleted_at: Option<UtcInstant>,
}

/// Decodes a raw SQL row into a storage-internal typed projection.
trait DecodeRawRow<DB: Database>: Sized {
    fn decode(row: DB::Row) -> Result<Self>;
}

impl<DB> DecodeRawRow<DB> for RevisionDetailRow
where
    DB: Database,
    for<'r> RevisionId: Decode<'r, DB> + Type<DB>,
    for<'r> PostId: Decode<'r, DB> + Type<DB>,
    for<'r> UserId: Decode<'r, DB> + Type<DB>,
    for<'r> Option<PostTitle>: Decode<'r, DB> + Type<DB>,
    for<'r> Slug: Decode<'r, DB> + Type<DB>,
    for<'r> PostBody: Decode<'r, DB> + Type<DB>,
    for<'r> PostFormat: Decode<'r, DB> + Type<DB>,
    for<'r> RenderedHtml: Decode<'r, DB> + Type<DB>,
    for<'r> Option<PostSummary>: Decode<'r, DB> + Type<DB>,
    for<'r> UtcInstant: Decode<'r, DB> + Type<DB>,
    for<'r> Option<UtcInstant>: Decode<'r, DB> + Type<DB>,
    for<'r> &'r str: sqlx::ColumnIndex<DB::Row>,
{
    fn decode(row: DB::Row) -> Result<Self> {
        Ok(Self {
            revision_id: row.try_get::<RevisionId, _>("revision_id")?,
            post_id: row.try_get::<PostId, _>("post_id")?,
            user_id: row.try_get::<UserId, _>("user_id")?,
            title: row.try_get::<Option<PostTitle>, _>("title")?,
            slug: row.try_get::<Slug, _>("slug")?,
            body: row.try_get::<PostBody, _>("body")?,
            format: row.try_get::<PostFormat, _>("format")?,
            rendered_html: row.try_get::<RenderedHtml, _>("rendered_html")?,
            summary: row.try_get::<Option<PostSummary>, _>("summary")?,
            created_at: row.try_get::<UtcInstant, _>("created_at")?,
            updated_at: row.try_get::<UtcInstant, _>("updated_at")?,
            published_at: row.try_get::<Option<UtcInstant>, _>("published_at")?,
            deleted_at: row.try_get::<Option<UtcInstant>, _>("deleted_at")?,
            captured_at: row.try_get::<UtcInstant, _>("captured_at")?,
        })
    }
}

impl<DB> DecodeRawRow<DB> for RevisionMetadataRow
where
    DB: Database,
    for<'r> RevisionId: Decode<'r, DB> + Type<DB>,
    for<'r> PostId: Decode<'r, DB> + Type<DB>,
    for<'r> Option<PostTitle>: Decode<'r, DB> + Type<DB>,
    for<'r> Slug: Decode<'r, DB> + Type<DB>,
    for<'r> UtcInstant: Decode<'r, DB> + Type<DB>,
    for<'r> Option<UtcInstant>: Decode<'r, DB> + Type<DB>,
    for<'r> &'r str: sqlx::ColumnIndex<DB::Row>,
{
    fn decode(row: DB::Row) -> Result<Self> {
        Ok(Self {
            revision_id: row.try_get::<RevisionId, _>("revision_id")?,
            post_id: row.try_get::<PostId, _>("post_id")?,
            title: row.try_get::<Option<PostTitle>, _>("title")?,
            slug: row.try_get::<Slug, _>("slug")?,
            captured_at: row.try_get::<UtcInstant, _>("captured_at")?,
            deleted_at: row.try_get::<Option<UtcInstant>, _>("deleted_at")?,
            published_at: row.try_get::<Option<UtcInstant>, _>("published_at")?,
            current_deleted_at: row.try_get::<Option<UtcInstant>, _>("current_deleted_at")?,
        })
    }
}

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
    (Exists,): for<'r> sqlx::FromRow<'r, DB::Row>,
    PostTag: for<'r> sqlx::FromRow<'r, DB::Row>,
    TagRecord: for<'r> sqlx::FromRow<'r, DB::Row>,
    (TargetKind, Option<AudienceId>): for<'r> sqlx::FromRow<'r, DB::Row>,
    (UtcInstant,): for<'r> sqlx::FromRow<'r, DB::Row>,
    RevisionDetailRow: DecodeRawRow<DB>,
    (Tag, TagLabel): for<'r> sqlx::FromRow<'r, DB::Row>,
    (
        MediaSource,
        ContentHash,
        Filename,
        MediaReferenceKind,
        MediaReferenceForm,
    ): for<'r> sqlx::FromRow<'r, DB::Row>,
    (
        PostId,
        Option<PostTitle>,
        Slug,
        PostFormat,
        UtcInstant,
        UtcInstant,
        Option<UtcInstant>,
        Option<UtcInstant>,
    ): for<'r> sqlx::FromRow<'r, DB::Row>,
    RevisionMetadataRow: DecodeRawRow<DB>,
    // `feed_urls_needing_catchup` reads `feed_cache` a row at a time (a bad `feed_url`
    // must not fail the scan), so it needs the column-decode bounds directly rather than
    // a `FromRow` tuple. `FeedPath` decodes as itself via the ADR-0071 bridge.
    for<'r> FeedPath: Decode<'r, DB> + Type<DB>,
    for<'r> UtcInstant: Decode<'r, DB> + Type<DB>,
    for<'r> &'r str: sqlx::ColumnIndex<DB::Row>,
    usize: sqlx::ColumnIndex<DB::Row>,
    // Every post-media column decodes as its domain type through ADR-0071's
    // bridge; this tuple keeps the reference form typed at the SQL boundary.
    (
        ContentHash,
        Filename,
        MediaReferenceKind,
        MediaReferenceForm,
    ): for<'r> sqlx::FromRow<'r, DB::Row>,
    (
        PostId,
        UserId,
        MediaSource,
        ContentHash,
        Filename,
        MediaReferenceKind,
        MediaReferenceForm,
    ): for<'r> sqlx::FromRow<'r, DB::Row>,
    (
        PostId,
        UserId,
        RevisionId,
        MediaSource,
        ContentHash,
        Filename,
        MediaReferenceKind,
        MediaReferenceForm,
    ): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> MediaReferenceKind: Encode<'q, DB> + Type<DB>,
    for<'q> MediaSource: Encode<'q, DB> + Type<DB>,
    for<'q> &'q ContentHash: Encode<'q, DB> + Type<DB>,
    for<'q> &'q Filename: Encode<'q, DB> + Type<DB>,
    for<'q> MediaReferenceForm: Encode<'q, DB> + Type<DB>,
    // makes every id newtype bind on a generic backend.
    for<'q> i64: Decode<'q, DB> + Encode<'q, DB> + Type<DB>,
    for<'q> RowCount: Decode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q str>: Encode<'q, DB> + Type<DB>,
    for<'q> Option<String>: Encode<'q, DB> + Type<DB>,
    // The viewer-resolution binds are NULL-able (`ResolutionBinds::bind_onto`).
    for<'q> Option<UserId>: Encode<'q, DB> + Type<DB>,
    for<'q> Option<ChannelId>: Encode<'q, DB> + Type<DB>,
    for<'q> &'q SubscriberRef: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q SubscriberRef>: Encode<'q, DB> + Type<DB>,
    // `Slug`/`Tag`/`Username` bind and decode as themselves via the ADR-0071 sqlx
    // into their newtypes). The `Option<&PostTitle>` bound is the nullable `title`
    // bind, forwarded from `write_post_in_tx` (create paths).
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q IdempotencyKey: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q PostTitle>: Encode<'q, DB> + Type<DB>,
    // `summary` binds as `Option<&PostSummary>` via the ADR-0071 sqlx bridge on
    // the create paths, mirroring the `Option<&PostTitle>` bound above.
    for<'q> Option<&'q PostSummary>: Encode<'q, DB> + Type<DB>,
    for<'q> Option<AudienceId>: Encode<'q, DB> + Type<DB>,
    // `RowLimit` binds as itself via the ADR-0071 sqlx bridge (delegates to `i64`) —
    // every listing's `LIMIT` placeholder (#696).
    for<'q> RowLimit: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'q> Option<UtcInstant>: Encode<'q, DB> + Type<DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.posts.create",
        skip(self, transaction, input),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn create_post(
        &self,
        transaction: &mut WriteTransaction,
        input: &CreatePostInput,
        now: UtcInstant,
    ) -> Result<CreatedPost, CreatePostError> {
        let connection = DB::write_connection(transaction)?;
        let (post_id, idempotency_key_expired) =
            write_post_in_tx::<DB>(connection, input, now).await?;
        let sql = format!(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format,
                    p.rendered_html, p.created_at, p.updated_at, p.published_at, p.deleted_at,
                    p.summary, {tags} AS tags
             FROM posts p JOIN users u ON p.user_id = u.user_id WHERE p.post_id = $1",
            tags = DB::TAGS_SUBQUERY,
        );
        let record = sqlx::query_as::<_, PostRecord>(&sql)
            .bind_storage(post_id)
            .fetch_one(connection)
            .await?;
        Ok(CreatedPost {
            record,
            idempotency_key_expired,
        })
    }

    #[tracing::instrument(
        name = "storage.posts.create_batch",
        skip(self, transaction, inputs),
        fields(db.system = DB::DB_SYSTEM, count = inputs.len())
    )]
    async fn create_posts(
        &self,
        transaction: &mut WriteTransaction,
        inputs: &[CreatePostInput],
    ) -> Result<Vec<PostId>, CreatePostError> {
        let connection = DB::write_connection(transaction)?;
        let media = inputs
            .iter()
            .flat_map(|input| {
                input
                    .rendered
                    .media()
                    .iter()
                    .map(|reference| reference.media().clone())
            })
            .collect();
        DB::lock_media_references(connection, &media).await?;
        let mut ids = Vec::with_capacity(inputs.len());
        for input in inputs {
            let (post_id, _) = write_post_in_tx::<DB>(connection, input, UtcInstant::now()).await?;
            ids.push(post_id);
        }
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
        key: &IdempotencyKey,
        now: UtcInstant,
    ) -> Result<Option<PostId>, sqlx::Error> {
        let cutoff = idempotency_replay_cutoff(now);
        sqlx::query_scalar::<_, PostId>(
            "SELECT post_id FROM idempotency_keys
             WHERE user_id = $1 AND key = $2 AND created_at > $3",
        )
        .bind_storage(user_id)
        .bind_storage(key)
        .bind_storage(cutoff)
        .fetch_optional(&self.pool)
        .await
    }

    #[tracing::instrument(
        name = "storage.posts.prune_expired_idempotency_keys",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn prune_expired_idempotency_keys(&self, now: UtcInstant) -> Result<u64, sqlx::Error> {
        const BATCH_SIZE: RowLimit = RowLimit::at_most(100);
        let cutoff = idempotency_replay_cutoff(now);
        let mut deleted = 0;

        loop {
            let batch = sqlx::query_scalar::<_, RowCount>(
                "DELETE FROM idempotency_keys
                 WHERE idempotency_key_id IN (
                     SELECT idempotency_key_id FROM idempotency_keys
                     WHERE created_at <= $1
                     ORDER BY idempotency_key_id
                     LIMIT $2
                 )
                 RETURNING CAST(1 AS BIGINT)",
            )
            .bind_storage(cutoff)
            .bind_storage(BATCH_SIZE)
            .fetch_all(&self.pool)
            .await?
            .len() as u64;
            if batch > 0 {
                metrics::retention_pruned(Domain::IdempotencyKeys, batch);
            }
            deleted += batch;
            if batch < BATCH_SIZE.value().unsigned_abs() {
                return Ok(deleted);
            }
        }
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
    ) -> Result<Option<PostRecord>> {
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
        let query = sqlx::query_as::<_, PostRecord>(&sql).bind_storage(post_id);
        Ok(binds.bind_onto(query).fetch_optional(&self.pool).await?)
    }

    #[tracing::instrument(name = "storage.posts.list_owned_revision_history", skip(self))]
    async fn list_owned_revision_history(
        &self,
        user_id: UserId,
        cursor: Option<PostRevisionCursor>,
        page_size: PageSize,
    ) -> Result<PostRevisionPage> {
        let rows =
            revision_metadata_rows(&self.pool, user_id, None, cursor, page_size.fetch_limit())
                .await?;
        Ok(revision_page(rows, page_size))
    }

    #[tracing::instrument(name = "storage.posts.list_post_revision_history", skip(self))]
    async fn list_post_revision_history(
        &self,
        user_id: UserId,
        post_id: PostId,
        cursor: Option<PostRevisionCursor>,
        page_size: PageSize,
    ) -> Result<Option<PostRevisionPage>> {
        let owned = sqlx::query_scalar::<_, PostId>(
            "SELECT post_id FROM posts WHERE post_id = $1 AND user_id = $2",
        )
        .bind_storage(post_id)
        .bind_storage(user_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(_) = owned else {
            return Ok(None);
        };
        let rows = revision_metadata_rows(
            &self.pool,
            user_id,
            Some(post_id),
            cursor,
            page_size.fetch_limit(),
        )
        .await?;
        Ok(Some(revision_page(rows, page_size)))
    }

    #[tracing::instrument(name = "storage.posts.get_current_revision_summary", skip(self))]
    async fn get_current_revision_summary(
        &self,
        user_id: UserId,
        post_id: PostId,
        now: UtcInstant,
    ) -> Result<Option<CurrentPostRevisionSummary>> {
        let row: Option<(
            PostId,
            Option<PostTitle>,
            Slug,
            PostFormat,
            UtcInstant,
            UtcInstant,
            Option<UtcInstant>,
            Option<UtcInstant>,
        )> = sqlx::query_as(
            "SELECT post_id, title, slug, format, created_at, updated_at, published_at, deleted_at
             FROM posts WHERE post_id = $1 AND user_id = $2",
        )
        .bind_storage(post_id)
        .bind_storage(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(
            |(post_id, title, slug, format, created_at, updated_at, published_at, deleted_at)| {
                CurrentPostRevisionSummary {
                    post_id,
                    title,
                    slug,
                    format,
                    created_at,
                    updated_at,
                    published_at,
                    deleted_at,
                    lifecycle: post_lifecycle(deleted_at, published_at, now),
                }
            },
        ))
    }

    #[tracing::instrument(name = "storage.posts.get_post_revision_detail", skip(self))]
    async fn get_post_revision_detail(
        &self,
        user_id: UserId,
        post_id: PostId,
        revision_id: RevisionId,
    ) -> Result<Option<PostRevisionDetail>> {
        let row = sqlx::query(
            "SELECT revision_id, post_id, user_id, title, slug, body, format, rendered_html,
                    summary, created_at, updated_at, published_at, deleted_at, captured_at
             FROM post_revisions
             WHERE revision_id = $1 AND post_id = $2 AND user_id = $3",
        )
        .bind_storage(revision_id)
        .bind_storage(post_id)
        .bind_storage(user_id)
        .fetch_optional(&self.pool)
        .await?
        .map(RevisionDetailRow::decode)
        .transpose()?;
        let Some(RevisionDetailRow {
            revision_id,
            post_id,
            user_id,
            title,
            slug,
            body,
            format,
            rendered_html,
            summary,
            created_at,
            updated_at,
            published_at,
            deleted_at,
            captured_at,
        }) = row
        else {
            return Ok(None);
        };

        let tags: Vec<(Tag, TagLabel)> = sqlx::query_as(
            "SELECT tag_slug, tag_display FROM post_revision_tags
             WHERE revision_id = $1 ORDER BY tag_slug",
        )
        .bind_storage(revision_id)
        .fetch_all(&self.pool)
        .await?;
        let audiences: Vec<(TargetKind, Option<AudienceId>)> = sqlx::query_as(
            "SELECT target_kind, audience_id FROM post_revision_audiences
             WHERE revision_id = $1 ORDER BY target_kind, audience_id",
        )
        .bind_storage(revision_id)
        .fetch_all(&self.pool)
        .await?;
        let media: Vec<(
            MediaSource,
            ContentHash,
            Filename,
            MediaReferenceKind,
            MediaReferenceForm,
        )> = sqlx::query_as(
            "SELECT source, sha256, filename, reference_kind, reference_form
             FROM post_media
             WHERE post_id = $1 AND subject_kind = 'revision' AND revision_id = $2
             ORDER BY source, sha256, filename, reference_kind, reference_form",
        )
        .bind_storage(post_id)
        .bind_storage(revision_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(Some(PostRevisionDetail {
            revision: PostRevisionRecord {
                revision_id,
                post_id,
                user_id,
                title,
                slug,
                body,
                format,
                rendered_html,
                summary,
                created_at,
                updated_at,
                published_at,
                deleted_at,
                captured_at,
                tags: tags
                    .into_iter()
                    .map(|(tag, display)| PostRevisionTag { tag, display })
                    .collect(),
                audiences: audiences
                    .into_iter()
                    .filter_map(|(kind, audience_id)| audience_target_from_row(kind, audience_id))
                    .collect(),
                media: media
                    .into_iter()
                    .map(|(_, _, _, _, form)| {
                        let Some(reference) = common::media::parse_media_url(form.as_ref()) else {
                            unreachable!("MediaReferenceForm decodes only exact parser output");
                        };
                        reference
                    })
                    .collect(),
            },
        }))
    }

    #[tracing::instrument(
        name = "storage.posts.get_audiences",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_post_audiences(&self, post_id: PostId) -> Result<Vec<AudienceTarget>> {
        // Owner-only: no viewer resolution. `ORDER BY` makes the result
        // deterministic so callers can compare vecs directly.
        let rows: Vec<(TargetKind, Option<AudienceId>)> = sqlx::query_as(
            "SELECT tk.name, pa.audience_id \
             FROM post_audiences pa \
             JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id \
             WHERE pa.post_id = $1 \
             ORDER BY tk.name, pa.audience_id",
        )
        .bind_storage(post_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, audience_id)| audience_target_from_row(kind, audience_id))
            .collect())
    }

    #[tracing::instrument(
        name = "storage.posts.list_media_references",
        skip(self, media),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_media_references(&self, media: &MediaRef) -> Result<MediaReferenceSnapshot> {
        let mut rows: Vec<(
            PostId,
            UserId,
            RevisionId,
            MediaSource,
            ContentHash,
            Filename,
            MediaReferenceKind,
            MediaReferenceForm,
        )> = sqlx::query_as(
            "SELECT pm.post_id, p.user_id, pm.revision_id, pm.source, pm.sha256, pm.filename, \
                    pm.reference_kind, pm.reference_form
             FROM post_media pm
             JOIN posts p ON p.post_id = pm.post_id
             WHERE pm.source = $1 AND pm.sha256 = $2 AND pm.filename = $3
             ORDER BY pm.post_id, pm.subject_kind, pm.revision_id, pm.reference_kind, pm.reference_form
             LIMIT $4",
        )
        .bind_storage(media.source)
        .bind_storage(&media.sha256)
        .bind_storage(&media.filename)
        .bind_storage(media::MEDIA_REFERENCE_SNAPSHOT_QUERY_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        let has_unexamined_references = rows.len() > media::MAX_MEDIA_REFERENCE_SNAPSHOT;
        rows.truncate(media::MAX_MEDIA_REFERENCE_SNAPSHOT);
        Ok(MediaReferenceSnapshot::new(
            rows.into_iter()
                .map(
                    |(
                        post_id,
                        owner_id,
                        revision_id,
                        source,
                        sha256,
                        filename,
                        kind,
                        reference_form,
                    )| {
                        let subject = if revision_id == RevisionId::from(0) {
                            PersistedMediaSubject::Current
                        } else {
                            PersistedMediaSubject::Revision(revision_id)
                        };
                        PersistedMediaReference::for_subject(
                            post_id,
                            subject,
                            MediaRef {
                                source,
                                sha256,
                                filename,
                            },
                            kind,
                            reference_form,
                        )
                        .with_owner(owner_id)
                    },
                )
                .collect(),
            has_unexamined_references,
        ))
    }

    #[tracing::instrument(
        name = "storage.posts.list_referencing_media",
        skip(self, media, evidence),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_posts_referencing_media(
        &self,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<Vec<PostId>> {
        DB::list_posts_referencing_media(&self.pool, user_id, media, current_instance_id, evidence)
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
        now: UtcInstant,
    ) -> Result<Option<PostRecord>> {
        let date_text = PermalinkDateText::from(date);
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
            .bind_storage(username)
            .bind_storage(slug)
            .bind_storage(date_text)
            .bind_storage(now);
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
        now: UtcInstant,
    ) -> Result<Option<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let date_clause = DB::PERMALINK_DATE_CLAUSE;
        let date_text = PermalinkDateText::from(date);
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
            .bind_storage(user_id)
            .bind_storage(slug)
            .bind_storage(date_text)
            .bind_storage(now)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    #[tracing::instrument(
        name = "storage.posts.update",
        skip(self, transaction, input),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn update_post(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        DB::update_post(transaction, post_id, editor_user_id, input).await
    }

    #[tracing::instrument(
        name = "storage.posts.publish",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn publish_post(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<PostRecord, UpdatePostError> {
        if let Some(row) = DB::publish_post(transaction, post_id, user_id).await? {
            return Ok(row);
        }
        let connection = DB::write_connection(transaction)?;
        let live = sqlx::query_scalar::<_, PostId>(
            "SELECT post_id FROM posts WHERE post_id = $1 AND deleted_at IS NULL",
        )
        .bind_storage(post_id)
        .fetch_optional(&mut *connection)
        .await?;
        Err(if live.is_some() {
            UpdatePostError::Unauthorized
        } else {
            UpdatePostError::NotFound
        })
    }

    #[tracing::instrument(
        name = "storage.posts.soft_delete",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn soft_delete_post(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<(), UpdatePostError> {
        if DB::soft_delete_post(transaction, post_id, user_id).await? {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        let live = sqlx::query_scalar::<_, PostId>(
            "SELECT post_id FROM posts WHERE post_id = $1 AND deleted_at IS NULL",
        )
        .bind_storage(post_id)
        .fetch_optional(&mut *connection)
        .await?;
        Err(if live.is_some() {
            UpdatePostError::Unauthorized
        } else {
            UpdatePostError::NotFound
        })
    }

    #[tracing::instrument(
        name = "storage.posts.unpublish",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn unpublish_post(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<PostRecord, UpdatePostError> {
        if let Some(row) = DB::unpublish_post(transaction, post_id, user_id).await? {
            return Ok(row);
        }
        let connection = DB::write_connection(transaction)?;
        let live = sqlx::query_scalar::<_, PostId>(
            "SELECT post_id FROM posts WHERE post_id = $1 AND deleted_at IS NULL",
        )
        .bind_storage(post_id)
        .fetch_optional(&mut *connection)
        .await?;
        Err(if live.is_some() {
            UpdatePostError::Unauthorized
        } else {
            UpdatePostError::NotFound
        })
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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>> {
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
                .bind_storage(username)
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.post_id)
                .bind_storage(now);
            binds
                .bind_onto(query)
                .bind_storage(limit)
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
                .bind_storage(username)
                .bind_storage(now);
            binds
                .bind_onto(query)
                .bind_storage(limit)
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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>> {
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
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.post_id)
                .bind_storage(now);
            binds
                .bind_onto(query)
                .bind_storage(limit)
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
            let query = sqlx::query_as::<_, PostRecord>(&sql).bind_storage(now);
            binds
                .bind_onto(query)
                .bind_storage(limit)
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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>> {
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
                .bind_storage(user_id)
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.post_id)
                .bind_storage(now)
                .bind_storage(limit)
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
                .bind_storage(user_id)
                .bind_storage(now)
                .bind_storage(limit)
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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>> {
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
                .bind_storage(user_id)
                .bind_storage(cursor.published_at)
                .bind_storage(cursor.published_at)
                .bind_storage(cursor.post_id)
                .bind_storage(now)
                .bind_storage(limit)
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
                .bind_storage(user_id)
                .bind_storage(now)
                .bind_storage(limit)
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
    ) -> Result<Vec<PostRecord>> {
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
                .bind_storage(user_id)
                .bind_storage(cursor.updated_at)
                .bind_storage(cursor.post_id)
                .bind_storage(limit)
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
                .bind_storage(user_id)
                .bind_storage(limit)
                .fetch_all(&self.pool)
                .await?
        };
        Ok(rows)
    }

    #[tracing::instrument(
        name = "storage.posts.set_post_tags",
        skip(self, transaction, desired),
        fields(db.system = DB::DB_SYSTEM, tag_count = desired.len())
    )]
    async fn set_post_tags(
        &self,
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError> {
        DB::set_post_tags(transaction, post_id, user_id, desired).await
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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists = sqlx::query_scalar::<_, Exists>(tags::TAG_EXISTS_SQL)
            .bind_storage(tag_slug)
            .fetch_one(&self.pool)
            .await?
            .into_bool();

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
                .bind_storage(tag_slug)
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.post_id)
                .bind_storage(now);
            binds
                .bind_onto(query)
                .bind_storage(limit)
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
                .bind_storage(tag_slug)
                .bind_storage(now);
            binds
                .bind_onto(query)
                .bind_storage(limit)
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
        now: UtcInstant,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists = sqlx::query_scalar::<_, Exists>(tags::TAG_EXISTS_SQL)
            .bind_storage(tag_slug)
            .fetch_one(&self.pool)
            .await?
            .into_bool();

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
                .bind_storage(user_id)
                .bind_storage(tag_slug)
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.created_at)
                .bind_storage(cursor.post_id)
                .bind_storage(now);
            binds
                .bind_onto(query)
                .bind_storage(limit)
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
                .bind_storage(user_id)
                .bind_storage(tag_slug)
                .bind_storage(now);
            binds
                .bind_onto(query)
                .bind_storage(limit)
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
    ) -> Result<Vec<TagRecord>> {
        let normalized = prefix
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_ascii_lowercase);
        let pattern = normalized
            .as_deref()
            .map(tags::TagSlugPrefixPattern::from_normalized_prefix);

        // `tag_slug` decodes straight into `Tag` via the sqlx bridge (#438), so a
        // malformed stored value is rejected as a column-decode error above.
        match &pattern {
            Some(like) => {
                sqlx::query_as::<_, TagRecord>(
                    "SELECT tag_id, tag_slug FROM tags
                     WHERE tag_slug LIKE $1
                     ORDER BY tag_slug
                     LIMIT $2",
                )
                .bind_storage(like)
                .bind_storage(limit)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as::<_, TagRecord>(
                    "SELECT tag_id, tag_slug FROM tags
                     ORDER BY tag_slug
                     LIMIT $1",
                )
                .bind_storage(limit)
                .fetch_all(&self.pool)
                .await
            }
        }
    }

    #[tracing::instrument(
        name = "storage.posts.list_published_in_window",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_published_in_window(
        &self,
        surface: &common::feed::FeedSurface,
        window: &host::feed::HybridWindow,
        now: UtcInstant,
        viewer: &ViewerIdentity,
    ) -> Result<Vec<PostRecord>> {
        // ROW_NUMBER() identifies the top `min_items` posts; OR-combining with
        // `published_at >= cutoff` produces the hybrid-window union in a single
        // query. Only the JSON tag aggregation differs per backend, so the SQL
        // is shared via `DB::TAGS_SUBQUERY`.
        let cutoff = UtcInstant::from(window.cutoff_date(now.value()));
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
        after: UtcInstant,
        upto: UtcInstant,
    ) -> Result<Vec<GoLivePost>> {
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
            .bind_storage(after)
            .bind_storage(upto)
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

    async fn feed_urls_needing_catchup(&self, now: UtcInstant) -> Result<Vec<FeedPath>> {
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
/// Derives the stable lifecycle label from a state snapshot and an explicit
/// clock. Revision callers pass `captured_at`; current-summary callers pass
/// their request clock.
fn post_lifecycle(
    deleted_at: Option<UtcInstant>,
    published_at: Option<UtcInstant>,
    now: UtcInstant,
) -> PostLifecycle {
    if deleted_at.is_some() {
        PostLifecycle::Deleted
    } else if published_at.is_none() {
        PostLifecycle::Draft
    } else if published_at.is_some_and(|published_at| published_at > now) {
        PostLifecycle::Scheduled
    } else {
        PostLifecycle::Published
    }
}

fn revision_page(
    mut revisions: Vec<PostRevisionMetadata>,
    page_size: PageSize,
) -> PostRevisionPage {
    let has_more = page_size.has_more(revisions.len());
    revisions.truncate(page_size.page_len());
    let next_cursor = has_more
        .then(|| {
            revisions.last().map(|revision| PostRevisionCursor {
                revision_id: revision.revision_id,
            })
        })
        .flatten();
    PostRevisionPage {
        revisions,
        next_cursor,
    }
}

async fn revision_metadata_rows<DB>(
    pool: &Pool<DB>,
    user_id: UserId,
    post_id: Option<PostId>,
    cursor: Option<PostRevisionCursor>,
    limit: RowLimit,
) -> Result<Vec<PostRevisionMetadata>>
where
    DB: Database,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> UserId: Encode<'q, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Encode<'q, DB> + Type<DB>,
    for<'q> RowLimit: Encode<'q, DB> + Type<DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    RevisionMetadataRow: DecodeRawRow<DB>,
{
    let sql = if post_id.is_some() {
        "SELECT r.revision_id, r.post_id, r.title, r.slug, r.captured_at,
                r.deleted_at, r.published_at, p.deleted_at AS current_deleted_at
         FROM post_revisions r
         JOIN posts p ON p.post_id = r.post_id
         WHERE r.user_id = $1 AND r.post_id = $2 AND r.revision_id < $3
         ORDER BY r.revision_id DESC LIMIT $4"
    } else {
        "SELECT r.revision_id, r.post_id, r.title, r.slug, r.captured_at,
                r.deleted_at, r.published_at, p.deleted_at AS current_deleted_at
         FROM post_revisions r
         JOIN posts p ON p.post_id = r.post_id
         WHERE r.user_id = $1 AND r.revision_id < $2
         ORDER BY r.revision_id DESC LIMIT $3"
    };
    let after = cursor.map_or(RevisionId::from(i64::MAX), |cursor| cursor.revision_id);
    let rows = if let Some(post_id) = post_id {
        sqlx::query(sql)
            .bind_storage(user_id)
            .bind_storage(post_id)
            .bind_storage(after)
            .bind_storage(limit)
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query(sql)
            .bind_storage(user_id)
            .bind_storage(after)
            .bind_storage(limit)
            .fetch_all(pool)
            .await?
    };
    Ok(rows
        .into_iter()
        .map(RevisionMetadataRow::decode)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|row| PostRevisionMetadata {
            revision_id: row.revision_id,
            post_id: row.post_id,
            title: row.title,
            slug: row.slug,
            captured_at: row.captured_at,
            snapshot_lifecycle: post_lifecycle(row.deleted_at, row.published_at, row.captured_at),
            current_deleted: row.current_deleted_at.is_some(),
        })
        .collect())
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
            subref: visibility::local_subscriber_ref(*user_id),
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
        i64: Encode<'q, DB> + Type<DB>,
        &'q str: Encode<'q, DB> + Type<DB>,
        &'q SubscriberRef: Encode<'q, DB> + Type<DB>,
        Option<&'q SubscriberRef>: Encode<'q, DB> + Type<DB>,
        // sqlx implements `Encode for Option<T>` per concrete database (the
        // `impl_encode_for_option!` macro), not blanket over a generic `DB`, so
        // each NULL-able bind's type has to be restated here — and, per ADR-0019,
        // again on every caller.
        Option<UserId>: Encode<'q, DB> + Type<DB>,
        Option<ChannelId>: Encode<'q, DB> + Type<DB>,
        Option<&'q str>: Encode<'q, DB> + Type<DB>,
    {
        match self {
            Self::Anonymous => query
                .bind_storage(None::<UserId>)
                .bind_storage(None::<ChannelId>)
                .bind_storage(None::<&SubscriberRef>)
                .bind_storage(None::<ChannelId>)
                .bind_storage(None::<&SubscriberRef>),
            Self::Local { user_id, subref } => query
                .bind_storage(Some(*user_id))
                .bind_storage(Some(subref))
                .bind_storage(Some(subref)),
            Self::Remote { channel, subref } => query
                .bind_storage(None::<UserId>)
                .bind_storage(Some(*channel))
                .bind_storage(Some(subref))
                .bind_storage(Some(*channel))
                .bind_storage(Some(subref)),
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

fn create_expectations_match(input: &CreatePostInput) -> bool {
    let expected = &input.expectations;
    expected
        .slug
        .as_ref()
        .is_none_or(|slug| slug == &input.slug)
        && expected.format.is_none_or(|format| format == input.format)
        && expected
            .published_at
            .is_none_or(|published_at| published_at == input.published_at)
}

/// `PostgreSQL`'s signed advisory-lock key for one user/idempotency-key pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, macros::SqlxBridge)]
pub(crate) struct IdempotencyAdvisoryLockKey(i64);

/// Derives the `PostgreSQL` advisory-lock key for one user's `Idempotency Key`.
///
/// A collision only serializes unrelated creates; it cannot change behavior.
#[must_use]
pub(crate) fn idempotency_advisory_lock_key(
    user_id: UserId,
    key: &IdempotencyKey,
) -> IdempotencyAdvisoryLockKey {
    let mut digest = Sha256::new();
    digest.update(i64::from(user_id).to_be_bytes());
    digest.update(key.as_ref().as_bytes());
    let digest: [u8; 32] = digest.finalize().into();
    IdempotencyAdvisoryLockKey(i64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]))
}

pub(crate) fn update_scalar_is_noop(
    existing: &PostBookkeepingRow,
    input: &UpdatePostInput,
) -> bool {
    let published_at = match input.publish {
        PublishUpdate::Unpublish => None,
        PublishUpdate::Publish { at: Some(at) } => Some(at),
        PublishUpdate::Publish { at: None } => existing.published_at.or(Some(input.request_clock)),
    };
    existing.title == input.title
        && (existing.published_at.is_some() || existing.slug == input.slug)
        && existing.body == input.body
        && existing.format == input.format
        && existing.rendered_html.as_ref() == input.rendered.html().as_ref()
        && existing.summary == input.summary
        && existing.published_at == published_at
}
/// Compares audience collections as normalized relation rows, ignoring caller
/// order, duplicate selections, and `Private`'s deliberate absence of a row.
pub(crate) fn audiences_are_equal(
    existing: &[(TargetKind, Option<AudienceId>)],
    desired: &[AudienceTarget],
) -> bool {
    let mut normalized = Vec::new();
    for target in desired {
        if let Some(row) = audience_target_row(target)
            && !normalized.contains(&row)
        {
            normalized.push(row);
        }
    }
    existing.len() == normalized.len() && existing.iter().all(|row| normalized.contains(row))
}

pub(crate) fn update_expectation_error(
    post_id: PostId,
    existing: &PostBookkeepingRow,
    tags: &[TagLabel],
    input: &UpdatePostInput,
) -> Option<UpdatePostError> {
    let expected = &input.expectations;
    if expected
        .post_id
        .is_some_and(|expected_id| expected_id != post_id)
    {
        return Some(UpdatePostError::BookkeepingMismatch);
    }
    let final_slug = if existing.published_at.is_some() {
        &existing.slug
    } else {
        &input.slug
    };
    let final_published_at = match input.publish {
        PublishUpdate::Unpublish => None,
        PublishUpdate::Publish { at: Some(at) } => Some(at),
        PublishUpdate::Publish { at: None } => existing.published_at.or(Some(input.request_clock)),
    };
    if expected
        .slug
        .as_ref()
        .is_some_and(|slug| slug != final_slug)
        || expected.format.is_some_and(|format| format != input.format)
        || expected
            .published_at
            .is_some_and(|published_at| published_at != final_published_at)
    {
        return Some(UpdatePostError::BookkeepingMismatch);
    }

    let current_etag = etag::post_content_etag(
        existing.title.as_ref(),
        &existing.body,
        &existing.format,
        existing.summary.as_ref(),
        tags.iter(),
        existing.published_at.is_none(),
    );
    expected
        .content_etag
        .as_ref()
        .is_some_and(|etag| etag != &current_etag)
        .then_some(UpdatePostError::StaleContent)
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
    now: UtcInstant,
) -> Result<(PostId, bool), CreatePostError>
where
    DB: PostDialect,
    for<'q> i64: Decode<'q, DB> + Encode<'q, DB> + Type<DB>,
    for<'q> RowCount: Decode<'q, DB> + Type<DB>,
    for<'q> Option<AudienceId>: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q str>: Encode<'q, DB> + Type<DB>,
    for<'q> Option<String>: Encode<'q, DB> + Type<DB>,
    for<'q> &'q IdempotencyKey: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'q> Option<UtcInstant>: Encode<'q, DB> + Type<DB>,
    // `Slug`/`PostBody` bind as themselves and `PostTitle` as `Option<&PostTitle>`
    // via the ADR-0071 sqlx bridge (the `Option<&…>` pair covers the nullable
    // `title` bind).
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> Option<&'q PostTitle>: Encode<'q, DB> + Type<DB>,
    // `summary` binds as `Option<&PostSummary>` via the ADR-0071 sqlx bridge on
    // the create paths, mirroring the `Option<&PostTitle>` bound above.
    for<'q> Option<&'q PostSummary>: Encode<'q, DB> + Type<DB>,
    (PostId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    DB::lock_media_references(conn, &media::media_lock_set(input.rendered.media())).await?;

    let idempotency_key_expired = if let Some(key) = input.idempotency_key.as_ref() {
        let cutoff = idempotency_replay_cutoff(now);
        if let Some(post_id) =
            DB::lock_live_idempotency_mapping(conn, input.user_id, key, cutoff).await?
        {
            return Err(CreatePostError::IdempotencyConflict(post_id));
        }
        sqlx::query_scalar::<_, RowCount>(
            "DELETE FROM idempotency_keys
             WHERE user_id = $1 AND key = $2 AND created_at <= $3
             RETURNING CAST(1 AS BIGINT)",
        )
        .bind_storage(input.user_id)
        .bind_storage(key)
        .bind_storage(cutoff)
        .fetch_optional(&mut *conn)
        .await
        .map_err(CreatePostError::Internal)?
        .is_some()
    } else {
        false
    };

    let post_id = sqlx::query_scalar::<_, PostId>(
        "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at, summary)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING post_id",
    )
    .bind_storage(input.user_id)
    // `Option::as_ref` → `Option<&PostTitle>` (a typed newtype bind, not an
    // `AsRef<str>` strip); the sqlx bridge encodes `Option<&PostTitle>`.
    .bind_storage(input.title.as_ref())
    .bind_storage(&input.slug)
    .bind_storage(&input.body)
    .bind_storage(input.format)
    .bind_storage(input.rendered.html())
    .bind_storage(now)
    .bind_storage(now)
    .bind_storage(input.published_at)
    // `Option::as_ref` → `Option<&PostSummary>` (a typed newtype bind via the
    // ADR-0071 sqlx bridge, not an `AsRef<str>` strip); the `sqlx-newtype-bind`
    // gate forbids stripping to `&str` here.
    .bind_storage(input.summary.as_ref())
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => CreatePostError::SlugConflict,
        e => CreatePostError::Internal(e),
    })?;

    if !create_expectations_match(input) {
        return Err(CreatePostError::BookkeepingMismatch);
    }

    replace_post_audiences::<DB>(conn, post_id, &input.audiences).await?;
    media::replace_post_media::<DB>(conn, post_id, input.rendered.media()).await?;
    tags::insert_post_tags::<DB>(conn, post_id, &input.tags).await?;

    if let Some(key) = input.idempotency_key.as_ref() {
        sqlx::query(
            "INSERT INTO idempotency_keys (user_id, key, post_id, created_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind_storage(input.user_id)
        .bind_storage(key)
        .bind_storage(post_id)
        .bind_storage(now)
        .execute(&mut *conn)
        .await
        .map_err(CreatePostError::Internal)?;
    }

    Ok((post_id, idempotency_key_expired))
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
) -> Result<()>
where
    DB: PostDialect,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> Option<AudienceId>: Encode<'q, DB> + Type<DB>,
    for<'q> TargetKind: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    sqlx::query(DB::DELETE_POST_AUDIENCES)
        .bind_storage(post_id)
        .execute(&mut *conn)
        .await?;
    for target in audiences {
        if let Some((kind, audience_id)) = audience_target_row(target) {
            sqlx::query(DB::INSERT_POST_AUDIENCE)
                .bind_storage(post_id)
                .bind_storage(audience_id)
                .bind_storage(kind)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}
/// Captures the locked current state and every normalized child before mutation.
///
/// The copies are SQL-to-SQL rather than reconstructed from an application read:
/// this preserves the exact current media spelling and keeps immutable history
/// independent of later tag/audience lookup changes.
pub(crate) async fn capture_complete_post_revision<DB>(
    conn: &mut DB::Connection,
    post_id: PostId,
    captured_at: UtcInstant,
) -> Result<RevisionId>
where
    DB: Database,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Decode<'q, DB> + Type<DB>,
    for<'q> i64: Decode<'q, DB> + Encode<'q, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let revision_id = sqlx::query_scalar::<_, RevisionId>(INSERT_COMPLETE_POST_REVISION)
        .bind_storage(captured_at)
        .bind_storage(post_id)
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO post_revision_tags (revision_id, tag_slug, tag_display)
         SELECT $1, t.tag_slug, pt.tag_display
         FROM post_tags pt JOIN tags t ON t.tag_id = pt.tag_id
         WHERE pt.post_id = $2",
    )
    .bind_storage(revision_id)
    .bind_storage(post_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO post_revision_audiences (revision_id, target_kind, audience_id)
         SELECT $1, tk.name, pa.audience_id
         FROM post_audiences pa JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
         WHERE pa.post_id = $2",
    )
    .bind_storage(revision_id)
    .bind_storage(post_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO post_media
             (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form)
         SELECT post_id, 'revision', $1, source, sha256, filename, reference_kind, reference_form
         FROM post_media
         WHERE post_id = $2 AND subject_kind = 'current' AND revision_id = 0",
    )
    .bind_storage(revision_id)
    .bind_storage(post_id)
    .execute(&mut *conn)
    .await?;
    Ok(revision_id)
}

/// Runs the hybrid-window query for `surface`, returning [`PostRecord`]s.
///
/// Shared across backends: the four `FeedSurface` variants differ only in the
/// ranked-CTE source/predicate and bind list, and the JSON tag aggregation is
/// supplied by [`PostDialect::TAGS_SUBQUERY`].
async fn list_published_in_window_rows<DB>(
    pool: &Pool<DB>,
    surface: &common::feed::FeedSurface,
    now: UtcInstant,
    cutoff: UtcInstant,
    min_items: FeedMinItems,
    viewer: &ViewerIdentity,
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
    for<'q> &'q SubscriberRef: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q SubscriberRef>: Encode<'q, DB> + Type<DB>,
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
            // resolution fragment from $4. `window_sql` places it last, so
            // nothing binds after it and the returned `next` is discarded.
            let (resolution, binds, _) = resolution_where(viewer, 4);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRecord>(&sql)
                .bind_storage(now)
                .bind_storage(min_items)
                .bind_storage(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::User { username } => {
            // Binds: $1 now, $2 username, $3 min_items, $4 cutoff, then the
            // variant-sized resolution fragment last, from $5.
            let (resolution, binds, _) = resolution_where(viewer, 5);
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
            // variant-sized resolution fragment last, from $5.
            let (resolution, binds, _) = resolution_where(viewer, 5);
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
            // the variant-sized resolution fragment last, from $6.
            let (resolution, binds, _) = resolution_where(viewer, 6);
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
            sqlx::query_as(
                "SELECT p.published_at FROM posts p
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL
                 ORDER BY p.published_at DESC LIMIT 1",
            )
            .bind_storage(now)
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
            .bind_storage(now)
            .bind_storage(username)
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
            .bind_storage(now)
            .bind_storage(tag)
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
            .bind_storage(now)
            .bind_storage(username)
            .bind_storage(tag)
            .fetch_optional(pool)
            .await?
        }
    };
    Ok(row.map(|(published_at,)| published_at))
}

/// Database-provided physical identity retained only by the no-write regression.
#[cfg(test)]
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct PhysicalPostTagRowId(String);

#[cfg(test)]
impl PhysicalPostTagRowId {
    pub(crate) fn into_inner(self) -> String {
        self.0
    }
}
/// Deliberately malformed value for the `tags.tag_slug` decode fixture.
#[cfg(test)]
#[derive(macros::SqlxBridge)]
pub(crate) struct CorruptTagSlug(String);

/// Deliberately malformed value for the `posts.slug` decode fixture.
#[cfg(test)]
#[derive(macros::SqlxBridge)]
pub(crate) struct CorruptPostSlug(String);

/// Deliberately malformed value for the `posts.format` decode fixture.
#[cfg(test)]
#[derive(macros::SqlxBridge)]
pub(crate) struct CorruptPostFormat(String);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed_cache::FeedCacheRow;
    use crate::posts::cursors::{
        keyset_cursor, scheduled_keyset_cursor, to_scheduled_post_cursor, wire_cursor,
        wire_scheduled_cursor,
    };
    use crate::posts::models::PostBookkeepingExpectation;
    use crate::test_support::{
        Backend, CloseablePool, MEDIA_TEST_SHA256, SeedRawPost, SeedUser, TestEnv, UpdateRawPost,
        backends, create_draft_via_service, create_post_via_service, create_posts_confirmed,
        fetch_post_media, fp, media_ref_for, media_row_exists, media_url_for, seed_media,
        seed_users, set_post_tags_confirmed, update_post_body_via_service,
    };

    use chrono::Utc;
    use common::test_support::{
        parse_etag, parse_post_body, parse_post_summary, parse_post_title, parse_row_limit,
        parse_slug, parse_tag_label, parse_username, parse_utc_instant,
    };
    use common::time::UtcInstant;
    use host::feed::SyndicationFeedRepresentation;
    use rstest::*;
    use rstest_reuse::*;
    use sqlx::Row;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::Barrier;

    #[test]
    fn publication_state_projects_publish_update() {
        use common::org::PublicationState;

        let at = parse_utc_instant("2026-11-01T05:30:00Z");

        for (state, expected) in [
            (PublicationState::Draft, PublishUpdate::Unpublish),
            (
                PublicationState::Scheduled(at),
                PublishUpdate::Publish { at: Some(at) },
            ),
            (
                PublicationState::Published(at),
                PublishUpdate::Publish { at: Some(at) },
            ),
        ] {
            assert_eq!(PublishUpdate::from(state), expected);
        }
    }

    async fn update_post_scoped(
        state: &Arc<crate::AppState>,
        post_id: PostId,
        editor_user_id: UserId,
        input: UpdatePostInput,
    ) -> Result<common::MutationOutcome<PostRecord>, crate::WriteScopeError<UpdatePostError>> {
        let write_scope = state.write_scope.clone();
        let posts = Arc::clone(&state.posts);
        write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    posts
                        .update_post(transaction, post_id, editor_user_id, &input)
                        .await
                })
            })
            .await
    }

    async fn update_post_confirmed(
        state: &Arc<crate::AppState>,
        post_id: PostId,
        editor_user_id: UserId,
        input: UpdatePostInput,
    ) -> PostRecord {
        crate::test_support::confirmed_for(
            update_post_scoped(state, post_id, editor_user_id, input)
                .await
                .expect("post update succeeds"),
            "post update fixture",
        )
    }

    async fn create_post_confirmed(
        state: &Arc<crate::AppState>,
        input: CreatePostInput,
    ) -> PostRecord {
        let posts = Arc::clone(&state.posts);
        let write_scope = state.write_scope.clone();
        crate::test_support::confirmed_for(
            write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        posts
                            .create_post(transaction, &input, UtcInstant::now())
                            .await
                    })
                })
                .await
                .expect("post creation succeeds"),
            "post creation fixture",
        )
        .record
    }

    async fn publish_post_scoped(
        state: &Arc<crate::AppState>,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<common::MutationOutcome<PostRecord>, crate::WriteScopeError<UpdatePostError>> {
        let posts = Arc::clone(&state.posts);
        let write_scope = state.write_scope.clone();
        write_scope
            .run(move |transaction| {
                Box::pin(async move { posts.publish_post(transaction, post_id, user_id).await })
            })
            .await
    }

    async fn publish_post_confirmed(
        state: &Arc<crate::AppState>,
        post_id: PostId,
        user_id: UserId,
    ) -> PostRecord {
        crate::test_support::confirmed_for(
            publish_post_scoped(state, post_id, user_id)
                .await
                .expect("post publication succeeds"),
            "post publication fixture",
        )
    }

    async fn unpublish_post_scoped(
        state: &Arc<crate::AppState>,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<common::MutationOutcome<PostRecord>, crate::WriteScopeError<UpdatePostError>> {
        let posts = Arc::clone(&state.posts);
        let write_scope = state.write_scope.clone();
        write_scope
            .run(move |transaction| {
                Box::pin(async move { posts.unpublish_post(transaction, post_id, user_id).await })
            })
            .await
    }

    async fn unpublish_post_confirmed(
        state: &Arc<crate::AppState>,
        post_id: PostId,
        user_id: UserId,
    ) -> PostRecord {
        crate::test_support::confirmed_for(
            unpublish_post_scoped(state, post_id, user_id)
                .await
                .expect("post unpublication succeeds"),
            "post unpublication fixture",
        )
    }

    async fn soft_delete_post_scoped(
        state: &Arc<crate::AppState>,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<common::MutationOutcome<()>, crate::WriteScopeError<UpdatePostError>> {
        let posts = Arc::clone(&state.posts);
        let write_scope = state.write_scope.clone();
        write_scope
            .run(move |transaction| {
                Box::pin(async move { posts.soft_delete_post(transaction, post_id, user_id).await })
            })
            .await
    }

    async fn soft_delete_post_confirmed(
        state: &Arc<crate::AppState>,
        post_id: PostId,
        user_id: UserId,
    ) {
        crate::test_support::confirmed_for(
            soft_delete_post_scoped(state, post_id, user_id)
                .await
                .expect("post deletion succeeds"),
            "post deletion fixture",
        );
    }

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

    #[apply(backends)]
    #[tokio::test]
    async fn post_record_decodes_required_and_nullable_instants_at_microsecond_precision(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let post_id = SeedRawPost::new(user.user_id)
            .published_at(parse_utc_instant("2026-04-12T08:30:00.123456Z"))
            .seed(&env.state)
            .await
            .post_id;

        env.base
            .pool()
            .execute(
                "UPDATE posts SET
                   created_at = '2026-04-10T01:02:03.123456Z',
                   updated_at = '2026-04-11T04:05:06.654321Z',
                   deleted_at = '2026-04-13T07:08:09.999999Z'",
            )
            .await
            .unwrap();

        let record = env
            .state
            .posts
            .get_post_by_id(
                post_id,
                &ViewerIdentity::Local {
                    user_id: user.user_id,
                },
            )
            .await
            .unwrap()
            .expect("owner can decode a post record");

        assert_eq!(
            record.created_at,
            parse_utc_instant("2026-04-10T01:02:03.123456Z")
        );
        assert_eq!(
            record.updated_at,
            parse_utc_instant("2026-04-11T04:05:06.654321Z")
        );
        assert_eq!(
            record.published_at,
            Some(parse_utc_instant("2026-04-12T08:30:00.123456Z"))
        );
        assert_eq!(
            record.deleted_at,
            Some(parse_utc_instant("2026-04-13T07:08:09.999999Z"))
        );

        let draft_id = SeedRawPost::new(user.user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;
        let draft = env
            .state
            .posts
            .get_post_by_id(
                draft_id,
                &ViewerIdentity::Local {
                    user_id: user.user_id,
                },
            )
            .await
            .unwrap()
            .expect("owner can decode a draft record");
        assert_eq!(draft.published_at, None);
        assert_eq!(draft.deleted_at, None);
    }
    #[test]
    fn org_bookkeeping_conversion_preserves_each_persistence_expectation() {
        let published_at = "2026-08-26T12:00:00Z".parse().unwrap();
        let bookkeeping = common::org::OrgBookkeeping {
            slug: Some(parse_slug("expected-slug")),
            format: Some(PostFormat::Org),
            post_id: Some(PostId::from(7)),
            synced: Some(parse_etag("\"sha256-current\"")),
            synced_at: None,
            date_utc: Some(published_at),
        };

        let expectations: PostBookkeepingExpectation = bookkeeping.into();
        assert_eq!(expectations.slug, Some(parse_slug("expected-slug")));
        assert_eq!(expectations.format, Some(PostFormat::Org));
        assert_eq!(expectations.published_at, Some(Some(published_at)));
        assert_eq!(expectations.post_id, Some(PostId::from(7)));
        assert_eq!(
            expectations.content_etag,
            Some(parse_etag("\"sha256-current\""))
        );
    }

    /// Two independent edits whose old/new media sets are reversed must complete:
    /// `PostgreSQL` acquires their common advisory keys in one order, while `SQLite`
    /// serializes both writers through its immediate transaction.
    #[apply(backends)]
    #[tokio::test]
    async fn opposite_media_updates_complete_without_deadlock(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let first = media_url_for("first-lock.jpg");
        let second = media_url_for("second-lock.jpg");
        let first_post = create_post_via_service(
            &env.state,
            user,
            parse_post_body(&format!("<img src=\"{first}\">")),
        )
        .await;
        let second_post = create_post_via_service(
            &env.state,
            user,
            parse_post_body(&format!("<img src=\"{second}\">")),
        )
        .await;

        let barrier = Arc::new(Barrier::new(3));
        let first_update = tokio::spawn({
            let state = Arc::clone(&env.state);
            let barrier = Arc::clone(&barrier);
            let body = parse_post_body(&format!("<img src=\"{second}\">"));
            async move {
                barrier.wait().await;
                update_post_body_via_service(&state, first_post, user, body).await;
            }
        });
        let second_update = tokio::spawn({
            let state = Arc::clone(&env.state);
            let barrier = Arc::clone(&barrier);
            let body = parse_post_body(&format!("<img src=\"{first}\">"));
            async move {
                barrier.wait().await;
                update_post_body_via_service(&state, second_post, user, body).await;
            }
        });
        barrier.wait().await;

        tokio::time::timeout(Duration::from_secs(5), async {
            first_update
                .await
                .expect("first update task does not panic");
            second_update
                .await
                .expect("second update task does not panic");
        })
        .await
        .expect("opposite media updates must not deadlock");

        assert_eq!(
            fetch_post_media(&env.base, first_post).await[0].0,
            media_ref_for("second-lock.jpg"),
            "the first post completed its reversed update"
        );
        assert_eq!(
            fetch_post_media(&env.base, second_post).await[0].0,
            media_ref_for("first-lock.jpg"),
            "the second post completed its reversed update"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn reversed_media_batches_complete_without_deadlock(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let first = media_url_for("batch-first-lock.jpg");
        let second = media_url_for("batch-second-lock.jpg");
        let forward = [
            SeedRawPost::new(user)
                .body(parse_post_body(&format!("<img src=\"{first}\">")))
                .build(),
            SeedRawPost::new(user)
                .body(parse_post_body(&format!("<img src=\"{second}\">")))
                .build(),
        ];
        let reverse = [
            SeedRawPost::new(user)
                .body(parse_post_body(&format!("<img src=\"{second}\">")))
                .build(),
            SeedRawPost::new(user)
                .body(parse_post_body(&format!("<img src=\"{first}\">")))
                .build(),
        ];
        let forward_media: BTreeSet<_> = forward
            .iter()
            .flat_map(|input| input.rendered.media())
            .map(|reference| reference.media().clone())
            .collect();
        let reverse_media: BTreeSet<_> = reverse
            .iter()
            .flat_map(|input| input.rendered.media())
            .map(|reference| reference.media().clone())
            .collect();
        assert_eq!(
            forward_media, reverse_media,
            "each batch prelocks the full media union before writing either post"
        );

        let barrier = Arc::new(Barrier::new(3));
        let forward_create = tokio::spawn({
            let state = Arc::clone(&env.state);
            let barrier = Arc::clone(&barrier);
            async move {
                barrier.wait().await;
                create_posts_confirmed(&state, forward.to_vec()).await
            }
        });
        let reverse_create = tokio::spawn({
            let state = Arc::clone(&env.state);
            let barrier = Arc::clone(&barrier);
            async move {
                barrier.wait().await;
                create_posts_confirmed(&state, reverse.to_vec()).await
            }
        });
        barrier.wait().await;

        let (forward_ids, reverse_ids) = tokio::time::timeout(Duration::from_secs(5), async {
            (
                forward_create
                    .await
                    .expect("forward batch task does not panic"),
                reverse_create
                    .await
                    .expect("reverse batch task does not panic"),
            )
        })
        .await
        .expect("reversed batch transactions must not deadlock");
        assert_eq!(forward_ids.len(), 2);
        assert_eq!(reverse_ids.len(), 2);
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
                sqlx::query_scalar::<_, PhysicalPostTagRowId>(
                    "SELECT ctid::text FROM post_tags WHERE post_id = $1 ORDER BY tag_id",
                )
                .bind_storage(post_id)
                .fetch_all(pool)
                .await
            }
            CloseablePool::Sqlite(pool) => {
                sqlx::query_scalar::<_, PhysicalPostTagRowId>(
                    "SELECT CAST(rowid AS TEXT) FROM post_tags WHERE post_id = $1 ORDER BY tag_id",
                )
                .bind_storage(post_id)
                .fetch_all(pool)
                .await
            }
        }
        .expect("read physical row ids")
        .into_iter()
        .map(PhysicalPostTagRowId::into_inner)
        .collect()
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

    async fn owner_slugs_of(
        posts: &dyn PostStorage,
        post_id: PostId,
        user_id: UserId,
    ) -> Vec<String> {
        posts
            .get_post_by_id(post_id, &ViewerIdentity::Local { user_id })
            .await
            .expect("read owner post")
            .expect("owner post exists")
            .tags
            .iter()
            .map(|tag| tag.tag_slug.to_string())
            .collect()
    }

    #[derive(sqlx::FromRow)]
    struct RevisionRow {
        revision_id: RevisionId,
        post_id: PostId,
        user_id: UserId,
        title: Option<PostTitle>,
        slug: Slug,
        body: PostBody,
        format: PostFormat,
        rendered_html: RenderedHtml,
        summary: Option<PostSummary>,
        created_at: UtcInstant,
        updated_at: UtcInstant,
        published_at: Option<UtcInstant>,
        deleted_at: Option<UtcInstant>,
        captured_at: UtcInstant,
    }

    async fn single_revision(env: &TestEnv, post_id: PostId) -> RevisionRow {
        macro_rules! decode {
            ($row:expr) => {{
                let row = $row;
                RevisionRow {
                    revision_id: row.try_get::<RevisionId, _>("revision_id").unwrap(),
                    post_id: row.try_get::<PostId, _>("post_id").unwrap(),
                    user_id: row.try_get::<UserId, _>("user_id").unwrap(),
                    title: row.try_get::<Option<PostTitle>, _>("title").unwrap(),
                    slug: row.try_get::<Slug, _>("slug").unwrap(),
                    body: row.try_get::<PostBody, _>("body").unwrap(),
                    format: row.try_get::<PostFormat, _>("format").unwrap(),
                    rendered_html: row.try_get::<RenderedHtml, _>("rendered_html").unwrap(),
                    summary: row.try_get::<Option<PostSummary>, _>("summary").unwrap(),
                    created_at: row.try_get::<UtcInstant, _>("created_at").unwrap(),
                    updated_at: row.try_get::<UtcInstant, _>("updated_at").unwrap(),
                    published_at: row
                        .try_get::<Option<UtcInstant>, _>("published_at")
                        .unwrap(),
                    deleted_at: row.try_get::<Option<UtcInstant>, _>("deleted_at").unwrap(),
                    captured_at: row.try_get::<UtcInstant, _>("captured_at").unwrap(),
                }
            }};
        }
        let sql = "SELECT revision_id, post_id, user_id, title, slug, body, format, rendered_html, summary,
                          created_at, updated_at, published_at, deleted_at, captured_at
                   FROM post_revisions
                   WHERE post_id = $1";
        match env.base.pool() {
            CloseablePool::Postgres(pool) => decode!(
                sqlx::query(sql)
                    .bind_storage(post_id)
                    .fetch_one(pool)
                    .await
                    .expect("read revision")
            ),
            CloseablePool::Sqlite(pool) => decode!(
                sqlx::query(sql)
                    .bind_storage(post_id)
                    .fetch_one(pool)
                    .await
                    .expect("read revision")
            ),
        }
    }

    async fn media_for_subject(
        env: &TestEnv,
        post_id: PostId,
        subject_kind: &str,
        revision_id: RevisionId,
    ) -> Vec<(MediaRef, MediaReferenceKind, MediaReferenceForm)> {
        env.base
            .pool()
            .string_quintuples(&format!(
                "SELECT source, sha256, filename, reference_kind, reference_form
                 FROM post_media
                 WHERE post_id = {post_id} AND subject_kind = '{subject_kind}'
                   AND revision_id = {revision_id}
                 ORDER BY source, sha256, filename, reference_kind, reference_form"
            ))
            .await
            .expect("read post media subject")
            .into_iter()
            .map(|(source, sha256, filename, kind, form)| {
                (
                    MediaRef {
                        source: source.parse().expect("valid media source"),
                        sha256: sha256.parse().expect("valid media hash"),
                        filename: filename.parse().expect("valid media filename"),
                    },
                    kind.parse().expect("valid media reference kind"),
                    form.parse().expect("valid media reference form"),
                )
            })
            .collect()
    }

    async fn assert_complete_prior_revision(
        env: &TestEnv,
        post_id: PostId,
        prior: &PostRecord,
        prior_audiences: &[AudienceTarget],
        prior_media: &[(MediaRef, MediaReferenceKind, MediaReferenceForm)],
        captured_at: UtcInstant,
    ) -> RevisionId {
        assert_eq!(
            env.base
                .pool()
                .scalar_i64(&format!(
                    "SELECT COUNT(*) FROM post_revisions WHERE post_id = {post_id}"
                ))
                .await
                .expect("count post revisions"),
            1,
            "one meaningful mutation creates exactly one revision"
        );
        let revision = single_revision(env, post_id).await;
        let revision_id = revision.revision_id;
        assert_eq!(revision.post_id, prior.post_id);
        assert_eq!(revision.user_id, prior.user_id);
        assert_eq!(revision.title, prior.title);
        assert_eq!(revision.slug, prior.slug);
        assert_eq!(revision.body, prior.body);
        assert_eq!(revision.format, prior.format);
        assert_eq!(revision.rendered_html, prior.rendered_html);
        assert_eq!(revision.summary, prior.summary);
        assert_eq!(revision.created_at, prior.created_at);
        assert_eq!(revision.updated_at, prior.updated_at);
        assert_eq!(revision.published_at, prior.published_at);
        assert_eq!(revision.deleted_at, prior.deleted_at);
        assert_eq!(revision.captured_at, captured_at);

        let tags = env
            .base
            .pool()
            .string_quintuples(&format!(
                "SELECT tag_slug, tag_display, '', '', ''
                 FROM post_revision_tags WHERE revision_id = {revision_id} ORDER BY tag_slug"
            ))
            .await
            .expect("read revision tags");
        assert_eq!(
            tags.into_iter()
                .map(|(slug, display, _, _, _)| (slug, display))
                .collect::<Vec<_>>(),
            prior
                .tags
                .iter()
                .map(|tag| (tag.tag_slug.to_string(), tag.tag_display.to_string()))
                .collect::<Vec<_>>()
        );

        let audiences = env
            .base
            .pool()
            .string_quintuples(&format!(
                "SELECT target_kind, COALESCE(CAST(audience_id AS TEXT), ''), '', '', ''
                 FROM post_revision_audiences
                 WHERE revision_id = {revision_id} ORDER BY target_kind, audience_id"
            ))
            .await
            .expect("read revision audiences");
        assert_eq!(
            audiences
                .into_iter()
                .map(|(kind, audience_id, _, _, _)| (kind, audience_id))
                .collect::<Vec<_>>(),
            prior_audiences
                .iter()
                .filter_map(audience_target_row)
                .map(|(kind, audience_id)| {
                    (
                        kind.as_ref().to_owned(),
                        audience_id.map_or_else(String::new, |id| i64::from(id).to_string()),
                    )
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(
            media_for_subject(env, post_id, "revision", revision_id).await,
            prior_media,
            "a revision copies the exact current media references rather than extracting anew"
        );
        revision_id
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_adds_removes_and_clears(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &[parse_tag_label("rust"), parse_tag_label("web")],
        )
        .await
        .expect("set initial tags");
        assert_eq!(slugs_of(posts, post).await, vec!["rust", "web"]);

        // Reconcile: "web" drops, "nix" arrives, "rust" stays.
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &[parse_tag_label("rust"), parse_tag_label("nix")],
        )
        .await
        .expect("reconcile tags");
        assert_eq!(slugs_of(posts, post).await, vec!["nix", "rust"]);

        // An empty desired set clears; it is deliberately NOT a no-op, unlike
        // `enqueue_many`'s empty-input early return (#771).
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &[],
        )
        .await
        .expect("clear tags");
        assert!(slugs_of(posts, post).await.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_preserves_existing_display_casing(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &[parse_tag_label("Rust")],
        )
        .await
        .expect("initial casing");
        // Same slug, different casing: the stored row is left untouched, so the
        // original casing survives.
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &[parse_tag_label("rUsT")],
        )
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
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &desired,
        )
        .await
        .expect("first");
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &desired,
        )
        .await
        .expect("second");
        assert_eq!(slugs_of(posts, post).await, vec!["rust", "web"]);

        // `post_tag_diff` does not dedupe its input, so two labels sharing a slug
        // both reach the insert; the conflict-tolerant insert absorbs the second
        // and the first occurrence's casing wins.
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &[parse_tag_label("Nix"), parse_tag_label("nix")],
        )
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
    async fn set_post_tags_requires_an_active_owner(#[case] backend: Backend) {
        let env = backend.setup().await;
        let owner = SeedUser::new().seed(&env.state).await.user_id;
        let other = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(owner).seed(&env.state).await.post_id;

        let missing = set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            PostId::from(999_999),
            owner,
            &[parse_tag_label("rust")],
        )
        .await
        .expect_err("missing post must be rejected");
        assert!(matches!(
            missing,
            crate::WriteScopeError::Operation(TaggingError::PostNotFound)
        ));

        let unauthorized = set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            other,
            &[parse_tag_label("rust")],
        )
        .await
        .expect_err("another owner must be rejected");
        assert!(matches!(
            unauthorized,
            crate::WriteScopeError::Operation(TaggingError::Unauthorized)
        ));

        soft_delete_post_confirmed(&env.state, post, owner).await;
        let deleted = set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            owner,
            &[parse_tag_label("rust")],
        )
        .await
        .expect_err("deleted post must be masked as absent");
        assert!(matches!(
            deleted,
            crate::WriteScopeError::Operation(TaggingError::PostNotFound)
        ));
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

        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &[parse_tag_label("alpha")],
        )
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
        let write_scope = env.state.write_scope.clone();
        let mut racer = tokio::spawn(async move {
            set_post_tags_confirmed(&write_scope, posts, post, user, &[parse_tag_label("gamma")])
                .await
        });

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

    /// An abandoned test lock must have the same rollback and reuse behavior on
    /// both backends: its writes stay uncommitted, and it cannot poison the
    /// pooled connection for the next writer.
    #[apply(backends)]
    #[tokio::test]
    async fn dropping_post_write_lock_rolls_back_and_leaves_writer_usable(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;

        let mut abandoned = env
            .base
            .pool()
            .lock_post_for_write(post)
            .await
            .expect("take post write lock");
        abandoned
            .add_tag(&parse_tag_label("uncommitted"))
            .await
            .expect("write through held lock");
        drop(abandoned);

        assert_eq!(
            slugs_of(&*env.state.posts, post).await,
            Vec::<String>::new()
        );

        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &[parse_tag_label("committed")],
        )
        .await
        .expect("subsequent writer succeeds");
        assert_eq!(slugs_of(&*env.state.posts, post).await, vec!["committed"]);
    }

    /// A tag mutation captures a complete revision, including current media. On
    /// `PostgreSQL` copy must wait for the ordinary media lock rather than
    /// racing a guarded delete or reclaim (ADR-0154).
    // guard:low-level-db — exercises a held PostgreSQL advisory lock directly
    #[tokio::test]
    async fn postgres_tag_revision_capture_waits_for_current_media_lock() {
        let env = Backend::Postgres.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let media = media_ref_for("tag-revision-lock.jpg");
        let post = create_post_via_service(
            &env.state,
            user,
            parse_post_body(&format!(
                "<img src=\"{}\">",
                media_url_for("tag-revision-lock.jpg")
            )),
        )
        .await;
        let held = env
            .base
            .pool()
            .lock_media_reference_for_write(&media)
            .await
            .expect("take the current media lock");
        let posts = Arc::clone(&env.state.posts);
        let write_scope = env.state.write_scope.clone();
        let mut tag_update = tokio::spawn(async move {
            set_post_tags_confirmed(
                &write_scope,
                posts,
                post,
                user,
                &[parse_tag_label("locked")],
            )
            .await
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut tag_update)
                .await
                .is_err(),
            "tag revision capture completed while its current media lock was held"
        );

        held.rollback()
            .await
            .expect("release the current media lock");
        tag_update
            .await
            .expect("tag update task panicked")
            .expect("tag update failed after lock release");
        assert_eq!(
            env.base
                .pool()
                .scalar_i64(&format!(
                    "SELECT COUNT(*) FROM post_revisions WHERE post_id = {post}"
                ))
                .await
                .expect("count captured revisions"),
            1,
            "the deferred tag mutation captures exactly one prior-state revision"
        );
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

        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            first,
            user,
            &[parse_tag_label("rust")],
        )
        .await
        .expect("first post takes the insert path");
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            second,
            user,
            &[parse_tag_label("rust")],
        )
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

        let desired = [parse_tag_label("rust"), parse_tag_label("web")];
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &desired,
        )
        .await
        .expect("seed tags");

        // Decoy: seeded second, so on SQLite its post_tags rows occupy HIGHER
        // rowids. Without it, `max(rowid)+1` would hand the target's rows their
        // original rowids back after a delete-and-reinsert and this test would
        // pass against the very implementation it exists to reject.
        let decoy = SeedRawPost::new(user).seed(&env.state).await.post_id;
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            decoy,
            user,
            &[parse_tag_label("decoy-a"), parse_tag_label("decoy-b")],
        )
        .await
        .expect("seed decoy");

        let before = physical_row_ids(&env, post).await;
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post,
            user,
            &desired,
        )
        .await
        .expect("re-apply the identical set");
        let after = physical_row_ids(&env, post).await;

        assert_eq!(
            before, after,
            "rows were rewritten; set_post_tags must leave unchanged tags physically untouched"
        );
    }
    #[apply(backends)]
    #[tokio::test]
    async fn update_post_semantic_no_op_keeps_timestamp_and_revision_count(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(owner)
            .draft()
            .seed(&env.state)
            .await
            .post_id;
        let input = UpdateRawPost::new("semantic-no-op").build();

        let first = update_post_confirmed(&env.state, post, owner, input.clone()).await;
        let revision_count = env
            .base
            .pool()
            .scalar_i64("SELECT COUNT(*) FROM post_revisions")
            .await
            .expect("count revisions");

        let unchanged = update_post_confirmed(&env.state, post, owner, input).await;

        assert_eq!(unchanged.updated_at, first.updated_at);
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM post_revisions")
                .await
                .expect("count revisions after no-op"),
            revision_count,
            "a canonical full-state no-op must not create a revision"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn update_post_archives_complete_prior_state_in_single_revision(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = SeedUser::new().seed(&env.state).await.user_id;
        let old_media = seed_media(&env.state, owner, "revision-prior.jpg").await;
        let new_media = seed_media(&env.state, owner, "revision-current.jpg").await;
        let prior_body = parse_post_body(&format!(
            "<img src=\"{}\">",
            media_url_for("revision-prior.jpg")
        ));
        let mut seed = SeedRawPost::new(owner)
            .draft()
            .slug("revision-prior-slug")
            .body(prior_body)
            .format(PostFormat::Html)
            .summary(parse_post_summary("prior summary"))
            .audiences(vec![AudienceTarget::Subscribers])
            .tags(["PriorTag", "AnotherTag"])
            .build();
        seed.title = Some(parse_post_title("Prior title"));
        let post_id = create_post_confirmed(&env.state, seed).await.post_id;
        let prior = env
            .state
            .posts
            .get_post_by_id(post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .expect("read prior post")
            .expect("prior post exists");
        let prior_audiences = env
            .state
            .posts
            .get_post_audiences(post_id)
            .await
            .expect("read prior audiences");
        let prior_media = media_for_subject(&env, post_id, "current", RevisionId::from(0)).await;
        assert_eq!(
            prior_media,
            vec![(
                old_media.clone(),
                MediaReferenceKind::Local,
                media_url_for("revision-prior.jpg")
                    .parse()
                    .expect("valid prior media form"),
            )],
            "the prior current row retains the exact parsed reference form"
        );
        let capture_clock = parse_utc_instant("2026-08-27T12:00:00Z");

        let updated = update_post_confirmed(
            &env.state,
            post_id,
            owner,
            UpdateRawPost::new("revision-current-slug")
                .title("Current title")
                .body(parse_post_body(&format!(
                    "<p><img src=\"{}\"></p>",
                    media_url_for("revision-current.jpg")
                )))
                .format(PostFormat::Markdown)
                .summary(parse_post_summary("current summary"))
                .audiences(vec![AudienceTarget::Public])
                .tags(["CurrentTag"])
                .request_clock(capture_clock)
                .build(),
        )
        .await;

        let revision_id = assert_complete_prior_revision(
            &env,
            post_id,
            &prior,
            &prior_audiences,
            &prior_media,
            capture_clock,
        )
        .await;
        assert_eq!(
            media_for_subject(&env, post_id, "current", RevisionId::from(0)).await,
            vec![(
                new_media,
                MediaReferenceKind::Local,
                media_url_for("revision-current.jpg")
                    .parse()
                    .expect("valid current media form"),
            )],
            "the current subject is replaced while the revision retains the prior form"
        );
        assert_eq!(
            env.base
                .pool()
                .scalar_i64(&format!(
                    "SELECT COUNT(*) FROM post_media WHERE post_id = {post_id}
                 AND subject_kind = 'revision' AND revision_id = {revision_id}"
                ))
                .await
                .expect("count revision media"),
            1,
            "the copied row carries the revision subject key"
        );
        assert_eq!(updated.title, Some(parse_post_title("Current title")));
        assert_eq!(old_media, media_ref_for("revision-prior.jpg"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn update_post_with_only_tags_changed_archives_complete_prior_state(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = SeedUser::new().seed(&env.state).await.user_id;
        seed_media(&env.state, owner, "tag-only.jpg").await;
        let body = parse_post_body(&format!("<img src=\"{}\">", media_url_for("tag-only.jpg")));
        let post_id = SeedRawPost::new(owner)
            .draft()
            .slug("tag-only")
            .body(body.clone())
            .audiences(vec![AudienceTarget::Subscribers])
            .tags(["OldTag"])
            .seed(&env.state)
            .await
            .post_id;
        let prior = env
            .state
            .posts
            .get_post_by_id(post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .unwrap()
            .unwrap();
        let audiences = env.state.posts.get_post_audiences(post_id).await.unwrap();
        let media = media_for_subject(&env, post_id, "current", RevisionId::from(0)).await;
        let clock = parse_utc_instant("2026-08-27T12:01:00Z");

        update_post_confirmed(
            &env.state,
            post_id,
            owner,
            UpdateRawPost::new("tag-only")
                .title(prior.title.as_ref().unwrap().as_ref())
                .body(body)
                .format(prior.format)
                .summary(prior.summary.clone())
                .audiences(audiences.clone())
                .tags(["NewTag"])
                .request_clock(clock)
                .build(),
        )
        .await;

        assert_complete_prior_revision(&env, post_id, &prior, &audiences, &media, clock).await;
        assert_eq!(
            owner_slugs_of(&*env.state.posts, post_id, owner).await,
            vec!["newtag"]
        );
        assert_eq!(
            env.state.posts.get_post_audiences(post_id).await.unwrap(),
            audiences
        );
        assert_eq!(
            media_for_subject(&env, post_id, "current", RevisionId::from(0)).await,
            media
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn update_post_with_only_audiences_changed_archives_complete_prior_state(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = SeedUser::new().seed(&env.state).await.user_id;
        seed_media(&env.state, owner, "audience-only.jpg").await;
        let body = parse_post_body(&format!(
            "<img src=\"{}\">",
            media_url_for("audience-only.jpg")
        ));
        let post_id = SeedRawPost::new(owner)
            .draft()
            .slug("audience-only")
            .body(body.clone())
            .audiences(vec![AudienceTarget::Subscribers])
            .tags(["KeptTag"])
            .seed(&env.state)
            .await
            .post_id;
        let prior = env
            .state
            .posts
            .get_post_by_id(post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .unwrap()
            .unwrap();
        let audiences = env.state.posts.get_post_audiences(post_id).await.unwrap();
        let media = media_for_subject(&env, post_id, "current", RevisionId::from(0)).await;
        let clock = parse_utc_instant("2026-08-27T12:02:00Z");

        update_post_confirmed(
            &env.state,
            post_id,
            owner,
            UpdateRawPost::new("audience-only")
                .title(prior.title.as_ref().unwrap().as_ref())
                .body(body)
                .format(prior.format)
                .summary(prior.summary.clone())
                .audiences(vec![AudienceTarget::Public])
                .tags(["KeptTag"])
                .request_clock(clock)
                .build(),
        )
        .await;

        assert_complete_prior_revision(&env, post_id, &prior, &audiences, &media, clock).await;
        assert_eq!(
            owner_slugs_of(&*env.state.posts, post_id, owner).await,
            vec!["kepttag"]
        );
        assert_eq!(
            env.state.posts.get_post_audiences(post_id).await.unwrap(),
            vec![AudienceTarget::Public]
        );
        assert_eq!(
            media_for_subject(&env, post_id, "current", RevisionId::from(0)).await,
            media
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn update_post_with_only_media_changed_archives_complete_prior_state(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = SeedUser::new().seed(&env.state).await.user_id;
        seed_media(&env.state, owner, "media-only-prior.jpg").await;
        let current_media = seed_media(&env.state, owner, "media-only-current.jpg").await;
        let prior_body = parse_post_body(&format!(
            "<img src=\"{}\">",
            media_url_for("media-only-prior.jpg")
        ));
        let post_id = SeedRawPost::new(owner)
            .draft()
            .slug("media-only")
            .body(prior_body)
            .audiences(vec![AudienceTarget::Subscribers])
            .tags(["KeptTag"])
            .seed(&env.state)
            .await
            .post_id;
        let prior = env
            .state
            .posts
            .get_post_by_id(post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .unwrap()
            .unwrap();
        let audiences = env.state.posts.get_post_audiences(post_id).await.unwrap();
        let media = media_for_subject(&env, post_id, "current", RevisionId::from(0)).await;
        let clock = parse_utc_instant("2026-08-27T12:03:00Z");

        update_post_confirmed(
            &env.state,
            post_id,
            owner,
            UpdateRawPost::new("media-only")
                .title(prior.title.as_ref().unwrap().as_ref())
                .body(parse_post_body(&format!(
                    "<img src=\"{}\">",
                    media_url_for("media-only-current.jpg")
                )))
                .format(prior.format)
                .summary(prior.summary.clone())
                .audiences(audiences.clone())
                .tags(["KeptTag"])
                .request_clock(clock)
                .build(),
        )
        .await;

        assert_complete_prior_revision(&env, post_id, &prior, &audiences, &media, clock).await;
        assert_eq!(
            owner_slugs_of(&*env.state.posts, post_id, owner).await,
            vec!["kepttag"]
        );
        assert_eq!(
            env.state.posts.get_post_audiences(post_id).await.unwrap(),
            audiences
        );
        assert_eq!(
            media_for_subject(&env, post_id, "current", RevisionId::from(0)).await,
            vec![(
                current_media,
                MediaReferenceKind::Local,
                media_url_for("media-only-current.jpg")
                    .parse()
                    .expect("valid current media form"),
            )]
        );
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
        SeedRawPost::new(author)
            .published_at(UtcInstant::from(now))
            .seed(state)
            .await;

        // Two stale cached feeds, both older than the post above, so both would need
        // catch-up if they were readable.
        let stale = now - chrono::Duration::hours(1);
        for url in ["/feed.rss", "/feed.atom"] {
            let feed_path = fp(url);
            let (_, format) = feed_path.parts().expect("valid feed path");
            let row = FeedCacheRow::new(
                feed_path,
                SyndicationFeedRepresentation::try_from_stored(
                    format,
                    format.content_type(),
                    "<rss/>".into(),
                )
                .expect("matching stored representation metadata"),
                parse_etag("\"sha256-deadbeef\""),
                UtcInstant::from(stale),
                UtcInstant::from(stale),
            )
            .expect("matching cache row formats");
            let write_scope = state.write_scope.clone();
            let feed_cache = Arc::clone(&state.feed_cache);
            let outcome = write_scope
                .run(move |transaction| {
                    Box::pin(async move { feed_cache.upsert(transaction, row).await })
                })
                .await
                .expect("seed cached feed");
            assert!(matches!(
                outcome,
                common::mutation::MutationOutcome::Confirmed(())
            ));
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
            state.posts.feed_urls_needing_catchup(UtcInstant::from(now)),
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
    async fn lifecycle_decode_failure_rolls_back_revision_and_state(#[case] backend: Backend) {
        let env = backend.setup().await;
        let owner = SeedUser::new().seed(&env.state).await.user_id;
        let post_id = SeedRawPost::new(owner)
            .draft()
            .seed(&env.state)
            .await
            .post_id;
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post_id,
            owner,
            &[parse_tag_label("Rust")],
        )
        .await
        .expect("seed tag");
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "UPDATE tags SET tag_slug = $1
                 WHERE tag_id = (SELECT tag_id FROM post_tags WHERE post_id = $2)",
            )
            .bind_storage(CorruptTagSlug("not a slug".to_owned()))
            .bind_storage(post_id)
            .execute(pool)
            .await
            .expect("corrupt tag slug");
        });

        let error = publish_post_scoped(&env.state, post_id, owner)
            .await
            .expect_err("malformed aggregate must reject publication");
        assert!(
            matches!(
                &error,
                crate::WriteScopeError::Operation(UpdatePostError::Internal(
                    sqlx::Error::ColumnDecode { index, .. }
                )) if index == "\"tags\""
            ),
            "{error:?}"
        );
        assert_eq!(
            env.base
                .pool()
                .scalar_i64(&format!(
                    "SELECT COUNT(*) FROM posts
                     WHERE post_id = {post_id} AND published_at IS NULL"
                ))
                .await
                .expect("read publication state"),
            1
        );
        assert_eq!(
            env.base
                .pool()
                .scalar_i64(&format!(
                    "SELECT COUNT(*) FROM post_revisions WHERE post_id = {post_id}"
                ))
                .await
                .expect("count rolled-back revisions"),
            1
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
            rendered_html: common::test_support::rendered_html(
                "<p>The first non-empty line of the body is here.</p>",
            ),
            created_at: UtcInstant::now(),
            updated_at: UtcInstant::now(),
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
            rendered_html: common::test_support::rendered_html("<p>My body</p>"),
            created_at: UtcInstant::from(Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap()),
            updated_at: UtcInstant::from(Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap()),
            published_at: Some(UtcInstant::from(
                Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            )),
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
            rendered_html: common::test_support::rendered_html("<p>My body</p>"),
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
        let changed = update_post_confirmed(
            &env.state,
            post_id,
            user_id,
            update(Some(parse_post_summary("edited summary"))),
        )
        .await;
        assert_eq!(changed.summary, Some(parse_post_summary("edited summary")));

        // `None` clears it.
        let cleared = update_post_confirmed(&env.state, post_id, user_id, update(None)).await;
        assert_eq!(cleared.summary, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publish_post_captures_complete_prior_state(#[case] backend: Backend) {
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
        let revisions_before = env
            .base
            .pool()
            .scalar_i64("SELECT COUNT(*) FROM post_revisions")
            .await
            .unwrap();

        let after = publish_post_confirmed(&env.state, seeded.post_id, user.user_id).await;

        assert!(after.published_at.is_some());
        assert_eq!(after.title, before.title);
        assert_eq!(after.slug, before.slug);
        assert_eq!(after.body, before.body);
        assert_eq!(after.format, before.format);
        assert_eq!(after.rendered_html, before.rendered_html);
        assert_eq!(after.summary, before.summary);
        assert_eq!(after.created_at, before.created_at);
        assert_eq!(
            posts.get_post_audiences(seeded.post_id).await.unwrap(),
            audiences_before
        );
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM post_revisions")
                .await
                .unwrap(),
            revisions_before + 1
        );
        let revision = single_revision(&env, seeded.post_id).await;
        assert_eq!(revision.title, before.title);
        assert_eq!(revision.slug, before.slug);
        assert_eq!(revision.body, before.body);
        assert_eq!(revision.format, before.format);
        assert_eq!(revision.rendered_html, before.rendered_html);
        assert_eq!(revision.summary, before.summary);
        assert_eq!(revision.created_at, before.created_at);
        assert_eq!(revision.updated_at, before.updated_at);
        assert_eq!(revision.published_at, before.published_at);
        assert_eq!(revision.deleted_at, before.deleted_at);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publish_post_keeps_an_already_published_timestamp(#[case] backend: Backend) {
        // COALESCE, not overwrite: the permalink is derived from `published_at`, so
        // re-publishing must not restamp it.
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let post_id = SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await
            .post_id;

        let first = publish_post_confirmed(&env.state, post_id, user_id).await;
        let second = publish_post_confirmed(&env.state, post_id, user_id).await;

        assert!(first.published_at.is_some());
        assert_eq!(first.published_at, second.published_at);
        assert_eq!(first.updated_at, second.updated_at);
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM post_revisions")
                .await
                .unwrap(),
            1,
            "repeated publish is a semantic no-op"
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
            publish_post_scoped(&env.state, PostId::from(999_999), owner).await,
            Err(crate::WriteScopeError::Operation(UpdatePostError::NotFound))
        ));
        assert!(matches!(
            publish_post_scoped(&env.state, post_id, stranger).await,
            Err(crate::WriteScopeError::Operation(
                UpdatePostError::Unauthorized
            ))
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

        soft_delete_post_confirmed(&env.state, post_id, owner).await;
        assert!(matches!(
            publish_post_scoped(&env.state, post_id, owner).await,
            Err(crate::WriteScopeError::Operation(UpdatePostError::NotFound))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn unpublish_post_captures_complete_prior_state(#[case] backend: Backend) {
        // Unpublish changes lifecycle state while preserving current content and
        // recording that complete prior state.
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let posts = &*env.state.posts;
        let seeded = SeedRawPost::new(user.user_id)
            .slug("unpublish-complete-row")
            .summary(parse_post_summary("the summary"))
            .tags(["Zed", "alpha", "beta"])
            .seed(&env.state)
            .await;
        let before = posts
            .get_post_by_id(seeded.post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        let revisions_before = env
            .base
            .pool()
            .scalar_i64("SELECT COUNT(*) FROM post_revisions")
            .await
            .unwrap();

        let updated = unpublish_post_confirmed(&env.state, seeded.post_id, user.user_id).await;

        assert_eq!(updated.post_id, before.post_id);
        assert_eq!(updated.user_id, before.user_id);
        assert_eq!(updated.author_username, user.username);
        assert_eq!(updated.title, before.title);
        assert_eq!(updated.slug, before.slug);
        assert_eq!(updated.body, before.body);
        assert_eq!(updated.format, before.format);
        assert_eq!(updated.rendered_html, before.rendered_html);
        assert_eq!(updated.created_at, before.created_at);
        assert!(updated.updated_at >= before.updated_at);
        assert_eq!(updated.deleted_at, before.deleted_at);
        assert_eq!(updated.summary, before.summary);
        assert!(
            updated.published_at.is_none(),
            "the returned row is a draft"
        );
        assert_eq!(
            updated
                .tags
                .iter()
                .map(|tag| tag.tag_slug.as_ref())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta", "zed"],
            "RETURNING preserves the canonical slug order"
        );
        for (returned, original) in updated.tags.iter().zip(&before.tags) {
            assert_eq!(returned.post_id, original.post_id);
            assert_eq!(returned.tag_id, original.tag_id);
            assert_eq!(returned.tag_slug, original.tag_slug);
            assert_eq!(returned.tag_display, original.tag_display);
        }
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM post_revisions")
                .await
                .unwrap(),
            revisions_before + 1
        );
        let revision = single_revision(&env, seeded.post_id).await;
        assert_eq!(revision.title, before.title);
        assert_eq!(revision.slug, before.slug);
        assert_eq!(revision.body, before.body);
        assert_eq!(revision.format, before.format);
        assert_eq!(revision.rendered_html, before.rendered_html);
        assert_eq!(revision.summary, before.summary);
        assert_eq!(revision.created_at, before.created_at);
        assert_eq!(revision.updated_at, before.updated_at);
        assert_eq!(revision.published_at, before.published_at);
        assert_eq!(revision.deleted_at, before.deleted_at);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn soft_delete_captures_prior_state_and_rejects_repeats(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [owner, stranger] = seed_users::<2>(&env.state).await;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(owner)
            .draft()
            .summary(parse_post_summary("the summary"))
            .tags(["Rust"])
            .seed(&env.state)
            .await
            .post_id;
        let before = posts
            .get_post_by_id(post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .unwrap()
            .unwrap();
        let revisions_before = env
            .base
            .pool()
            .scalar_i64("SELECT COUNT(*) FROM post_revisions")
            .await
            .unwrap();

        assert!(matches!(
            soft_delete_post_scoped(&env.state, PostId::from(999_999), owner).await,
            Err(crate::WriteScopeError::Operation(UpdatePostError::NotFound))
        ));
        assert!(matches!(
            soft_delete_post_scoped(&env.state, post_id, stranger).await,
            Err(crate::WriteScopeError::Operation(
                UpdatePostError::Unauthorized
            ))
        ));
        soft_delete_post_confirmed(&env.state, post_id, owner).await;
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM post_revisions")
                .await
                .unwrap(),
            revisions_before + 1
        );
        let revision = single_revision(&env, post_id).await;
        assert_eq!(revision.title, before.title);
        assert_eq!(revision.slug, before.slug);
        assert_eq!(revision.body, before.body);
        assert_eq!(revision.format, before.format);
        assert_eq!(revision.rendered_html, before.rendered_html);
        assert_eq!(revision.summary, before.summary);
        assert_eq!(revision.created_at, before.created_at);
        assert_eq!(revision.updated_at, before.updated_at);
        assert_eq!(revision.published_at, before.published_at);
        assert_eq!(revision.deleted_at, before.deleted_at);
        assert!(matches!(
            soft_delete_post_scoped(&env.state, post_id, owner).await,
            Err(crate::WriteScopeError::Operation(UpdatePostError::NotFound))
        ));
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM post_revisions")
                .await
                .unwrap(),
            revisions_before + 1,
            "repeated soft-delete is a no-op"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn unpublish_draft_is_a_semantic_no_op(#[case] backend: Backend) {
        let env = backend.setup().await;
        let owner = SeedUser::new().seed(&env.state).await.user_id;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(owner)
            .draft()
            .seed(&env.state)
            .await
            .post_id;
        let before = posts
            .get_post_by_id(post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .unwrap()
            .unwrap();
        let after = unpublish_post_confirmed(&env.state, post_id, owner).await;
        assert_eq!(after.published_at, None);
        assert_eq!(after.updated_at, before.updated_at);
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM post_revisions")
                .await
                .unwrap(),
            0
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn unpublish_post_rejects_missing_foreign_and_deleted_rows_without_writing(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let [owner, stranger] = seed_users::<2>(&env.state).await;
        let posts = &*env.state.posts;
        let post_id = SeedRawPost::new(owner)
            .slug("guarded-unpublish")
            .seed(&env.state)
            .await
            .post_id;

        assert!(matches!(
            unpublish_post_scoped(&env.state, PostId::from(999_999), owner).await,
            Err(crate::WriteScopeError::Operation(UpdatePostError::NotFound))
        ));
        assert!(matches!(
            unpublish_post_scoped(&env.state, post_id, stranger).await,
            Err(crate::WriteScopeError::Operation(
                UpdatePostError::Unauthorized
            ))
        ));
        assert!(
            posts
                .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
                .await
                .unwrap()
                .unwrap()
                .published_at
                .is_some(),
            "the foreign rejection must not clear publication"
        );

        soft_delete_post_confirmed(&env.state, post_id, owner).await;
        assert!(matches!(
            unpublish_post_scoped(&env.state, post_id, owner).await,
            Err(crate::WriteScopeError::Operation(UpdatePostError::NotFound))
        ));
        let publication_rows = format!(
            "SELECT COUNT(*) FROM posts WHERE post_id = {} AND published_at IS NOT NULL",
            i64::from(post_id)
        );
        assert_eq!(
            env.base.pool().scalar_i64(&publication_rows).await.unwrap(),
            1,
            "the deleted rejection must not clear publication"
        );
    }
    #[apply(backends)]
    #[tokio::test]
    async fn publish_post_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = publish_post_scoped(&env.state, PostId::from(1), UserId::from(1)).await;
        assert!(matches!(result, Err(crate::WriteScopeError::Begin(_))));
    }

    // -----------------------------------------------------------------------
    // post_media: what a post's rendered HTML points a reader at (#711)
    // -----------------------------------------------------------------------

    #[apply(backends)]
    #[tokio::test]
    async fn post_media_create_post_writes_its_media_rows(#[case] backend: Backend) {
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
            vec![(
                media_ref_for("photo.jpg"),
                MediaReferenceKind::Local,
                media_url_for("photo.jpg")
                    .parse()
                    .expect("valid media reference form"),
            )]
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
            .map(|(media, _, _)| media.filename.to_string())
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
            (
                media_ref_for("b.jpg"),
                MediaReferenceKind::Local,
                media_url_for("b.jpg")
                    .parse()
                    .expect("valid media reference form"),
            ),
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

        publish_post_confirmed(&env.state, post_id, user).await;

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
        let evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));

        let first = create_post_via_service(&env.state, owner, parse_post_body(&embed)).await;
        let second = create_post_via_service(&env.state, owner, parse_post_body(&embed)).await;
        let deleted = create_post_via_service(&env.state, owner, parse_post_body(&embed)).await;
        let foreign = create_post_via_service(&env.state, stranger, parse_post_body(&embed)).await;
        let unrelated =
            create_post_via_service(&env.state, owner, parse_post_body("no media")).await;
        soft_delete_post_confirmed(&env.state, deleted, owner).await;

        let found = env
            .state
            .posts
            .list_posts_referencing_media(
                owner,
                &media_ref_for("photo.jpg"),
                env.base.instance_id(),
                &evidence,
            )
            .await
            .expect("listing succeeds");

        assert_eq!(
            found,
            vec![first, second, deleted],
            "owner current and retained history references are ascending"
        );
        assert!(
            found.contains(&deleted),
            "a Deleted Post's retained current/revision state remains protected"
        );
        assert!(
            !found.contains(&foreign),
            "another user's post is not reported (spec D9)"
        );
        assert!(!found.contains(&unrelated));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_posts_referencing_media_reports_a_post_once_for_local_and_foreign_spellings(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
        let [user] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, user, "mixed-origin.jpg").await;
        let media_url = media_url_for("mixed-origin.jpg");
        let post_id = create_post_via_service(
            &env.state,
            user,
            parse_post_body(&format!(
                "<img src=\"{media_url}\"><img src=\"https://foreign.example{media_url}\">"
            )),
        )
        .await;

        assert_eq!(
            fetch_post_media(&env.base, post_id).await.len(),
            2,
            "both persisted URL spellings retain their distinct exact forms"
        );
        assert_eq!(
            env.state
                .posts
                .list_posts_referencing_media(user, &media, env.base.instance_id(), &evidence)
                .await
                .expect("listing succeeds"),
            vec![post_id],
            "one post ID is reported even though the Post has multiple persisted spellings"
        );
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
        let evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
        let [user] = seed_users::<1>(&env.state).await;
        let body = parse_post_body(&format!("<img src=\"{}\">", media_url_for("needle.jpg")));

        // One batched transaction, not 1201 round trips. `create_posts` shares
        // `write_post_in_tx` with `create_post`, so each row's `post_media` is written
        // too, and the ids come back in input order.
        let inputs: Vec<CreatePostInput> = (0..1201)
            .map(|_| SeedRawPost::new(user).body(body.clone()).build())
            .collect();
        let ids = create_posts_confirmed(&env.state, inputs).await;

        let found = env
            .state
            .posts
            .list_posts_referencing_media(
                user,
                &media_ref_for("needle.jpg"),
                env.base.instance_id(),
                &evidence,
            )
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
    async fn list_media_references_returns_a_bounded_snapshot_with_a_sentinel(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let media = media_ref_for("bounded-snapshot.jpg");
        let body = parse_post_body(&format!(
            "<img src=\"{}\">",
            media_url_for("bounded-snapshot.jpg")
        ));
        let inputs: Vec<CreatePostInput> = (0..=media::MAX_MEDIA_REFERENCE_SNAPSHOT)
            .map(|_| SeedRawPost::new(user).body(body.clone()).build())
            .collect();
        create_posts_confirmed(&env.state, inputs).await;

        let snapshot = env.state.posts.list_media_references(&media).await.unwrap();

        assert_eq!(
            snapshot.references().len(),
            media::MAX_MEDIA_REFERENCE_SNAPSHOT
        );
        assert!(
            snapshot.has_unexamined_references(),
            "the extra row is a fail-closed sentinel, not an unbounded allocation"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_posts_referencing_media_returns_empty_for_unreferenced_media(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
        create_post_via_service(&env.state, user, parse_post_body("no media")).await;

        let found = env
            .state
            .posts
            .list_posts_referencing_media(
                user,
                &media_ref_for("absent.jpg"),
                env.base.instance_id(),
                &evidence,
            )
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
                UtcInstant::now(),
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

        let result = set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post_id,
            uid,
            &[parse_tag_label("rust")],
        )
        .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Operation(TaggingError::Internal(_)))
        ));
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
                builder.published_at(UtcInstant::from(now - chrono::Duration::minutes(30)))
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

        for error in [
            UpdatePostError::BookkeepingMismatch,
            UpdatePostError::StaleContent,
        ] {
            let expected_operator_message = error.to_string();
            let internal: InternalError = error.into();
            assert_eq!(internal.kind(), ErrorKind::Validation);
            assert_eq!(internal.public_message(), expected_operator_message);
            assert_eq!(internal.operator_message(), expected_operator_message);
        }
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
    fn post_cursor_round_trips_through_wire_cursor() {
        let cursor = PostCursor {
            created_at: "2026-04-12T08:30:00.123456Z".parse().unwrap(),
            post_id: PostId::from(42),
        };

        assert_eq!(
            keyset_cursor(Some(wire_cursor(&cursor)))
                .unwrap()
                .created_at,
            cursor.created_at
        );
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
        let date = PermalinkDate::from(record.created_at.value().date_naive());

        // A published, public post is visible to an anonymous viewer at its permalink.
        let found = fetch_post_record(
            posts,
            &ViewerIdentity::Anonymous,
            &record.author_username,
            date,
            &record.slug,
            UtcInstant::now(),
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
            UtcInstant::now(),
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
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post_id,
            user_id,
            &[parse_tag_label("Rust")],
        )
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
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post_id,
            user_id,
            &[parse_tag_label("Rust")],
        )
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
        let untitled_id = create_post_confirmed(
            &env.state,
            CreatePostInput {
                user_id,
                title: None,
                slug: parse_slug("no-title"),
                body: untitled_body.clone(),
                format: PostFormat::Markdown,
                rendered: host::render::with_media(&untitled_body, &PostFormat::Markdown),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                expectations: PostBookkeepingExpectation::default(),
                idempotency_key: None,
            },
        )
        .await
        .post_id;
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
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post_id,
            user_id,
            &[parse_tag_label("Rust")],
        )
        .await
        .expect("seed tag");

        // The tags aggregate reads `tag_slug` from `tags`. Land a value the `Tag`
        // serde bridge rejects with a column-specific corruption role, because a
        // typed domain bind cannot create it.
        let sql = "UPDATE tags SET tag_slug = $1
                   WHERE tag_id = (SELECT tag_id FROM post_tags WHERE post_id = $2)";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind_storage(CorruptTagSlug("not a slug".to_owned()))
                .bind_storage(post_id)
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
            matches!(&err, sqlx::Error::ColumnDecode { index, .. } if index == "\"tags\""),
            "expected a tags column-decode error, got: {err:?}"
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
        // is not a valid slug character), binding it through the column-specific
        // corruption role so the bad value actually lands in the column.
        let sql = "UPDATE posts SET slug = $1 WHERE post_id = $2";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind_storage(CorruptPostSlug("not a slug".to_owned()))
                .bind_storage(post_id)
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

        // Land a bogus token in `format` via its column-specific corruption role,
        // then assert the read fails at column-decode — the bridge's `Decode` error
        // arm (`parse()` → `InvalidPostFormat`).
        let sql = "UPDATE posts SET format = $1 WHERE post_id = $2";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind_storage(CorruptPostFormat("bogus".to_owned()))
                .bind_storage(post_id)
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
            .published_at(UtcInstant::from(scheduled_at))
            .seed(&env.state)
            .await;
        let live_at_boundary = SeedRawPost::new(author)
            .published_at(UtcInstant::from(now))
            .seed(&env.state)
            .await;
        let deleted = SeedRawPost::new(author)
            .published_at(UtcInstant::from(scheduled_at))
            .seed(&env.state)
            .await;
        soft_delete_post_confirmed(&env.state, deleted.post_id, author).await;

        let draft_record = posts
            .get_post_by_id(draft.post_id, &ViewerIdentity::Local { user_id: author })
            .await
            .unwrap()
            .expect("author can read seeded draft");
        let draft_date = PermalinkDate::from(draft_record.created_at.value().date_naive());
        let scheduled_date = PermalinkDate::from(scheduled_at.date_naive());

        let found_draft = posts
            .get_unpublished_post_by_permalink(
                author,
                draft_date,
                &draft.slug,
                UtcInstant::from(now),
            )
            .await
            .unwrap();
        assert_eq!(found_draft.map(|post| post.post_id), Some(draft.post_id));

        let found_scheduled = posts
            .get_unpublished_post_by_permalink(
                author,
                scheduled_date,
                &scheduled.slug,
                UtcInstant::from(now),
            )
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
                    .get_unpublished_post_by_permalink(user_id, date, slug, UtcInstant::from(now))
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }
}
