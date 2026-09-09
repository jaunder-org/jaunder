//! Pool-owning storage construction for composition roots.
//!
//! [`StorageFactory`] erases the runtime-selected database while retaining the
//! concrete pool needed to construct object-safe storage handles and
//! [`WriteScope`]. Composition roots request only the dependencies they need;
//! application code receives those dependencies directly and never receives the
//! factory.

use std::sync::Arc;

use sqlx::{PgPool, SqlitePool};

use crate::backend::WriteScopeFactoryBackend;
use crate::{
    AudienceStorage, AudienceStore, EmailVerificationStorage, EmailVerificationStore,
    FeedCacheStorage, FeedCacheStore, FeedEventStorage, FeedEventStore, InviteStorage, InviteStore,
    MediaStorage, MediaStore, PasswordResetStorage, PasswordResetStore, PostStorage, PostStore,
    PublisherStorage, PublisherStore, SessionStorage, SessionStore, SiteConfigStorage,
    SiteConfigStore, SubscriptionStorage, SubscriptionStore, ThemeStorage, ThemeStore,
    UserConfigStorage, UserConfigStore, UserStorage, UserStore, WriteScope,
};

/// Pool-owning factory for the storage dependencies assembled at a composition root.
pub struct StorageFactory {
    inner: StorageFactoryInner,
}

enum StorageFactoryInner {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl StorageFactory {
    pub(crate) fn sqlite(pool: SqlitePool) -> Self {
        Self {
            inner: StorageFactoryInner::Sqlite(pool),
        }
    }

    pub(crate) fn postgres(pool: PgPool) -> Self {
        Self {
            inner: StorageFactoryInner::Postgres(pool),
        }
    }

    /// Constructs site-configuration storage over the owned pool.
    #[must_use]
    pub fn site_config(&self) -> Arc<dyn SiteConfigStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(SiteConfigStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(SiteConfigStore::new(pool.clone())),
        }
    }

    /// Constructs user storage over the owned pool.
    #[must_use]
    pub fn users(&self) -> Arc<dyn UserStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(UserStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(UserStore::new(pool.clone())),
        }
    }

    /// Constructs session storage over the owned pool.
    #[must_use]
    pub fn sessions(&self) -> Arc<dyn SessionStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(SessionStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(SessionStore::new(pool.clone())),
        }
    }

    /// Constructs Invitation storage over the owned pool.
    #[must_use]
    pub fn invites(&self) -> Arc<dyn InviteStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(InviteStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(InviteStore::new(pool.clone())),
        }
    }

    /// Constructs email-verification storage over the owned pool.
    #[must_use]
    pub fn email_verifications(&self) -> Arc<dyn EmailVerificationStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => {
                Arc::new(EmailVerificationStore::new(pool.clone()))
            }
            StorageFactoryInner::Postgres(pool) => {
                Arc::new(EmailVerificationStore::new(pool.clone()))
            }
        }
    }

    /// Constructs password-reset storage over the owned pool.
    #[must_use]
    pub fn password_resets(&self) -> Arc<dyn PasswordResetStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(PasswordResetStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(PasswordResetStore::new(pool.clone())),
        }
    }

    /// Constructs Post storage over the owned pool.
    #[must_use]
    pub fn posts(&self) -> Arc<dyn PostStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(PostStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(PostStore::new(pool.clone())),
        }
    }

    /// Constructs subscription storage over the owned pool.
    #[must_use]
    pub fn subscriptions(&self) -> Arc<dyn SubscriptionStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(SubscriptionStore::new(
                pool.clone(),
                Arc::new(common::visibility::OpenSubscriptionPolicy),
            )),
            StorageFactoryInner::Postgres(pool) => Arc::new(SubscriptionStore::new(
                pool.clone(),
                Arc::new(common::visibility::OpenSubscriptionPolicy),
            )),
        }
    }

    /// Constructs audience storage over the owned pool.
    #[must_use]
    pub fn audiences(&self) -> Arc<dyn AudienceStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(AudienceStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(AudienceStore::new(pool.clone())),
        }
    }

    /// Constructs Media storage over the owned pool.
    #[must_use]
    pub fn media(&self) -> Arc<dyn MediaStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(MediaStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(MediaStore::new(pool.clone())),
        }
    }

    /// Constructs user-configuration storage over the owned pool.
    #[must_use]
    pub fn user_config(&self) -> Arc<dyn UserConfigStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(UserConfigStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(UserConfigStore::new(pool.clone())),
        }
    }

    /// Constructs Syndication Feed cache storage over the owned pool.
    #[must_use]
    pub fn feed_cache(&self) -> Arc<dyn FeedCacheStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(FeedCacheStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(FeedCacheStore::new(pool.clone())),
        }
    }

    /// Constructs Syndication Feed event storage over the owned pool.
    #[must_use]
    pub fn feed_events(&self) -> Arc<dyn FeedEventStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(FeedEventStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(FeedEventStore::new(pool.clone())),
        }
    }

    /// Constructs publisher storage over the owned pool.
    #[must_use]
    pub fn publisher(&self) -> Arc<dyn PublisherStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(PublisherStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(PublisherStore::new(pool.clone())),
        }
    }

    /// Constructs theme storage over the owned pool.
    #[must_use]
    pub fn themes(&self) -> Arc<dyn ThemeStorage> {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => Arc::new(ThemeStore::new(pool.clone())),
            StorageFactoryInner::Postgres(pool) => Arc::new(ThemeStore::new(pool.clone())),
        }
    }

    /// Constructs the write-composition scope matching the owned pool.
    #[must_use]
    pub fn write_scope(&self) -> WriteScope {
        match &self.inner {
            StorageFactoryInner::Sqlite(pool) => sqlx::Sqlite::write_scope(pool.clone()),
            StorageFactoryInner::Postgres(pool) => sqlx::Postgres::write_scope(pool.clone()),
        }
    }
}
