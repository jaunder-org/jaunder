//! Centralized application state management.

use std::sync::Arc;

use common::websub::WebSubClient;

use super::{
    AtomicOps, EmailVerificationStorage, FeedCacheStorage, FeedEventStorage, InviteStorage,
    MediaStorage, PasswordResetStorage, PostStorage, SessionStorage, SiteConfigStorage,
    UserConfigStorage, UserStorage,
};

/// Bundle of every storage handle the application needs.
///
/// `open_database` constructs this struct so callers get all handles in one
/// shot; the server then unpacks it into individual Leptos contexts (see
/// `server::context::provide_app_state_contexts`). Server functions never
/// touch `AppState` directly — they `expect_context::<Arc<dyn FooStorage>>()`
/// for the specific traits they need.
///
/// The mailer deliberately lives outside this bundle. It is not a storage
/// concern, and it is constructed by the server (which knows about SMTP /
/// file-capture transports) rather than by `open_database`.
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
    /// Interface for media file metadata management.
    pub media: Arc<dyn MediaStorage>,
    /// Interface for per-user preference storage.
    pub user_config: Arc<dyn UserConfigStorage>,
    /// Cache of fully-rendered feed bodies, keyed by canonical feed URL.
    pub feed_cache: Arc<dyn FeedCacheStorage>,
    /// Queue of feed-regeneration events drained by the feed worker.
    pub feed_events: Arc<dyn FeedEventStorage>,
    /// `WebSub` publisher used to notify subscribers when feeds change.
    ///
    /// Production builders select the client via
    /// [`common::websub::default_client_from_env`]: a
    /// [`common::websub::FileCapturingWebSubClient`] when
    /// `JAUNDER_WEBSUB_CAPTURE_FILE` is set (e2e capture), otherwise the live
    /// [`common::websub::HttpWebSubClient`]. The worker only pings when
    /// `feeds.websub_hub_url` is configured. Test helpers use
    /// [`common::websub::NoopWebSubClient`].
    pub websub: Arc<dyn WebSubClient>,
}
