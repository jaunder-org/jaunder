use std::sync::Arc;

use crate::mailer::MailSender;

use super::{
    AtomicOps, EmailVerificationStorage, InviteStorage, PasswordResetStorage, PostStorage,
    SessionStorage, SiteConfigStorage, UserStorage,
};

/// Application-wide state bundling all storage handles.
pub struct AppState {
    pub site_config: Arc<dyn SiteConfigStorage>,
    pub users: Arc<dyn UserStorage>,
    pub sessions: Arc<dyn SessionStorage>,
    pub invites: Arc<dyn InviteStorage>,
    /// Cross-table atomic operations.  The concrete implementation (in the
    /// `server` crate) holds the database pool so `common` and `web` stay
    /// free of `SQLite` implementation details.
    pub atomic: Arc<dyn AtomicOps>,
    /// Email verification token storage (stub until Step 7).
    pub email_verifications: Arc<dyn EmailVerificationStorage>,
    /// Password reset token storage (stub until Step 8).
    pub password_resets: Arc<dyn PasswordResetStorage>,
    /// Post storage.
    pub posts: Arc<dyn PostStorage>,
    /// Outbound email sender.  `NoopMailSender` when SMTP is not configured.
    pub mailer: Arc<dyn MailSender>,
}
