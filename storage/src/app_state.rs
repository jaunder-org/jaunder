//! Centralized application state management.

use std::sync::Arc;

use super::{
    AudienceStorage, EmailVerificationStorage, FeedCacheStorage, FeedEventStorage, InviteStorage,
    MediaStorage, PasswordResetStorage, PostStorage, PublisherStorage, SessionStorage,
    SiteConfigStorage, SubscriptionStorage, ThemeStorage, UserConfigStorage, UserStorage,
    WriteScope,
};

/// Bundle of every storage handle the application needs.
///
/// [`crate::StorageFactory`] constructs this bundle only when the serve
/// composition root requests every handle. The root then unpacks it into
/// individual Leptos contexts for `#[server]` functions (see
/// `server::context::provide_app_state_contexts`) and per-trait axum
/// `Extension`s for raw HTTP handlers. Consumers never receive the whole
/// `AppState`: they take exactly the `Arc<dyn FooStorage>` handles they need.
/// Per [ADR-0016](../../docs/adr/0016-dependency-injection-and-appstate.md), the
/// bundle holds only storage and never crosses the composition root.
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
    /// Interface for custom public-theme catalogs and immutable revision rows.
    pub themes: Arc<dyn ThemeStorage>,
    /// Factory-minted boundary for composing application storage writes.
    pub write_scope: WriteScope,
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
    use crate::test_support::{Backend, backends, recorded_postgres_url, sqlite_url};
    use crate::{StorageRuntimeConfig, open_database};
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn factory_constructs_every_app_state_handle(#[case] backend: Backend) {
        let env = backend.setup().await;
        let options = match backend {
            Backend::Sqlite => sqlite_url(&env.base),
            Backend::Postgres => recorded_postgres_url(&env.base).parse().unwrap(),
        };
        let factory = open_database(&options, &StorageRuntimeConfig::default())
            .await
            .expect("open database");
        let state = factory.app_state();

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
            state.themes.as_ref(),
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
