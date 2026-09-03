//! Content storage for posts, revisions, and tagging.

pub(crate) mod cursors;
pub(crate) mod errors;
pub(crate) mod lifecycle;
pub(crate) mod media;
pub(crate) mod models;
pub(crate) mod store;
pub(crate) mod syndication;
pub(crate) mod tags;
pub(crate) mod visibility;

pub use cursors::{
    CollectionCursor, PostCursor, PostRevisionCursor, ScheduledPostCursor, keyset_cursor,
    scheduled_keyset_cursor, to_post_cursor, to_scheduled_post_cursor, wire_cursor,
    wire_scheduled_cursor,
};
pub use errors::{CreatePostError, ListByTagError, TaggingError, UpdatePostError};
pub use media::{
    MAX_MEDIA_REFERENCE_SNAPSHOT, MediaReferenceEvidence, MediaReferenceSnapshot,
    PersistedMediaReference, PersistedMediaSubject, PostMediaReferenceBackfill,
    ProvenForeignReference,
};
pub use models::PermalinkDate;
pub use models::{
    CreatePostInput, CreatedPost, CurrentPostRevisionSummary, InvalidPostFormat,
    PostBookkeepingExpectation, PostFormat, PostLifecycle, PostMutation, PostRecord,
    PostRevisionDetail, PostRevisionMetadata, PostRevisionPage, PostRevisionRecord,
    PostRevisionTag, PublishUpdate, RenderedHtml, UpdatePostInput,
};
#[cfg(any(test, feature = "test-utils"))]
pub use store::MockPostStorage;
pub use store::{PostDialect, PostStorage, PostStore, fetch_post_record, list_by_tag_rows};
pub use syndication::GoLivePost;
pub use tags::{PostTag, TagRecord};
