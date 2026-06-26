//! Centralized application state management.

use std::sync::Arc;

use super::{
    AtomicOps, AudienceStorage, EmailVerificationStorage, FeedCacheStorage, FeedEventStorage,
    InviteStorage, MediaStorage, PasswordResetStorage, PostStorage, SessionStorage,
    SiteConfigStorage, SubscriptionStorage, UserConfigStorage, UserStorage,
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
    /// Cross-table atomic operations.
    ///
    /// These operations span multiple storage traits and are implemented as
    /// atomic transactions in the concrete backend.
    pub atomic: Arc<dyn AtomicOps>,
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
}
