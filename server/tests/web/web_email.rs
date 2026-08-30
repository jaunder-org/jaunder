use std::sync::Arc;

use axum::http::StatusCode;
use common::MutationOutcome;
use common::mailer::test_utils::CapturingMailSender;
use common::test_support::parse_email;
use common::time::UtcInstant;
use server_fn::ServerFn;

use crate::helpers::{
    assert_no_email, assert_one_absolute_link_email, create_user_and_session,
    post_form_with_mailer, setup_with_base_url,
};
use storage::test_support::{Backend, SeedUser, TestEnv, backends};

use rstest::*;
use rstest_reuse::*;

// M3.10.7: request_email_verification creates a row and sends an email via CapturingMailSender.
#[apply(backends)]
#[tokio::test]
async fn request_email_verification_creates_row_and_sends_email(#[case] backend: Backend) {
    // The verification email composes an absolute link, so the flow requires a
    // seeded `site.base_url` (canonicalized to `https://example.com/`).
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let mailer = Arc::new(CapturingMailSender::new());

    let cookie = create_user_and_session(&state).await.cookie();

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::RequestVerification as ServerFn>::PATH,
        "email=alice%40example.com",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_one_absolute_link_email(&mailer, "alice@example.com", "/verify-email");
}

// The verification email composes an absolute link, so without a seeded
// `site.base_url` the request fails rather than emailing a dead relative link.
#[apply(backends)]
#[tokio::test]
async fn request_email_verification_without_base_url_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await; // no base_url seeded
    let mailer = Arc::new(CapturingMailSender::new());

    let cookie = create_user_and_session(&state).await.cookie();

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::RequestVerification as ServerFn>::PATH,
        "email=alice%40example.com",
        Some(&cookie),
    )
    .await;

    assert_ne!(status, StatusCode::OK, "should fail without a base URL");
    assert_no_email(&mailer);
}

// M3.10.8: verify_email with a valid token sets the email as verified.
#[apply(backends)]
#[tokio::test]
async fn verify_email_with_valid_token_sets_email_verified(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = SeedUser::new().seed(&state).await.user_id;

    let email = parse_email("bob@example.com");
    let fixture_email = email.clone();
    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let email_verifications = Arc::clone(&state.email_verifications);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                email_verifications
                    .create_email_verification(transaction, user_id, &fixture_email, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = match outcome {
        MutationOutcome::Confirmed(raw_token) => raw_token,
        MutationOutcome::CommitIndeterminate(_) => {
            panic!("email-verification fixture setup requires a confirmed commit")
        }
    };

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::Verify as ServerFn>::PATH,
        format!("token={raw_token}"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(user.email, Some(email));
    assert!(user.email_verified, "email should be marked as verified");
}

/// A later email update failure rolls back verification-token consumption.
#[apply(backends)]
#[tokio::test]
async fn verify_email_set_failure_rolls_back_token_consumption(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let user_id = SeedUser::new().seed(&state).await.user_id;
    let email = parse_email("rollback@example.com");
    let email_for_token = email.clone();
    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let verifications = Arc::clone(&state.email_verifications);
    let raw_token = match state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                verifications
                    .create_email_verification(transaction, user_id, &email_for_token, expires_at)
                    .await
            })
        })
        .await
        .unwrap()
    {
        MutationOutcome::Confirmed(token) => token,
        MutationOutcome::CommitIndeterminate(_) => panic!("fixture requires a confirmed commit"),
    };
    match backend {
        Backend::Sqlite => {
            base.pool()
                .execute(
                    "CREATE TRIGGER fail_email_update BEFORE UPDATE OF email ON users \
                     BEGIN SELECT RAISE(FAIL, 'blocked'); END",
                )
                .await
                .unwrap();
        }
        Backend::Postgres => {
            base.pool()
                .execute(
                    "CREATE FUNCTION fail_email_update() RETURNS trigger AS $$ \
                     BEGIN RAISE EXCEPTION 'blocked'; END; $$ LANGUAGE plpgsql",
                )
                .await
                .unwrap();
            base.pool()
                .execute(
                    "CREATE TRIGGER fail_email_update BEFORE UPDATE OF email ON users \
                     FOR EACH ROW EXECUTE FUNCTION fail_email_update()",
                )
                .await
                .unwrap();
        }
    }
    let (status, _) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::Verify as ServerFn>::PATH,
        format!("token={raw_token}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert!(user.email.is_none());

    match backend {
        Backend::Sqlite => base
            .pool()
            .execute("DROP TRIGGER fail_email_update")
            .await
            .unwrap(),
        Backend::Postgres => {
            base.pool()
                .execute("DROP TRIGGER fail_email_update ON users")
                .await
                .unwrap();
            base.pool()
                .execute("DROP FUNCTION fail_email_update()")
                .await
                .unwrap();
        }
    }
    let (status, _) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::Verify as ServerFn>::PATH,
        format!("token={raw_token}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(user.email, Some(email));
    assert!(user.email_verified);
}

// M3.10.9: verify_email with an expired token returns an error.
#[apply(backends)]
#[tokio::test]
async fn verify_email_with_expired_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = SeedUser::new().seed(&state).await.user_id;

    let email = "carol@example.com".parse().unwrap();
    let expires_at: UtcInstant = "2000-01-02T03:04:05.123456Z".parse().unwrap();
    let email_verifications = Arc::clone(&state.email_verifications);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                email_verifications
                    .create_email_verification(transaction, user_id, &email, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = match outcome {
        MutationOutcome::Confirmed(raw_token) => raw_token,
        MutationOutcome::CommitIndeterminate(_) => {
            panic!("email-verification fixture setup requires a confirmed commit")
        }
    };

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::Verify as ServerFn>::PATH,
        format!("token={raw_token}"),
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.10.10: verify_email with an unknown token returns an error.
#[apply(backends)]
#[tokio::test]
async fn verify_email_with_unknown_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::Verify as ServerFn>::PATH,
        "token=this_token_does_not_exist",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn verify_email_with_malformed_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    // `bad!token` is not valid base64url shape, so `RawToken` rejects it — in-body today,
    // at wire-decode once `token` is typed. Either way a non-OK response.
    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::Verify as ServerFn>::PATH,
        "token=bad!token",
        None,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "a malformed verification token must be rejected"
    );
}

#[apply(backends)]
#[tokio::test]
async fn request_email_verification_unauthorized_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::RequestVerification as ServerFn>::PATH,
        "email=alice@example.com",
        None,
    )
    .await;

    // Leptos server functions return 500 for ServerFnError (which require_auth returns).
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn request_email_verification_invalid_email_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let cookie_header = create_user_and_session(&state).await.cookie();

    let (status, _) = post_form_with_mailer(
        &state,
        &mailer,
        <web::email::RequestVerification as ServerFn>::PATH,
        "email=invalid",
        Some(&cookie_header),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
