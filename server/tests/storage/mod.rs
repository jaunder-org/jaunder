use chrono::Utc;
use common::config_key::SiteConfigKey;
use common::test_support::{
    parse_bio, parse_display_name, parse_email, parse_raw_token, parse_session_label,
};
use host::invite::InviteCode;

use storage::{
    ConfirmPasswordResetError, CreateUserError, ProfileUpdate, RegisterWithInviteError,
    SessionAuthError, UseEmailVerificationError, UsePasswordResetError, UserAuthError,
    UserConfigKey,
};

use rstest::*;
// `#[template]`/`#[apply]` come from the `rstest_reuse` companion crate; the
// glob alone is not enough
// (docs/adr/0124-rstest-reuse-cross-module-templates.md).
use rstest_reuse::*;

use crate::helpers::create_session_for;
use storage::test_support::{Backend, SeedUser, backends};

mod audiences;
mod database;
mod email_verification;
mod feed_events;
mod fixtures;
mod fk_constraints;
mod invites;
mod listing;
mod lookups;
mod media;
mod password_reset;
mod posts;
mod resolution;
mod sessions;
mod site_config;
mod subscriptions;
mod tags;
mod users_auth;

use fixtures::{password, raw_exec, username};

// The Postgres-backed cases below (the `::postgres` expansion of each
// `#[apply(backends)]` test) run against PostgreSQL when `JAUNDER_PG_TEST_URL`
// is set; each acquires its own database (a template clone via
// `unique_postgres_url`/`template_postgres_url`, see helpers), so they run
// safely under the default in-process parallelism. No `--test-threads=1` is
// needed (jaunder-qguq).

#[apply(backends)]
#[tokio::test]
async fn site_config_set_then_get_roundtrips(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    state
        .site_config
        .set(SiteConfigKey::SiteTitle, "Parity Site")
        .await
        .unwrap();
    assert_eq!(
        state
            .site_config
            .get(SiteConfigKey::SiteTitle)
            .await
            .unwrap()
            .as_deref(),
        Some("Parity Site")
    );
}

#[apply(backends)]
#[tokio::test]
async fn get_missing_key_returns_none(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    assert!(
        state
            .site_config
            .get(SiteConfigKey::SiteTitle)
            .await
            .unwrap()
            .is_none()
    );
}

#[apply(backends)]
#[tokio::test]
async fn set_overwrites_existing_value(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    state
        .site_config
        .set(SiteConfigKey::SiteTitle, "First")
        .await
        .unwrap();
    state
        .site_config
        .set(SiteConfigKey::SiteTitle, "Second")
        .await
        .unwrap();

    assert_eq!(
        state
            .site_config
            .get(SiteConfigKey::SiteTitle)
            .await
            .unwrap()
            .as_deref(),
        Some("Second")
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_user_duplicate_and_authenticate_work(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let username = username("alice");
    let initial_password = password("password123");

    let user_id = state
        .users
        .create_user(
            &username,
            &initial_password,
            Some(&parse_display_name("Alice")),
            false,
        )
        .await
        .unwrap();
    let record = state
        .users
        .get_user_by_username(&username)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.user_id, user_id);

    let duplicate = state
        .users
        .create_user(&username, &password("other_password"), None, false)
        .await
        .unwrap_err();
    assert!(matches!(duplicate, CreateUserError::UsernameTaken));

    let authed = state
        .users
        .authenticate(&username, &initial_password)
        .await
        .unwrap();
    assert_eq!(authed.username, "alice");
    assert!(authed.last_authenticated_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn session_lifecycle_works(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await;

    let raw_token = state
        .sessions
        .create_session(user.user_id, &parse_session_label("Laptop"))
        .await
        .unwrap();
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.user_id, user.user_id);
    assert_eq!(record.username, user.username);

    let sessions = state.sessions.list_sessions(user.user_id).await.unwrap();
    assert_eq!(sessions.len(), 1);
    state
        .sessions
        .revoke_session(&record.token_hash)
        .await
        .unwrap();
    let err = state.sessions.authenticate(&raw_token).await.unwrap_err();
    assert!(matches!(err, SessionAuthError::SessionNotFound));
}

#[apply(backends)]
#[tokio::test]
async fn invite_and_atomic_registration_work(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let user_id = state
        .atomic
        .create_user_with_invite(
            &username("carol"),
            &password("password123"),
            Some(&parse_display_name("Carol")),
            false,
            &code,
        )
        .await
        .unwrap();
    let created = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(created.username, "carol");

    let err = state
        .atomic
        .create_user_with_invite(
            &username("carol2"),
            &password("password123"),
            None,
            false,
            &code,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RegisterWithInviteError::InviteAlreadyUsed));
}

#[apply(backends)]
#[tokio::test]
async fn email_verification_and_password_reset_work(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await;
    let user_id = user.user_id;

    let verify_token = state
        .email_verifications
        .create_email_verification(
            user_id,
            &"dave@example.com".parse().unwrap(),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();
    let (verified_user_id, verified_email) = state
        .email_verifications
        .use_email_verification(&verify_token)
        .await
        .unwrap();
    assert_eq!(verified_user_id, user_id);
    assert_eq!(verified_email, "dave@example.com");

    state
        .users
        .set_email(user_id, Some(&"dave@example.com".parse().unwrap()), true)
        .await
        .unwrap();

    let reset_token = state
        .password_resets
        .create_password_reset(user_id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    let claimed_user_id = state
        .password_resets
        .use_password_reset(&reset_token)
        .await
        .unwrap();
    assert_eq!(claimed_user_id, user_id);

    let reset_token = state
        .password_resets
        .create_password_reset(user_id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    state
        .atomic
        .confirm_password_reset(&reset_token, &password("new_password123"))
        .await
        .unwrap();

    let authed = state
        .users
        .authenticate(&user.username, &password("new_password123"))
        .await
        .unwrap();
    assert_eq!(authed.user_id, user_id);
}

// --- UserStorage integration tests ---

// --- SessionStorage integration tests ---

// --- InviteStorage integration tests ---

// --- UserStorage::set_email integration tests ---

// --- EmailVerificationStorage integration tests ---

// --- UserStorage::set_password integration tests ---

// --- PasswordResetStorage integration tests ---

// ---------------------------------------------------------------------------
// PostStorage integration tests
// ---------------------------------------------------------------------------

// ── UserConfigStorage tests ───────────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn user_config_get_returns_none_when_unset(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert!(val.is_none());
}

/// D8: the typed key is the only way in, and a value survives it unchanged.
#[apply(backends)]
#[tokio::test]
async fn user_config_round_trips_through_typed_keys(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "markdown")
        .await
        .unwrap();
    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert_eq!(val.as_deref(), Some("markdown"));
}

#[apply(backends)]
#[tokio::test]
async fn user_config_set_and_get(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "org")
        .await
        .unwrap();
    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert_eq!(val.as_deref(), Some("org"));
}

#[apply(backends)]
#[tokio::test]
async fn user_config_overwrite(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "markdown")
        .await
        .unwrap();
    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "org")
        .await
        .unwrap();
    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert_eq!(val.as_deref(), Some("org"));
}

#[apply(backends)]
#[tokio::test]
async fn user_config_delete_removes_key(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "org")
        .await
        .unwrap();
    state
        .user_config
        .delete(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert!(val.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn user_config_delete_nonexistent_is_ok(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .delete(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
}
