use chrono::Utc;
use common::test_support::{parse_bio, parse_display_name, parse_email, parse_raw_token};
use host::invite::InviteCode;

use storage::{
    ConfirmPasswordResetError, ProfileUpdate, RegisterWithInviteError, UseEmailVerificationError,
    UsePasswordResetError, UserAuthError,
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
mod user_config;
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
