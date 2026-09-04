//! Centralized application state management.

use std::sync::Arc;

use super::backend::AppStateBackend;
use super::{
    AudienceStorage, AudienceStore, EmailVerificationStorage, EmailVerificationStore,
    FeedCacheStorage, FeedCacheStore, FeedEventStorage, FeedEventStore, InviteStorage, InviteStore,
    MediaStorage, MediaStore, PasswordResetStorage, PasswordResetStore, PostStorage, PostStore,
    PublisherStorage, PublisherStore, SessionStorage, SessionStore, SiteConfigStorage,
    SiteConfigStore, SubscriptionStorage, SubscriptionStore, UserConfigStorage, UserConfigStore,
    UserStorage, UserStore, WriteScope,
};

/// Bundle of every storage handle the application needs.
///
/// `open_database` constructs this struct so callers get all handles in one
/// shot; the composition root then unpacks it — into individual Leptos contexts
/// for `#[server]` functions (see `server::context::provide_app_state_contexts`)
/// and into per-trait axum `Extension`s for the raw HTTP handlers. Consumers
/// never receive the whole `AppState`: they take exactly the `Arc<dyn FooStorage>`
/// handles they need. The bundle is purely a construction convenience; per
/// [ADR-0016](../../docs/adr/0016-dependency-injection-and-appstate.md) it
/// holds *only* storage and is never passed beyond the composition root.
///
/// Services that are not storage — the mailer and the `WebSub` publisher — are
/// constructed by the server (which knows about SMTP / file-capture / HTTP
/// transports) and injected per-consumer, not bundled here.
pub struct AppState {
    /// Interface for site-wide configuration settings.
    pub site_config: Arc<dyn SiteConfigStorage>,
    /// Interface for user account management.
    pub users: Arc<dyn UserStorage>,
    /// Interface for session lifecycle management.
    pub sessions: Arc<dyn SessionStorage>,
    /// Interface for invite code management.
    pub invites: Arc<dyn InviteStorage>,
    /// Storage for email verification tokens.
    pub email_verifications: Arc<dyn EmailVerificationStorage>,
    /// Storage for password reset tokens.
    pub password_resets: Arc<dyn PasswordResetStorage>,
    /// Interface for post and revision management.
    pub posts: Arc<dyn PostStorage>,
    /// Interface for subscription management and the subscription-admission seam.
    pub subscriptions: Arc<dyn SubscriptionStorage>,
    /// Interface for named audiences and their membership.
    pub audiences: Arc<dyn AudienceStorage>,
    /// Interface for media file metadata management.
    pub media: Arc<dyn MediaStorage>,
    /// Interface for per-user preference storage.
    pub user_config: Arc<dyn UserConfigStorage>,
    /// Cache of fully-rendered feed bodies, keyed by canonical feed URL.
    pub feed_cache: Arc<dyn FeedCacheStorage>,
    /// Queue of feed-regeneration events drained by the feed worker.
    pub feed_events: Arc<dyn FeedEventStorage>,
    /// Coherent publisher configuration, hub mutation, and generation-fenced cache writes.
    pub publisher: Arc<dyn PublisherStorage>,
    /// Factory-minted boundary for composing application storage writes.
    pub write_scope: WriteScope,
}

/// Constructs every application storage handle over one concrete database pool.
///
/// The crate-private [`AppStateBackend`] bound keeps pool-to-`WriteScope`
/// construction at this composition seam; public [`crate::Backend`] users cannot
/// mint scopes.
pub(crate) fn make_app_state<DB>(pool: sqlx::Pool<DB>) -> Arc<AppState>
where
    DB: AppStateBackend,
    SiteConfigStore<DB>: SiteConfigStorage,
    UserStore<DB>: UserStorage,
    SessionStore<DB>: SessionStorage,
    InviteStore<DB>: InviteStorage,
    EmailVerificationStore<DB>: EmailVerificationStorage,
    PasswordResetStore<DB>: PasswordResetStorage,
    PostStore<DB>: PostStorage,
    SubscriptionStore<DB>: SubscriptionStorage,
    AudienceStore<DB>: AudienceStorage,
    MediaStore<DB>: MediaStorage,
    UserConfigStore<DB>: UserConfigStorage,
    FeedCacheStore<DB>: FeedCacheStorage,
    FeedEventStore<DB>: FeedEventStorage,
    PublisherStore<DB>: PublisherStorage,
{
    Arc::new(AppState {
        site_config: Arc::new(SiteConfigStore::new(pool.clone())),
        users: Arc::new(UserStore::new(pool.clone())),
        sessions: Arc::new(SessionStore::new(pool.clone())),
        invites: Arc::new(InviteStore::new(pool.clone())),
        email_verifications: Arc::new(EmailVerificationStore::new(pool.clone())),
        password_resets: Arc::new(PasswordResetStore::new(pool.clone())),
        posts: Arc::new(PostStore::new(pool.clone())),
        subscriptions: Arc::new(SubscriptionStore::new(
            pool.clone(),
            Arc::new(common::visibility::OpenSubscriptionPolicy),
        )),
        audiences: Arc::new(AudienceStore::new(pool.clone())),
        media: Arc::new(MediaStore::new(pool.clone())),
        user_config: Arc::new(UserConfigStore::new(pool.clone())),
        feed_cache: Arc::new(FeedCacheStore::new(pool.clone())),
        feed_events: Arc::new(FeedEventStore::new(pool.clone())),
        publisher: Arc::new(PublisherStore::new(pool.clone())),
        write_scope: DB::write_scope(pool),
    })
}

impl AppState {
    /// Borrows the site configuration store.
    #[must_use]
    pub fn site_config(&self) -> &dyn SiteConfigStorage {
        self.site_config.as_ref()
    }

    /// Borrows the user account store.
    #[must_use]
    pub fn users(&self) -> &dyn UserStorage {
        self.users.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::{Backend, backends};
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn opening_constructs_every_app_state_handle(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;

        let _ = (
            state.site_config.as_ref(),
            state.users.as_ref(),
            state.sessions.as_ref(),
            state.invites.as_ref(),
            state.email_verifications.as_ref(),
            state.password_resets.as_ref(),
            state.posts.as_ref(),
            state.subscriptions.as_ref(),
            state.audiences.as_ref(),
            state.media.as_ref(),
            state.user_config.as_ref(),
            state.feed_cache.as_ref(),
            state.feed_events.as_ref(),
            state.publisher.as_ref(),
        );
        assert!(
            state
                .write_scope
                .run(|_| Box::pin(async { Ok::<(), std::convert::Infallible>(()) }))
                .await
                .is_ok()
        );
    }
}
