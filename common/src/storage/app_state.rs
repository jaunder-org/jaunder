//! Centralized application state management.

use std::sync::Arc;

use crate::mailer::MailSender;

use super::{
    AtomicOps, EmailVerificationStorage, InviteStorage, MediaStorage, PasswordResetStorage,
    PostStorage, SessionStorage, SiteConfigStorage, UserConfigStorage, UserStorage,
};

/// Application-wide state bundling all storage handles.
///
/// This struct is the primary way that application logic (both in the server
/// and in Leptos server functions) accesses the persistence layer. It uses
/// `Arc<dyn Trait>` to remain agnostic of the concrete database implementation
/// (e.g., `SQLite` vs. `PostgreSQL`).
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
    /// The system's outbound email sender.
    pub mailer: Arc<dyn MailSender>,
}
