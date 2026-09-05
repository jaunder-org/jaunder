use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, Sender, channel},
};

use async_trait::async_trait;
use axum::http::StatusCode;
use common::MutationOutcome;
use common::mailer::{EmailMessage, MailError, MailSender, test_utils::CapturingMailSender};
use common::site::SiteIdentity;
use common::test_support::{parse_email, parse_raw_token};
use common::time::UtcInstant;
use server_fn::ServerFn;
use storage::{
    AppState, EmailVerified, MockPasswordResetStorage, MockSiteConfigStorage, MockUserStorage,
    UserRecord,
};
use tokio::sync::oneshot;

use crate::helpers::{
    SeededSession, assert_no_email, assert_one_absolute_link_email, create_session_for,
    create_user_and_session, post_form_with_mailer, post_password_reset_form_with_dependencies,
    post_password_reset_request_with_dependencies, post_server_fn_request_fixture_with_mailer,
    post_server_fn_with_mailer,
};
use storage::test_support::{Backend, SeedUser, TestEnv, backends, mock_write_scope};

#[derive(serde::Serialize)]
struct ConfirmPasswordResetDecodeFixture<'a> {
    token: &'a str,
    new_password: &'a str,
}

use rstest::*;

#[test]
fn password_reset_request_keeps_its_route() {
    assert_eq!(
        <web::password_reset::Request as ServerFn>::PATH,
        "/api/password_reset/request"
    );
}

async fn await_sent_count(mailer: &CapturingMailSender, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while mailer.sent().len() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("detached password-reset delivery should finish");
}

fn reset_token_from_message(message: &EmailMessage) -> common::token::RawToken {
    let token = message
        .body_text
        .lines()
        .find_map(|line| line.strip_prefix("https://example.com/reset-password?token="))
        .expect("password-reset email contains an absolute reset link");
    parse_raw_token(token)
}

async fn assert_invalid_reset_identifier_does_not_start_worker(
    state: &Arc<AppState>,
    identifier: &str,
) {
    let mailer = Arc::new(CapturingMailSender::new());
    let mut users = MockUserStorage::new();
    users.expect_get_user_by_username().never();
    users.expect_get_users_by_email().never();
    let mut password_resets = MockPasswordResetStorage::new();
    password_resets.expect_create_password_reset().never();
    let mut site_config = MockSiteConfigStorage::new();
    site_config.expect_get_identity().never();

    let (status, _) = post_password_reset_form_with_dependencies(
        state,
        mailer.clone(),
        format!("identifier={identifier}"),
        Arc::new(users),
        Arc::new(password_resets),
        mock_write_scope(),
        Arc::new(site_config),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_no_email(&mailer);
}
use rstest_reuse::*;

/// Creates a user with a verified email address and an authenticated session.
async fn create_user_with_verified_email(state: &Arc<AppState>, email: &str) -> SeededSession {
    let session = create_user_and_session(state).await;
    let email = parse_email(email);
    let users = Arc::clone(&state.users);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                users
                    .set_email(
                        transaction,
                        session.user_id,
                        Some(&email),
                        EmailVerified::VERIFIED,
                    )
                    .await
            })
        })
        .await
        .expect("set verified email");
    assert!(matches!(outcome, MutationOutcome::Confirmed(())));
    session
}

struct TerminalMailer {
    terminal: Sender<()>,
}

#[async_trait]
impl MailSender for TerminalMailer {
    async fn send_email(&self, _: &EmailMessage) -> Result<(), MailError> {
        self.terminal
            .send(())
            .expect("test waits for detached worker completion");
        Ok(())
    }
}

struct TerminalFailingMailer {
    terminal: Mutex<Option<oneshot::Sender<()>>>,
}

#[async_trait]
impl MailSender for TerminalFailingMailer {
    async fn send_email(&self, _: &EmailMessage) -> Result<(), MailError> {
        self.terminal
            .lock()
            .expect("terminal mutex")
            .take()
            .expect("mail sender runs once")
            .send(())
            .expect("test waits for detached worker completion");
        Err(MailError::Send(Box::new(std::io::Error::other(
            "mail failure for private-reset@example.test with reset-token-secret",
        ))))
    }
}

fn assert_redacted_reset_failure(event: &str) {
    assert!(
        event.contains(r#""error.source":"redacted""#),
        "reset failure source is redacted: {event}"
    );
    for prohibited in [
        "private-reset@example.test",
        "reset-token-secret",
        "submitted-identifier",
        "super-secret-password",
    ] {
        assert!(
            !event.contains(prohibited),
            "reset failure telemetry must exclude {prohibited}: {event}"
        );
    }
}

fn reset_site_config() -> MockSiteConfigStorage {
    let mut site_config = MockSiteConfigStorage::new();
    site_config.expect_get_identity().returning(|| {
        Ok(SiteIdentity {
            title: "Jaunder".parse().expect("valid title"),
            base_url: Some("https://example.com/".parse().expect("valid base URL")),
        })
    });
    site_config
}

// Held lookup and token issuance prove the routed server function returns before
// either account-dependent phase. The mailer's terminal signal makes the
// post-release assertion deterministic rather than scheduler-dependent.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_returns_before_gated_lookup_and_token_then_delivers(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_with_verified_email(&state, "alice@example.com").await;
    let user = state
        .users
        .get_user(session.user_id)
        .await
        .expect("load seeded user")
        .expect("seeded user exists");
    let (lookup_entered_tx, lookup_entered_rx) = channel();
    let (lookup_release_tx, lookup_release_rx): (Sender<()>, Receiver<()>) = channel();
    let lookup_release_rx = Arc::new(Mutex::new(lookup_release_rx));
    let mut users = MockUserStorage::new();
    users.expect_get_user_by_username().return_once(move |_| {
        lookup_entered_tx
            .send(())
            .expect("worker reports lookup entry");
        lookup_release_rx
            .lock()
            .expect("lookup release mutex")
            .recv()
            .expect("test releases lookup");
        Ok(Some(user))
    });
    let (token_entered_tx, token_entered_rx) = channel();
    let (token_release_tx, token_release_rx): (Sender<()>, Receiver<()>) = channel();
    let token_release_rx = Arc::new(Mutex::new(token_release_rx));
    let mut password_resets = MockPasswordResetStorage::new();
    password_resets
        .expect_create_password_reset()
        .return_once(move |_, _, _| {
            token_entered_tx
                .send(())
                .expect("worker reports token issuance entry");
            token_release_rx
                .lock()
                .expect("token release mutex")
                .recv()
                .expect("test releases token issuance");
            Ok(parse_raw_token("token"))
        });
    let (terminal_tx, terminal_rx) = channel();
    let mailer: Arc<dyn MailSender> = Arc::new(TerminalMailer {
        terminal: terminal_tx,
    });
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Username(session.username),
    };
    let (status, _body) = post_password_reset_request_with_dependencies(
        &state,
        mailer,
        &request,
        Arc::new(users),
        Arc::new(password_resets),
        mock_write_scope(),
        Arc::new(reset_site_config()),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    lookup_entered_rx
        .recv()
        .expect("lookup enters after public response");
    lookup_release_tx.send(()).expect("release detached lookup");
    token_entered_rx
        .recv()
        .expect("token issuance enters after lookup release");
    token_release_tx
        .send(())
        .expect("release detached token issuance");
    terminal_rx
        .recv()
        .expect("worker delivers after token release");
}

// A structurally valid request is accepted before detached account lookup and
// delivery; eventual worker completion produces the reset email.
#[apply(backends)]
#[tokio::test]
async fn request_password_reset_accepts_and_eventually_sends_for_verified_user(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let session = create_user_with_verified_email(&state, "alice@example.com").await;

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::password_reset::Request as ServerFn>::PATH,
        format!("identifier={}", session.username),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    await_sent_count(&mailer, 1).await;
    assert_one_absolute_link_email(&mailer, "alice@example.com", "/reset-password");
}

// Email lookup deliberately fans out to every verified exact match; an
// unverified duplicate is excluded without suppressing either eligible User.
#[apply(backends)]
#[tokio::test]
async fn request_password_reset_email_fans_out_only_to_verified_users(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let first = create_user_with_verified_email(&state, "shared@example.com").await;
    let second = create_user_with_verified_email(&state, "shared@example.com").await;
    let unverified = SeedUser::new().seed(&state).await;
    let email = parse_email("shared@example.com");
    let users = Arc::clone(&state.users);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                users
                    .set_email(
                        transaction,
                        unverified.user_id,
                        Some(&email),
                        EmailVerified::UNVERIFIED,
                    )
                    .await
            })
        })
        .await
        .expect("set unverified duplicate");
    assert!(matches!(outcome, MutationOutcome::Confirmed(())));

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::password_reset::Request as ServerFn>::PATH,
        "identifier=shared%40example.com",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    await_sent_count(&mailer, 2).await;
    let messages = mailer.sent();
    assert!(
        messages
            .iter()
            .all(|message| message.to == vec![parse_email("shared@example.com")])
    );

    let first_token = reset_token_from_message(&messages[0]);
    let second_token = reset_token_from_message(&messages[1]);
    assert_ne!(
        first_token.as_ref(),
        second_token.as_ref(),
        "each User receives a distinct token"
    );
    for (token, new_password) in [
        (first_token, "first-reset-password"),
        (second_token, "second-reset-password"),
    ] {
        let (status, _body) = post_server_fn_with_mailer(
            &state,
            &mailer,
            &web::password_reset::Confirm {
                request: web::password_reset::ConfirmPasswordResetRequest {
                    token,
                    new_password: new_password.parse().expect("valid test password"),
                },
            },
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    for (session, password) in [
        (first, "first-reset-password"),
        (second, "second-reset-password"),
    ] {
        let users = Arc::clone(&state.users);
        let authentication = users
            .prepare_authentication(
                &session.username,
                &password.parse().expect("valid password"),
            )
            .await
            .expect("reset password authenticates its User");
        let outcome = state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { users.authenticate(transaction, authentication).await })
            })
            .await
            .expect("authenticate reset password");
        assert!(matches!(outcome, MutationOutcome::Confirmed(_)));
    }
}

// Missing delivery configuration is detached operational failure, not a public
// account signal; it reports one redacted server error and produces no mail.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_base_url_failure_is_neutral_and_reported_once(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let session = create_user_with_verified_email(&state, "private-reset@example.test").await;
    let (terminal_tx, terminal_rx) = oneshot::channel();
    let mut site_config = MockSiteConfigStorage::new();
    site_config.expect_get_identity().return_once(move || {
        terminal_tx
            .send(())
            .expect("worker reports base URL failure");
        Ok(SiteIdentity {
            title: "Jaunder".parse().expect("valid title"),
            base_url: None,
        })
    });
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Username(session.username),
    };

    let ((status, _body), event) = crate::assert_error_signal!(
        async {
            let response = post_password_reset_request_with_dependencies(
                &state,
                mailer.clone(),
                &request,
                state.users.clone(),
                state.password_resets.clone(),
                state.write_scope.clone(),
                Arc::new(site_config),
            )
            .await;
            terminal_rx
                .await
                .expect("worker terminates after base URL failure");
            response
        },
        event = "error swallowed after reporting",
        event_kind = "validation",
        event_class = "client",
        metric_kind = "validation",
        metric_class = "client",
        disposition = "swallowed",
        context = "web.password_reset.request"
    );

    assert_eq!(status, StatusCode::OK);
    assert_redacted_reset_failure(&event);
    assert_no_email(&mailer);
}

// A failed detached lookup must leave the public response neutral while
// reporting through the bounded server channel without exporting the identifier.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_lookup_failure_is_neutral_and_reported_once(
    #[case] backend: Backend,
    #[values("private-reset", "private-reset@example.test")] raw_identifier: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let (terminal_tx, terminal_rx) = oneshot::channel();
    let identifier = raw_identifier
        .parse::<web::password_reset::PasswordResetIdentifier>()
        .expect("valid reset identifier");
    let mut users = MockUserStorage::new();
    match &identifier {
        web::password_reset::PasswordResetIdentifier::Username(_) => {
            users.expect_get_user_by_username().return_once(move |_| {
                terminal_tx.send(()).expect("worker reports lookup failure");
                Err(sqlx::Error::Io(std::io::Error::other(
                    "lookup failed for private submitted identifier",
                )))
            });
        }
        web::password_reset::PasswordResetIdentifier::Email(_) => {
            users.expect_get_users_by_email().return_once(move |_| {
                terminal_tx.send(()).expect("worker reports lookup failure");
                Err(sqlx::Error::Io(std::io::Error::other(
                    "lookup failed for private submitted identifier",
                )))
            });
        }
    }
    let request = web::password_reset::Request { identifier };

    let ((status, _body), event) = crate::assert_error_signal!(
        async {
            let response = post_password_reset_request_with_dependencies(
                &state,
                mailer.clone(),
                &request,
                Arc::new(users),
                state.password_resets.clone(),
                state.write_scope.clone(),
                state.site_config.clone(),
            )
            .await;
            terminal_rx
                .await
                .expect("worker terminates after lookup failure");
            response
        },
        event = "error swallowed after reporting",
        event_kind = "storage",
        event_class = "bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "swallowed",
        context = "web.password_reset.request"
    );

    assert_eq!(status, StatusCode::OK);
    assert_redacted_reset_failure(&event);
    assert_no_email(&mailer);
}

// A token-write rollback is contained to detached work, including its storage
// error, so it cannot reveal account state through the accepted response.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_token_write_failure_is_neutral_and_reported_once(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let session = create_user_with_verified_email(&state, "private-reset@example.test").await;
    let user = state
        .users
        .get_user(session.user_id)
        .await
        .expect("load seeded user")
        .expect("seeded user exists");
    let (terminal_tx, terminal_rx) = oneshot::channel();
    let mut users = MockUserStorage::new();
    users
        .expect_get_user_by_username()
        .return_once(move |_| Ok(Some(user)));
    let mut password_resets = MockPasswordResetStorage::new();
    password_resets
        .expect_create_password_reset()
        .return_once(move |_, _, _| {
            terminal_tx
                .send(())
                .expect("worker reports token-write failure");
            Err(sqlx::Error::Io(std::io::Error::other(
                "token reset-token-secret rejected with super-secret-password",
            )))
        });
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Username(session.username),
    };

    let ((status, _body), event) = crate::assert_error_signal!(
        async {
            let response = post_password_reset_request_with_dependencies(
                &state,
                mailer.clone(),
                &request,
                Arc::new(users),
                Arc::new(password_resets),
                state.write_scope.clone(),
                Arc::new(reset_site_config()),
            )
            .await;
            terminal_rx
                .await
                .expect("worker terminates after token-write rollback");
            response
        },
        event = "error swallowed after reporting",
        event_kind = "storage",
        event_class = "bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "swallowed",
        context = "web.password_reset.request"
    );

    assert_eq!(status, StatusCode::OK);
    assert_redacted_reset_failure(&event);
    assert_no_email(&mailer);
}

// Mail transport failure happens after the reset token exists; its error must
// still be redacted and preserve the same neutral public result.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_mail_failure_is_neutral_and_reported_once(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_with_verified_email(&state, "private-reset@example.test").await;
    let user = state
        .users
        .get_user(session.user_id)
        .await
        .expect("load seeded user")
        .expect("seeded user exists");
    let mut users = MockUserStorage::new();
    users
        .expect_get_user_by_username()
        .return_once(move |_| Ok(Some(user)));
    let mut password_resets = MockPasswordResetStorage::new();
    password_resets
        .expect_create_password_reset()
        .return_once(|_, _, _| Ok(parse_raw_token("reset-token-secret")));
    let (terminal_tx, terminal_rx) = oneshot::channel();
    let mailer: Arc<dyn MailSender> = Arc::new(TerminalFailingMailer {
        terminal: Mutex::new(Some(terminal_tx)),
    });
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Username(session.username),
    };

    let ((status, _body), event) = crate::assert_error_signal!(
        async {
            let response = post_password_reset_request_with_dependencies(
                &state,
                Arc::clone(&mailer),
                &request,
                Arc::new(users),
                Arc::new(password_resets),
                state.write_scope.clone(),
                Arc::new(reset_site_config()),
            )
            .await;
            terminal_rx
                .await
                .expect("worker terminates after mail failure");
            response
        },
        event = "error swallowed after reporting",
        event_kind = "internal",
        event_class = "bug",
        metric_kind = "internal",
        metric_class = "bug",
        disposition = "swallowed",
        context = "web.password_reset.request"
    );

    assert_eq!(status, StatusCode::OK);
    assert_redacted_reset_failure(&event);
}

// Users without a verified Email are not eligible, but their structurally valid
// request is indistinguishable from every other accepted reset request.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_is_neutral_for_user_without_verified_email(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let seeded = SeedUser::new().seed(&state).await;
    let user = state
        .users
        .get_user(seeded.user_id)
        .await
        .expect("load seeded user")
        .expect("seeded user exists");
    let (terminal_tx, terminal_rx) = channel();
    let mut users = MockUserStorage::new();
    users.expect_get_user_by_username().return_once(move |_| {
        terminal_tx
            .send(())
            .expect("worker reports ineligible lookup");
        Ok(Some(user))
    });
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Username(seeded.username),
    };

    let (status, _body) = post_password_reset_request_with_dependencies(
        &state,
        mailer.clone(),
        &request,
        Arc::new(users),
        state.password_resets.clone(),
        state.write_scope.clone(),
        state.site_config.clone(),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    terminal_rx
        .recv()
        .expect("worker terminates after ineligible lookup");
    assert_no_email(&mailer);
}

async fn duplicate_verified_email_users(state: &Arc<AppState>) -> Vec<UserRecord> {
    let first = create_user_with_verified_email(state, "shared@example.com").await;
    let second = create_user_with_verified_email(state, "shared@example.com").await;
    vec![
        state
            .users
            .get_user(first.user_id)
            .await
            .expect("load first duplicate user")
            .expect("first duplicate user exists"),
        state
            .users
            .get_user(second.user_id)
            .await
            .expect("load second duplicate user")
            .expect("second duplicate user exists"),
    ]
}

struct FirstMailFailureThenTerminalMailer {
    attempts: Mutex<usize>,
    terminal: Sender<()>,
}

#[async_trait]
impl MailSender for FirstMailFailureThenTerminalMailer {
    async fn send_email(&self, _: &EmailMessage) -> Result<(), MailError> {
        let mut attempts = self.attempts.lock().expect("mail attempt mutex");
        *attempts += 1;
        if *attempts == 1 {
            return Err(MailError::Send(Box::new(std::io::Error::other(
                "first duplicate mail failure with reset-token-secret",
            ))));
        }
        self.terminal
            .send(())
            .expect("test waits for second duplicate delivery");
        Ok(())
    }
}

// Email lookup returns explicit ordered records at its public storage seam. A
// rollback-confirmed token failure for the first record must not skip the next.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_continues_after_first_duplicate_token_failure(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let users_to_deliver = duplicate_verified_email_users(&state).await;
    let first_user_id = users_to_deliver[0].user_id;
    let (terminal_tx, terminal_rx) = channel();
    let mut users = MockUserStorage::new();
    users
        .expect_get_users_by_email()
        .return_once(move |_| Ok(users_to_deliver));
    let mut password_resets = MockPasswordResetStorage::new();
    password_resets
        .expect_create_password_reset()
        .times(2)
        .returning(move |_, user_id, _| {
            if user_id == first_user_id {
                Err(sqlx::Error::Io(std::io::Error::other(
                    "first duplicate token failure with reset-token-secret",
                )))
            } else {
                Ok(parse_raw_token("second-token"))
            }
        });
    let mailer: Arc<dyn MailSender> = Arc::new(TerminalMailer {
        terminal: terminal_tx,
    });
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Email(parse_email(
            "shared@example.com",
        )),
    };

    let ((status, _body), event) = crate::assert_error_signal!(
        async {
            let response = post_password_reset_request_with_dependencies(
                &state,
                mailer,
                &request,
                Arc::new(users),
                Arc::new(password_resets),
                mock_write_scope(),
                Arc::new(reset_site_config()),
            )
            .await;
            terminal_rx
                .recv()
                .expect("second duplicate delivery terminates worker");
            response
        },
        event = "error swallowed after reporting",
        event_kind = "storage",
        event_class = "bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "swallowed",
        context = "web.password_reset.request"
    );

    assert_eq!(status, StatusCode::OK);
    assert_redacted_reset_failure(&event);
}

// A mail failure belongs to one recipient. The explicitly second delivery proves
// the loop did not abort after reporting the first transport failure.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_continues_after_first_duplicate_mail_failure(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let users_to_deliver = duplicate_verified_email_users(&state).await;
    let mut users = MockUserStorage::new();
    users
        .expect_get_users_by_email()
        .return_once(move |_| Ok(users_to_deliver));
    let mut password_resets = MockPasswordResetStorage::new();
    password_resets
        .expect_create_password_reset()
        .times(2)
        .returning(|_, _, _| Ok(parse_raw_token("duplicate-token")));
    let (terminal_tx, terminal_rx) = channel();
    let mailer: Arc<dyn MailSender> = Arc::new(FirstMailFailureThenTerminalMailer {
        attempts: Mutex::new(0),
        terminal: terminal_tx,
    });
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Email(parse_email(
            "shared@example.com",
        )),
    };

    let ((status, _body), event) = crate::assert_error_signal!(
        async {
            let response = post_password_reset_request_with_dependencies(
                &state,
                mailer,
                &request,
                Arc::new(users),
                Arc::new(password_resets),
                mock_write_scope(),
                Arc::new(reset_site_config()),
            )
            .await;
            terminal_rx
                .recv()
                .expect("second duplicate delivery terminates worker");
            response
        },
        event = "error swallowed after reporting",
        event_kind = "internal",
        event_class = "bug",
        metric_kind = "internal",
        metric_class = "bug",
        disposition = "swallowed",
        context = "web.password_reset.request"
    );

    assert_eq!(status, StatusCode::OK);
    assert_redacted_reset_failure(&event);
}

// An acknowledgement lost after commit is WriteScope's sole error report; the
// usable indeterminate token must still be mailed.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_mails_commit_indeterminate_token_once_reported_by_write_scope(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let users_to_deliver = duplicate_verified_email_users(&state).await;
    let (terminal_tx, terminal_rx) = channel();
    let mut users = MockUserStorage::new();
    users
        .expect_get_users_by_email()
        .return_once(move |_| Ok(vec![users_to_deliver[0].clone()]));
    let mut password_resets = MockPasswordResetStorage::new();
    password_resets
        .expect_create_password_reset()
        .return_once(|_, _, _| Ok(parse_raw_token("indeterminate-token")));
    let mailer: Arc<dyn MailSender> = Arc::new(TerminalMailer {
        terminal: terminal_tx,
    });
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Email(parse_email(
            "shared@example.com",
        )),
    };

    let ((status, _body), _event) = crate::assert_error_signal!(
        async {
            let response = post_password_reset_request_with_dependencies(
                &state,
                mailer,
                &request,
                Arc::new(users),
                Arc::new(password_resets),
                storage::test_support::mock_write_scope_with_commit_acknowledgement_loss(),
                Arc::new(reset_site_config()),
            )
            .await;
            terminal_rx
                .recv()
                .expect("indeterminate token is mailed before worker termination");
            response
        },
        event = "error swallowed after reporting",
        event_kind = "storage",
        event_class = "transient",
        metric_kind = "storage",
        metric_class = "transient",
        disposition = "swallowed",
        context = "storage.write_scope.commit_acknowledgement"
    );

    assert_eq!(status, StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn request_password_reset_invalid_identifier_does_not_start_worker(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    assert_invalid_reset_identifier_does_not_start_worker(&state, "invalid username").await;
    assert_invalid_reset_identifier_does_not_start_worker(&state, "invalid%40").await;
}

// Unknown identifiers have the same accepted response as eligible identifiers.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_is_neutral_for_unknown_username(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx): (Sender<()>, Receiver<()>) = channel();
    let (terminal_tx, terminal_rx) = channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let mut users = MockUserStorage::new();
    users.expect_get_user_by_username().return_once(move |_| {
        entered_tx
            .send(())
            .expect("worker reports unknown lookup entry");
        release_rx
            .lock()
            .expect("unknown lookup release mutex")
            .recv()
            .expect("test releases unknown lookup");
        terminal_tx
            .send(())
            .expect("worker reports unknown lookup completion");
        Ok(None)
    });
    let mut password_resets = MockPasswordResetStorage::new();
    password_resets.expect_create_password_reset().never();
    let mut site_config = MockSiteConfigStorage::new();
    site_config.expect_get_identity().never();
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Username(
            common::test_support::parse_username("nobody"),
        ),
    };

    let (status, _body) = post_password_reset_request_with_dependencies(
        &state,
        mailer.clone(),
        &request,
        Arc::new(users),
        Arc::new(password_resets),
        mock_write_scope(),
        Arc::new(site_config),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    entered_rx
        .recv()
        .expect("unknown lookup enters after public response");
    release_tx
        .send(())
        .expect("release unknown detached lookup");
    terminal_rx
        .recv()
        .expect("worker completes after unknown lookup release");
    assert_no_email(&mailer);
}

// Unknown Email lookup is neutral and does not resolve configuration, issue a
// token, or send mail.
#[apply(backends)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_password_reset_is_neutral_for_unknown_email(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let (terminal_tx, terminal_rx) = channel();
    let mut users = MockUserStorage::new();
    users.expect_get_users_by_email().return_once(move |_| {
        terminal_tx
            .send(())
            .expect("worker reports unknown Email lookup completion");
        Ok(Vec::new())
    });
    let mut password_resets = MockPasswordResetStorage::new();
    password_resets.expect_create_password_reset().never();
    let mut site_config = MockSiteConfigStorage::new();
    site_config.expect_get_identity().never();
    let request = web::password_reset::Request {
        identifier: web::password_reset::PasswordResetIdentifier::Email(parse_email(
            "nobody@example.com",
        )),
    };

    let (status, _body) = post_password_reset_request_with_dependencies(
        &state,
        mailer.clone(),
        &request,
        Arc::new(users),
        Arc::new(password_resets),
        mock_write_scope(),
        Arc::new(site_config),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    terminal_rx
        .recv()
        .expect("worker completes unknown Email lookup after public response");
    assert_no_email(&mailer);
}

// M3.11.10: the nested request maps its token and password exactly, applies the
// password, consumes the token, and revokes every existing session.
#[apply(backends)]
#[tokio::test]
async fn confirm_nested_request_maps_token_and_password(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let session = create_user_with_verified_email(&state, "carol@example.com").await;
    let user_id = session.user_id;
    // Create a second session to ensure all are revoked
    create_session_for(&state, user_id).await;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let password_resets = Arc::clone(&state.password_resets);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(transaction, user_id, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = storage::test_support::confirmed_for(outcome, "password-reset fixture setup");

    let (status, _body) = post_server_fn_with_mailer(
        &state,
        &mailer,
        &web::password_reset::Confirm {
            request: web::password_reset::ConfirmPasswordResetRequest {
                token: raw_token,
                new_password: "newpassword456".parse().unwrap(),
            },
        },
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Old password should fail authentication
    let users = Arc::clone(&state.users);
    let username = session.username.clone();
    let password = "password123".parse().unwrap();
    let old_auth = users.prepare_authentication(&username, &password).await;
    assert!(old_auth.is_err(), "old password should no longer work");

    // New password should succeed
    let users = Arc::clone(&state.users);
    let username = session.username.clone();
    let password = "newpassword456".parse().unwrap();
    let authentication = users
        .prepare_authentication(&username, &password)
        .await
        .unwrap();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { users.authenticate(transaction, authentication).await })
        })
        .await
        .unwrap();
    assert!(matches!(outcome, MutationOutcome::Confirmed(_)));

    // All sessions should be revoked
    let sessions = state.sessions.list_sessions(user_id).await.unwrap();
    assert!(sessions.is_empty(), "all sessions should be revoked");
}

// M3.11.11: confirm_password_reset with an expired token returns an error.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_expired_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = create_user_with_verified_email(&state, "dave@example.com")
        .await
        .user_id;

    let expires_at: UtcInstant = "2000-01-02T03:04:05.123456Z".parse().unwrap();
    let password_resets = Arc::clone(&state.password_resets);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(transaction, user_id, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = storage::test_support::confirmed_for(outcome, "password-reset fixture setup");

    let (status, response_body) = post_server_fn_with_mailer(
        &state,
        &mailer,
        &web::password_reset::Confirm {
            request: web::password_reset::ConfirmPasswordResetRequest {
                token: raw_token,
                new_password: "newpassword456".parse().unwrap(),
            },
        },
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
    assert!(
        response_body.contains("\"validation\""),
        "expected a validation-class password-reset error; body: {response_body}"
    );
}

// M3.11.12: confirm_password_reset with an invalid token returns an error.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_invalid_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, response_body) =
        post_server_fn_request_fixture_with_mailer::<web::password_reset::Confirm, _, _>(
            &state,
            &mailer,
            &ConfirmPasswordResetDecodeFixture {
                token: "not-a-real-token",
                new_password: "newpassword456",
            },
            None,
        )
        .await;

    assert_ne!(status, StatusCode::OK);
    assert!(
        response_body.contains("\"validation\""),
        "expected a validation-class password-reset error; body: {response_body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn confirm_nested_request_rejects_malformed_token_before_handler(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let session = create_user_with_verified_email(&state, "malformed@example.com").await;

    // `bad!token` is outside base64url, so `RawToken` rejects it (at wire-decode once
    // `token` is typed). `new_password` is valid-length, so the failure isolates to the
    // token.
    let (status, response_body) =
        post_server_fn_request_fixture_with_mailer::<web::password_reset::Confirm, _, _>(
            &state,
            &mailer,
            &ConfirmPasswordResetDecodeFixture {
                token: "bad!token",
                new_password: "newpassword456",
            },
            None,
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        response_body.contains("server_function"),
        "expected a server-fn decode rejection; body: {response_body}"
    );
    let users = Arc::clone(&state.users);
    let username = session.username.clone();
    let password = "password123".parse().unwrap();
    let authentication = users
        .prepare_authentication(&username, &password)
        .await
        .unwrap();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { users.authenticate(transaction, authentication).await })
        })
        .await
        .unwrap();
    assert!(
        matches!(outcome, MutationOutcome::Confirmed(_)),
        "a malformed token must not change the password"
    );
    assert_eq!(
        state
            .sessions
            .list_sessions(session.user_id)
            .await
            .unwrap()
            .len(),
        1,
        "a malformed token must not revoke sessions"
    );
}

// M3.11.13: confirm_password_reset with an already-used token returns an error.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_used_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = create_user_with_verified_email(&state, "eve@example.com")
        .await
        .user_id;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let password_resets = Arc::clone(&state.password_resets);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(transaction, user_id, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = storage::test_support::confirmed_for(outcome, "password-reset fixture setup");

    let request = web::password_reset::Confirm {
        request: web::password_reset::ConfirmPasswordResetRequest {
            token: raw_token,
            new_password: "newpassword456".parse().unwrap(),
        },
    };

    // Use it once — should succeed
    let (status, _) = post_server_fn_with_mailer(&state, &mailer, &request, None).await;
    assert_eq!(status, StatusCode::OK);

    // Use it again — should fail
    let (status, response_body) = post_server_fn_with_mailer(&state, &mailer, &request, None).await;
    assert_ne!(status, StatusCode::OK);
    assert!(
        response_body.contains("\"validation\""),
        "expected a validation-class password-reset error; body: {response_body}"
    );
}

// A too-short `new_password` is rejected while decoding the nested request before
// the reset is applied.
#[apply(backends)]
#[tokio::test]
async fn confirm_nested_request_rejects_short_password_before_handler(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let session = create_user_with_verified_email(&state, "frank@example.com").await;

    let password_resets = Arc::clone(&state.password_resets);
    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(transaction, session.user_id, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = storage::test_support::confirmed_for(outcome, "password-reset fixture setup");

    let (status, response_body) =
        post_server_fn_request_fixture_with_mailer::<web::password_reset::Confirm, _, _>(
            &state,
            &mailer,
            &ConfirmPasswordResetDecodeFixture {
                token: raw_token.as_ref(),
                new_password: "short",
            },
            None,
        )
        .await;

    // A decode rejection is HTTP 400 with a body tagged `server_function` —
    // distinct from an in-body failure, which projects to
    // `validation`/`unauthorized`/etc. (`WebError` is externally tagged,
    // snake_case.) This is the wire contract the decode-telemetry path in
    // `web::error` sits behind (#822).
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        response_body.contains("server_function"),
        "expected a server-fn decode rejection; body: {response_body}"
    );

    // The reset must not have been applied: the original password still authenticates.
    let users = Arc::clone(&state.users);
    let username = session.username.clone();
    let password = "password123".parse().unwrap();
    let authentication = users
        .prepare_authentication(&username, &password)
        .await
        .unwrap();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { users.authenticate(transaction, authentication).await })
        })
        .await
        .unwrap();
    assert!(
        matches!(outcome, MutationOutcome::Confirmed(_)),
        "a too-short new password must be rejected without applying the reset"
    );
    assert_eq!(
        state
            .sessions
            .list_sessions(session.user_id)
            .await
            .unwrap()
            .len(),
        1,
        "a too-short new password must not revoke sessions"
    );
}
