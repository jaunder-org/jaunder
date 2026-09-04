//! Wire `AppState`'s storage handles and the mailer into Leptos context.
//!
//! `AppState` bundles the handles for ergonomic construction (one call to
//! `open_database` returns one struct), but consumers (`#[server]` functions)
//! fetch individual traits from Leptos context so each function advertises
//! exactly which storage capabilities it needs.
//!
//! These helpers live in `server` rather than `storage` because the choice to
//! use Leptos context as the DI mechanism is an application-wiring decision,
//! not a storage concern.

use std::sync::Arc;

use crate::publisher::PublisherService;
use common::mailer::MailSender;
use leptos::prelude::provide_context;
use storage::{
    AppState, AudienceStorage, EmailVerificationStorage, FeedEventStorage, InviteStorage,
    MediaContentLocks, MediaManager, MediaStorage, PasswordResetStorage, PostStorage,
    SessionStorage, SiteConfigStorage, SubscriptionStorage, UserConfigStorage, UserStorage,
    WriteScope,
};
use web::websub::WebsubPublisher;

/// Place every storage handle and the factory-minted write scope in `state` into
/// the current Leptos context. Server functions fetch exact trait dependencies
/// with `expect_context::<Arc<dyn FooStorage>>()` and acquire mutations through
/// `expect_context::<WriteScope>()`.
pub fn provide_app_state_contexts(state: &Arc<AppState>, publisher: &Arc<PublisherService>) {
    provide_context::<Arc<dyn UserStorage>>(state.users.clone());
    provide_context::<Arc<dyn SessionStorage>>(state.sessions.clone());
    provide_context::<Arc<dyn InviteStorage>>(state.invites.clone());
    provide_context::<Arc<dyn EmailVerificationStorage>>(state.email_verifications.clone());
    provide_context::<Arc<dyn PasswordResetStorage>>(state.password_resets.clone());
    provide_context::<Arc<dyn PostStorage>>(state.posts.clone());
    provide_context::<WriteScope>(state.write_scope.clone());
    provide_context::<Arc<dyn SubscriptionStorage>>(state.subscriptions.clone());
    provide_context::<Arc<dyn AudienceStorage>>(state.audiences.clone());
    provide_context::<Arc<dyn MediaStorage>>(state.media.clone());
    provide_context::<Arc<dyn UserConfigStorage>>(state.user_config.clone());
    provide_context::<Arc<dyn SiteConfigStorage>>(state.site_config.clone());
    provide_context::<Arc<dyn FeedEventStorage>>(state.feed_events.clone());
    provide_context(Arc::clone(publisher));
    provide_context::<Arc<dyn WebsubPublisher>>(publisher.clone());
}

/// Places the shared media filesystem coordinator in the current Leptos
/// request context.
pub fn provide_media_content_locks_context(content_locks: &Arc<MediaContentLocks>) {
    provide_context(Arc::clone(content_locks));
}

/// Places the shared media operation manager in the current Leptos request context.
pub fn provide_media_manager_context(manager: &Arc<MediaManager>) {
    provide_context(Arc::clone(manager));
}

/// Place the mailer in the current Leptos context. Server functions that
/// send mail fetch it with `expect_context::<Arc<dyn MailSender>>()`.
pub fn provide_mailer_context(mailer: &Arc<dyn MailSender>) {
    provide_context::<Arc<dyn MailSender>>(mailer.clone());
}
