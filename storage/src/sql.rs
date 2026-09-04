//! Shared SQL-string helpers used by both dialects' assembled (non-placeholder) SQL.

use std::fmt::Display;

use crate::backup::{
    CatalogTableName, RestoreBoolean, RestoreInteger, RestoreJson, RestoreReal, RestoreText,
};
use crate::feed_cache::{
    FeedCacheGeneratedAt, FeedCacheRepresentationModifiedAt, StoredFeedBody,
    StoredFeedSemanticFingerprint,
};
use crate::feed_events::{FeedEventAttempts, StoredFeedDiagnostic};
use crate::helpers::{SerializedPostTags, SessionCreatedAt, SessionLastUsedAt, StoredSessionLabel};
use crate::invites::{InviteCreatedAt, InviteExpiresAt};
use crate::media::MediaDeleteMode;
use crate::posts::{
    lifecycle::IdempotencyAdvisoryLockKey,
    media::{MediaAdvisoryLockKey, MediaReferenceSnapshotLimit, PersistedMediaSubjectKind},
    models::{PermalinkDateText, PostPublicationClear},
    tags::TagSlugPrefixPattern,
};
use crate::publisher::PublisherGeneration;
use crate::subscriptions::SubscriptionStatusName;
use crate::user_config::StoredUserConfigValue;
use crate::users::{EmailVerified, OperatorStatus};
use crate::{InstanceId, StoredSiteConfigValue};
use common::audience::AudienceName;
use common::bio::Bio;
use common::display_name::DisplayName;
use common::email::Email;
use common::etag::ETag;
use common::idempotency_key::IdempotencyKey;
use common::ids::{
    AudienceId, ChannelId, FeedEventId, PostId, RevisionId, SubscriptionId, TagId, UserId,
};
use common::media::{
    ByteSize, ContentHash, ContentType, Filename, MediaReferenceForm, MediaReferenceKind,
    MediaSource,
};
use common::pagination::{PageOffset, PageSize, RowLimit};
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::render::{PostFormat, RenderedHtml};
use common::root_relative_url::RootRelativeUrl;
use common::seed::PageCursor;
use common::session_label::SessionLabel;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::tagged_url::MediaSourceUrl;
use common::time::UtcInstant;
use common::token::TokenHash;
use common::username::Username;
use common::visibility::SubscriberRef;
use common::visibility::{DefaultAudience, TargetKind};
use host::config_key::{SiteConfigKey, UserConfigKey};
use host::feed::{FeedEventClaimLimit, FeedEventPhase, FeedEventStatus, FeedMinItems, FeedPath};
use host::invite::InviteCode;
use host::stored_password_hash::StoredPasswordHash;
use sqlx::{
    Database, Encode, QueryBuilder, Type,
    query::{Query, QueryAs, QueryScalar},
    query_builder::Separated,
};

mod sealed {
    pub trait StorageBind {}
}

/// A value explicitly approved to cross the storage bind boundary.
///
/// This registry is deliberately independent of database capabilities. The
/// extension methods retain sqlx's `Encode` and `Type` bounds, which decide
/// whether an approved value is representable by a particular backend.
pub trait StorageBind: sealed::StorageBind {}

/// Adds typed value admission to sqlx's native query types.
///
/// Import this trait to use [`Self::bind_storage`]. The companion builder trait
/// preserves sqlx's fluent API too:
///
/// ```
/// use common::tag::Tag;
/// use sqlx::{Postgres, QueryBuilder, query_as, query_scalar};
/// use storage::sql::{QueryBuilderStorageExt, RowCount};
/// # use chrono::Utc;
/// # use serde_json::json;
/// # use sqlx::{Sqlite, query};
/// # use storage::sql::QueryStorageExt;
/// # let primitive_query = query::<Sqlite>("SELECT ?");
///
/// let tag: Tag = "rust".parse().unwrap();
/// let count = RowCount::try_from(1).unwrap();
/// let _query = query::<Sqlite>("SELECT ?").bind_storage(&tag);
/// let _query_as = query_as::<Sqlite, (Tag,)>("SELECT ?").bind_storage(count);
/// let _query_scalar = query_scalar::<Sqlite, Tag>("SELECT ?")
///     .bind_storage(Some(RowCount::try_from(1).unwrap()));
///
/// let tags = vec!["rust".parse::<Tag>().unwrap()];
/// let _postgres_array = query::<Postgres>("SELECT $1").bind_storage(&tags);
///
/// let mut builder = QueryBuilder::<Sqlite>::new("SELECT ");
/// builder.push_storage_bind(RowCount::try_from(1).unwrap());
/// builder.separated(", ").push_storage_bind(&tag);
/// ```
///
/// Primitive leaves are not admitted, even through the approval-preserving
/// generic forms:
///
/// ```compile_fail
/// # use sqlx::{Sqlite, query};
/// # use storage::sql::QueryStorageExt;
/// # let primitive_query = query::<Sqlite>("SELECT ?");
///
/// let _ = primitive_query.bind_storage(String::new());
/// ```
///
/// ```compile_fail
/// # use sqlx::{Sqlite, query};
/// # use storage::sql::QueryStorageExt;
/// # let primitive_query = query::<Sqlite>("SELECT ?");
///
/// let _ = primitive_query.bind_storage(1_i64);
/// ```
///
/// ```compile_fail
/// # use sqlx::{Sqlite, query};
/// # use storage::sql::QueryStorageExt;
/// # let primitive_query = query::<Sqlite>("SELECT ?");
///
/// let _ = primitive_query.bind_storage(false);
/// ```
///
/// ```compile_fail
/// # use chrono::Utc;
/// # use sqlx::{Sqlite, query};
/// # use storage::sql::QueryStorageExt;
/// # let primitive_query = query::<Sqlite>("SELECT ?");
///
/// let _ = primitive_query.bind_storage(Utc::now());
/// ```
///
/// ```compile_fail
/// # use serde_json::json;
/// # use sqlx::{Sqlite, query};
/// # use storage::sql::QueryStorageExt;
/// # let primitive_query = query::<Sqlite>("SELECT ?");
///
/// let _ = primitive_query.bind_storage(json!({}));
/// ```
///
/// ```compile_fail
/// # use sqlx::{Sqlite, query};
/// # use storage::sql::QueryStorageExt;
/// # let primitive_query = query::<Sqlite>("SELECT ?");
///
/// let _ = primitive_query.bind_storage(vec![0_u8]);
/// ```
pub trait QueryStorageExt<'q, DB: Database>: Sized {
    /// Binds an approved value, retaining the native query type.
    #[must_use]
    fn bind_storage<T>(self, value: T) -> Self
    where
        T: StorageBind + 'q + Encode<'q, DB> + Type<DB>;
}

/// Adds typed value admission to sqlx's native query builders.
///
/// Import this trait to use [`Self::push_storage_bind`].
pub trait QueryBuilderStorageExt<'args, DB: Database> {
    /// Pushes an approved bind, retaining sqlx's native fluent builder API.
    fn push_storage_bind<T>(&mut self, value: T) -> &mut Self
    where
        T: StorageBind + 'args + Encode<'args, DB> + Type<DB>;
}

macro_rules! approve_storage_binds {
    ($($type:ty),+ $(,)?) => {
        $(
            impl StorageBind for $type {}
            impl sealed::StorageBind for $type {}
        )+
    };
}

approve_storage_binds!(
    AudienceId,
    AudienceName,
    Bio,
    ByteSize,
    ChannelId,
    ContentHash,
    ContentType,
    DefaultAudience,
    DisplayName,
    ETag,
    Email,
    FeedEventId,
    Filename,
    IdempotencyKey,
    IdempotencyAdvisoryLockKey,
    InstanceId,
    MediaAdvisoryLockKey,
    MediaDeleteMode,
    MediaReferenceForm,
    MediaReferenceKind,
    MediaReferenceSnapshotLimit,
    MediaSource,
    MediaSourceUrl,
    PageOffset,
    PageSize,
    PermalinkDateText,
    TagSlugPrefixPattern,
    PersistedMediaSubjectKind,
    PostId,
    PostPublicationClear,
    PostTitle,
    RevisionId,
    RootRelativeUrl,
    PostBody,
    PostFormat,
    Slug,
    SubscriptionId,
    SubscriptionStatusName,
    Tag,
    TagId,
    TagLabel,
    RenderedHtml,
    UtcInstant,
    UserId,
    Username,
    FeedEventClaimLimit,
    FeedEventStatus,
    FeedEventPhase,
    FeedMinItems,
    InviteCode,
    OperatorStatus,
    EmailVerified,
    PostSummary,
    SiteConfigKey,
    StoredPasswordHash,
    SubscriberRef,
    UserConfigKey,
    CatalogTableName,
    RestoreBoolean,
    RestoreInteger,
    RestoreJson,
    PageCursor,
    FeedPath,
    RestoreReal,
    RestoreText,
    RowLimit,
    SessionLabel,
    TargetKind,
    TokenHash,
    FeedCacheGeneratedAt,
    FeedCacheRepresentationModifiedAt,
    FeedEventAttempts,
    InviteCreatedAt,
    InviteExpiresAt,
    SessionCreatedAt,
    SessionLastUsedAt,
    StoredFeedBody,
    StoredFeedSemanticFingerprint,
    StoredFeedDiagnostic,
    PublisherGeneration,
    StoredSessionLabel,
    StoredSiteConfigValue,
    SerializedPostTags,
    StoredUserConfigValue,
);
#[cfg(test)]
use crate::backup::CatalogDatabaseName;
#[cfg(test)]
use crate::email::CorruptEmailAddress;
#[cfg(test)]
use crate::invites::CorruptInviteCode;
#[cfg(test)]
use crate::posts::store::{CorruptPostFormat, CorruptPostSlug, CorruptTagSlug};
#[cfg(test)]
use crate::sessions::CorruptSessionTokenHash;
#[cfg(any(test, feature = "test-support"))]
use crate::test_support::{RawMediaFilename, TemplateDatabaseLockKey, TemplateDatabaseName};
#[cfg(test)]
use crate::users::CorruptUsername;

#[cfg(any(test, feature = "test-support"))]
approve_storage_binds!(
    RawMediaFilename,
    TemplateDatabaseLockKey,
    TemplateDatabaseName
);

#[cfg(test)]
approve_storage_binds!(
    CorruptEmailAddress,
    CorruptInviteCode,
    CorruptPostFormat,
    CorruptPostSlug,
    CorruptSessionTokenHash,
    CorruptTagSlug,
    CorruptUsername,
    CatalogDatabaseName,
);

impl<T> StorageBind for Option<T> where T: StorageBind {}
impl<T> sealed::StorageBind for Option<T> where T: StorageBind {}

impl<T> StorageBind for Vec<T> where T: StorageBind {}
impl<T> sealed::StorageBind for Vec<T> where T: StorageBind {}

impl<T> StorageBind for &T where T: StorageBind + ?Sized {}

impl<T> sealed::StorageBind for &T where T: StorageBind + ?Sized {}

impl<T> StorageBind for [T] where T: StorageBind {}
impl<T> sealed::StorageBind for [T] where T: StorageBind {}
impl StorageBind for RowCount {}
impl sealed::StorageBind for RowCount {}

impl<'q, DB> QueryStorageExt<'q, DB> for Query<'q, DB, DB::Arguments<'q>>
where
    DB: Database,
{
    fn bind_storage<T>(self, value: T) -> Self
    where
        T: StorageBind + 'q + Encode<'q, DB> + Type<DB>,
    {
        self.bind(value)
    }
}

impl<'q, DB, O> QueryStorageExt<'q, DB> for QueryAs<'q, DB, O, DB::Arguments<'q>>
where
    DB: Database,
{
    fn bind_storage<T>(self, value: T) -> Self
    where
        T: StorageBind + 'q + Encode<'q, DB> + Type<DB>,
    {
        self.bind(value)
    }
}

impl<'q, DB, O> QueryStorageExt<'q, DB> for QueryScalar<'q, DB, O, DB::Arguments<'q>>
where
    DB: Database,
{
    fn bind_storage<T>(self, value: T) -> Self
    where
        T: StorageBind + 'q + Encode<'q, DB> + Type<DB>,
    {
        self.bind(value)
    }
}

impl<'args, DB> QueryBuilderStorageExt<'args, DB> for QueryBuilder<'args, DB>
where
    DB: Database,
{
    fn push_storage_bind<T>(&mut self, value: T) -> &mut Self
    where
        T: StorageBind + 'args + Encode<'args, DB> + Type<DB>,
    {
        self.push_bind(value)
    }
}

impl<'qb, 'args, DB, Sep> QueryBuilderStorageExt<'args, DB> for Separated<'qb, 'args, DB, Sep>
where
    'args: 'qb,
    DB: Database,
    Sep: Display,
{
    fn push_storage_bind<T>(&mut self, value: T) -> &mut Self
    where
        T: StorageBind + 'args + Encode<'args, DB> + Type<DB>,
    {
        self.push_bind(value)
    }
}

/// A row cardinality decoded from SQL.
///
/// SQL count expressions are signed on both supported backends; the declaration
/// rejects the corrupt negative values that cannot represent a cardinality.
#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 0,
    error = "row count must be a non-negative integer"
)]
pub struct RowCount(i64);

impl RowCount {
    /// Converts this checked count to the unsigned representation exposed by
    /// count-facing storage APIs.
    #[must_use]
    pub(crate) fn into_u64(self) -> u64 {
        match u64::try_from(self.0) {
            Ok(count) => count,
            Err(_) => unreachable!("RowCount rejects negative values"),
        }
    }
}

/// A boolean fact decoded from an SQL `EXISTS` expression.
#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::SqlxBridge)]
pub(crate) struct Exists(bool);

impl Exists {
    /// Returns the existence fact as a Rust boolean.
    #[must_use]
    pub(crate) const fn into_bool(self) -> bool {
        self.0
    }
}

/// SQL-standard identifier quoting: wrap in double quotes, doubling any interior `"`.
pub(crate) fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// SQL-standard literal quoting: wrap in single quotes, doubling any interior `'`.
pub(crate) fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_identifier_wraps_and_escapes_double_quotes() {
        assert_eq!(quote_identifier("users"), "\"users\"");
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn quote_literal_wraps_and_escapes_single_quotes() {
        assert_eq!(quote_literal("password"), "'password'");
        assert_eq!(quote_literal("can't"), "'can''t'");
    }

    #[test]
    fn row_count_rejects_negative_values_and_converts_valid_boundaries_losslessly() {
        assert_eq!(RowCount::try_from(0).unwrap().into_u64(), 0);
        assert_eq!(
            RowCount::try_from(i64::MAX).unwrap().into_u64(),
            i64::MAX.unsigned_abs()
        );
        assert!(RowCount::try_from(-1).is_err());
    }
}
