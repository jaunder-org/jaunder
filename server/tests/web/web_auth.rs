use axum::http::StatusCode;
use chrono::Utc;
use common::registration::RegistrationPolicy;
use common::session_label::MAX_SESSION_LABEL_CHARS;
use common::time::UtcInstant;
use common::token::RawToken;
use common::username::Username;
use host::password::Password;
use server_fn::ServerFn;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    create_user_and_session, post_form_with_bearer, post_form_with_credentials,
    post_form_with_secure_flag, post_server_fn_request_fixture_with_secure_flag,
    post_server_fn_with_secure_flag, post_server_fn_with_ua, token_from_set_cookie,
};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};

#[derive(serde::Serialize)]
struct LoginDecodeFixture<'a> {
    username: &'a str,
    password: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct RegistrationDecodeFixture<'a> {
    username: &'a str,
    password: &'a str,
}

/// The session token a login/register response established, read from its
/// `Set-Cookie` header — the only channel that carries it (#533). Tests reach the
/// session this way, which is also what proves the cookie is still being set.
fn session_token_of(set_cookie: Option<String>) -> RawToken {
    let cookie = set_cookie.expect("Set-Cookie header should be present");
    token_from_set_cookie(&cookie)
}

/// Asserts a response body does not carry the session token its `Set-Cookie`
/// established.
///
/// These assertions are the enforcement mechanism named by
/// `docs/adr/0107-web-session-establishment-is-cookie-only.md`, so the invariant
/// is spelled out once here rather than re-derived per endpoint. It compares the
/// token *value*: `register`'s body is a bare `null`, so checking for a `"token"`
/// field name would be vacuous.
fn assert_body_carries_no_token(endpoint: &str, body: &str, token: &RawToken) {
    assert!(
        !body.contains(&**token),
        "{endpoint} body leaked the session token: {body}"
    );
}

fn register_input(
    username: &str,
    password: &str,
    invite_code: Option<&str>,
) -> web::registration::Register {
    web::registration::Register {
        username: username.parse().expect("valid test username"),
        password: password.parse().expect("valid test password"),
        invite_code: invite_code.map(|code| code.parse().expect("valid test invite code")),
    }
}

fn login_input(username: &str, password: &str, label: Option<&str>) -> web::auth::Login {
    web::auth::Login {
        username: username.parse().expect("valid test username"),
        password: password.parse().expect("valid test password"),
        label: label.map(|label| label.parse().expect("valid test session label")),
    }
}

type RecordedField = (String, String);
type RecordedSpan = (String, Vec<RecordedField>);

#[derive(Default)]
struct RecordedSpanFields(std::sync::Mutex<Vec<RecordedSpan>>);

struct RegistrationSpanRecorder(std::sync::Arc<RecordedSpanFields>);

struct Fields(Vec<RecordedField>);

impl tracing::field::Visit for Fields {
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.push((field.name().to_string(), value.to_string()));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .push((field.name().to_string(), format!("{value:?}")));
    }
}

impl<S> tracing_subscriber::layer::Layer<S> for RegistrationSpanRecorder
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        if span.metadata().name() != "web.registration.register" {
            return;
        }

        let mut fields = Fields(Vec::new());
        values.record(&mut fields);
        self.0
            .0
            .lock()
            .expect("registration span recorder mutex")
            .push((span.metadata().name().to_string(), fields.0));
    }
}

fn saw_registration_field(captured: &RecordedSpanFields, field: &str, expected: &str) -> bool {
    captured
        .0
        .lock()
        .expect("registration span recorder mutex")
        .iter()
        .any(|(span, fields)| {
            span == "web.registration.register"
                && fields
                    .iter()
                    .any(|(name, value)| name == field && value == expected)
        })
}

async fn post_register(
    state: &std::sync::Arc<storage::AppState>,
    username: &str,
    invite_code: Option<&str>,
) -> StatusCode {
    let (status, _, _) = post_server_fn_with_secure_flag(
        state,
        &register_input(username, "password123", invite_code),
        None,
        true,
    )
    .await;
    status
}

async fn create_registration_invite(state: &storage::AppState) -> host::invite::InviteCode {
    let invites = std::sync::Arc::clone(&state.invites);
    let expires_at = UtcInstant::from(Utc::now() + chrono::Duration::hours(24));
    storage::test_support::confirmed_for(
        state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { invites.create_invite(transaction, expires_at).await })
            })
            .await
            .expect("create registration fixture invite"),
        "registration fixture setup",
    )
}

// guard:low-level-db — observes one sqlite-backed server-fn span directly.
#[tokio::test]
async fn register_records_decision_determinants_on_the_server_fn_span() {
    use tracing_subscriber::prelude::*;

    let captured = std::sync::Arc::new(RecordedSpanFields::default());
    let subscriber =
        tracing_subscriber::registry().with(RegistrationSpanRecorder(captured.clone()));
    let _guard = tracing::subscriber::set_default(subscriber);

    let TestEnv { state, base: _base } = Backend::Sqlite.setup().await;
    let ignored_open_code = create_registration_invite(&state).await;
    assert_eq!(
        post_register(&state, "detopen", Some(ignored_open_code.as_ref())).await,
        StatusCode::OK
    );

    let site_config = std::sync::Arc::clone(&state.site_config);
    storage::test_support::confirmed_for(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    site_config
                        .set_registration_policy(transaction, RegistrationPolicy::OperatorInvites)
                        .await
                })
            })
            .await
            .unwrap(),
        "set operator-invites registration policy",
    );
    let code = create_registration_invite(&state).await;
    assert_eq!(
        post_register(&state, "detinvite", Some(code.as_ref())).await,
        StatusCode::OK
    );
    assert_ne!(
        post_register(&state, "detmissing", None).await,
        StatusCode::OK
    );

    let site_config = std::sync::Arc::clone(&state.site_config);
    storage::test_support::confirmed_for(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    site_config
                        .set_registration_policy(transaction, RegistrationPolicy::Closed)
                        .await
                })
            })
            .await
            .unwrap(),
        "set closed registration policy",
    );
    assert_ne!(
        post_register(&state, "detclosed", None).await,
        StatusCode::OK
    );

    assert!(saw_registration_field(
        &captured,
        "registration.policy",
        "open"
    ));
    assert!(saw_registration_field(
        &captured,
        "registration.policy",
        "operator_invites"
    ));
    assert!(saw_registration_field(
        &captured,
        "registration.policy",
        "closed"
    ));
    assert!(saw_registration_field(
        &captured,
        "registration.invite_present",
        "true"
    ));
    assert!(saw_registration_field(
        &captured,
        "registration.invite_present",
        "false"
    ));
    for outcome in [
        "create_user",
        "create_user_with_invite",
        "invite_required",
        "closed",
    ] {
        assert!(
            saw_registration_field(&captured, "registration.outcome", outcome),
            "missing outcome {outcome}"
        );
    }
}

// M2.9.8: `register` with Open policy creates the user and establishes the session
// through the HttpOnly cookie alone (#533).
#[apply(backends)]
#[tokio::test]
async fn register_nested_request_maps_open_fields(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, set_cookie, body) = post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let cookie = set_cookie.expect("Set-Cookie header should be present");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");

    let cookie_token = token_from_set_cookie(&cookie);
    assert_body_carries_no_token("register", &body, &cookie_token);

    let user = state
        .users
        .get_user_by_username(&"alice".parse::<Username>().unwrap())
        .await
        .unwrap()
        .expect("user should exist after registration");
    let users = std::sync::Arc::clone(&state.users);
    let username = "alice".parse::<Username>().unwrap();
    let password = "password123".parse::<Password>().unwrap();
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
        .expect("registration password should authenticate");
    let authenticated =
        storage::test_support::confirmed_for(outcome, "registration authentication");
    assert_eq!(authenticated.user_id, user.user_id);

    // …and the cookie actually establishes a session for that user. A
    // `starts_with("session=")` check alone would pass against a cookie carrying a
    // token that authenticates nothing.
    let sessions = std::sync::Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.authenticate(transaction, &cookie_token).await })
        })
        .await
        .expect("the register cookie authenticates");
    let record = storage::test_support::confirmed_for(outcome, "session authentication");
    assert_eq!(record.user_id, user.user_id);
}

#[apply(backends)]
#[tokio::test]
async fn register_duplicate_username_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Register alice once.
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    // Register alice again.
    let (status, _, _) = post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "otherpassword", None),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// M2.9.9: `register` with OperatorInvites + valid code creates a user, marks the invite
// used, and establishes the session through the cookie (#533).
#[apply(backends)]
#[tokio::test]
async fn register_nested_request_maps_invite_code(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend
        .setup()
        .registration(RegistrationPolicy::OperatorInvites)
        .await;
    let invites = std::sync::Arc::clone(&state.invites);
    let expires_at = UtcInstant::from(Utc::now() + chrono::Duration::hours(24));
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { invites.create_invite(transaction, expires_at).await })
        })
        .await
        .unwrap();
    let code = storage::test_support::confirmed_for(outcome, "invite fixture setup");

    let (status, set_cookie, _body) = post_server_fn_with_secure_flag(
        &state,
        &register_input("bob", "password123", Some(code.as_ref())),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let cookie = set_cookie.expect("Set-Cookie header should be present");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");

    let user = state
        .users
        .get_user_by_username(&"bob".parse::<Username>().unwrap())
        .await
        .unwrap();
    let user = user.expect("user should exist after invite registration");
    let users = std::sync::Arc::clone(&state.users);
    let username = "bob".parse::<Username>().unwrap();
    let password = "password123".parse::<Password>().unwrap();
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
        .expect("invite registration password should authenticate");
    let authenticated =
        storage::test_support::confirmed_for(outcome, "invite registration authentication");
    assert_eq!(authenticated.user_id, user.user_id);

    let invites = state.invites.list_invites().await.unwrap();
    let invite = invites
        .iter()
        .find(|i| i.code.as_ref() == code.as_ref())
        .unwrap();
    assert!(invite.used_at.is_some(), "invite should be marked as used");
}

/// A later session insert failure rolls back every open-registration mutation.
#[apply(backends)]
#[tokio::test]
async fn register_open_session_failure_rolls_back_user(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    match backend {
        Backend::Sqlite => {
            base.pool()
                .execute(
                    "CREATE TRIGGER fail_session_insert BEFORE INSERT ON sessions \
                     BEGIN SELECT RAISE(FAIL, 'blocked'); END",
                )
                .await
                .unwrap();
        }
        Backend::Postgres => {
            base.pool()
                .execute(
                    "CREATE FUNCTION fail_session_insert() RETURNS trigger AS $$ \
                     BEGIN RAISE EXCEPTION 'blocked'; END; $$ LANGUAGE plpgsql",
                )
                .await
                .unwrap();
            base.pool()
                .execute(
                    "CREATE TRIGGER fail_session_insert BEFORE INSERT ON sessions \
                     FOR EACH ROW EXECUTE FUNCTION fail_session_insert()",
                )
                .await
                .unwrap();
        }
    }

    let (status, _, _) = post_server_fn_with_secure_flag(
        &state,
        &register_input("rolledback", "password123", None),
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        state
            .users
            .get_user_by_username(&"rolledback".parse().unwrap())
            .await
            .unwrap()
            .is_none()
    );
}

/// A later session insert failure rolls back both invite consumption and user creation.
#[apply(backends)]
#[tokio::test]
async fn register_invite_session_failure_rolls_back_user_and_invite(#[case] backend: Backend) {
    let TestEnv { state, base } = backend
        .setup()
        .registration(RegistrationPolicy::OperatorInvites)
        .await;
    let invites = std::sync::Arc::clone(&state.invites);
    let expires_at = UtcInstant::from(Utc::now() + chrono::Duration::hours(24));
    let code = storage::test_support::confirmed_for(
        state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { invites.create_invite(transaction, expires_at).await })
            })
            .await
            .unwrap(),
        "invite fixture",
    );
    match backend {
        Backend::Sqlite => {
            base.pool()
                .execute(
                    "CREATE TRIGGER fail_session_insert BEFORE INSERT ON sessions \
                     BEGIN SELECT RAISE(FAIL, 'blocked'); END",
                )
                .await
                .unwrap();
        }
        Backend::Postgres => {
            base.pool()
                .execute(
                    "CREATE FUNCTION fail_session_insert() RETURNS trigger AS $$ \
                     BEGIN RAISE EXCEPTION 'blocked'; END; $$ LANGUAGE plpgsql",
                )
                .await
                .unwrap();
            base.pool()
                .execute(
                    "CREATE TRIGGER fail_session_insert BEFORE INSERT ON sessions \
                     FOR EACH ROW EXECUTE FUNCTION fail_session_insert()",
                )
                .await
                .unwrap();
        }
    }

    let (status, _, _) = post_server_fn_with_secure_flag(
        &state,
        &register_input("inviterolledback", "password123", Some(code.as_ref())),
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        state
            .users
            .get_user_by_username(&"inviterolledback".parse().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    let invite = state
        .invites
        .list_invites()
        .await
        .unwrap()
        .into_iter()
        .find(|invite| invite.code.as_ref() == code.as_ref())
        .unwrap();
    assert!(invite.used_at.is_none());
}

// M2.9.10: `register` with OperatorInvites policy and missing code returns an error.
#[apply(backends)]
#[tokio::test]
async fn register_operator_invites_missing_code_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend
        .setup()
        .registration(RegistrationPolicy::OperatorInvites)
        .await;

    let (status, _set_cookie, _body) = post_server_fn_with_secure_flag(
        &state,
        &register_input("carol", "password123", None),
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);

    let user = state
        .users
        .get_user_by_username(&"carol".parse::<Username>().unwrap())
        .await
        .unwrap();
    assert!(
        user.is_none(),
        "user should not exist when invite code is missing"
    );
}

// M2.9.15: `register` with OperatorInvites policy and an invalid code returns an error.
#[apply(backends)]
#[tokio::test]
async fn register_operator_invites_invalid_code_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend
        .setup()
        .registration(RegistrationPolicy::OperatorInvites)
        .await;

    let (status, _, _) = post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", Some("invalid-code")),
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M2.9.16: `register` with OperatorInvites policy and an expired code returns an error.
#[apply(backends)]
#[tokio::test]
async fn register_operator_invites_expired_code_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend
        .setup()
        .registration(RegistrationPolicy::OperatorInvites)
        .await;

    // Create an already-expired invite.
    let invites = std::sync::Arc::clone(&state.invites);
    let expires_at = UtcInstant::from(Utc::now() - chrono::Duration::hours(24));
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { invites.create_invite(transaction, expires_at).await })
        })
        .await
        .unwrap();
    let code = storage::test_support::confirmed_for(outcome, "invite fixture setup");

    let (status, _, _) = post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", Some(code.as_ref())),
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M2.9.11: `register` with Closed policy returns error.
#[apply(backends)]
#[tokio::test]
async fn register_closed_policy_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().pristine().await;

    let (status, _set_cookie, _body) = post_server_fn_with_secure_flag(
        &state,
        &register_input("dave", "password123", None),
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);

    let user = state
        .users
        .get_user_by_username(&"dave".parse::<Username>().unwrap())
        .await
        .unwrap();
    assert!(
        user.is_none(),
        "user should not exist on closed registration"
    );
}
/// Registration policy governs direct admission and whether an otherwise valid
/// invitation is redeemed.
#[apply(backends)]
#[tokio::test]
async fn registration_policy_matrix_controls_admission_and_invite_consumption(
    #[case] backend: Backend,
) {
    for (policy, direct_succeeds, invite_succeeds, invite_consumed) in [
        (RegistrationPolicy::Closed, false, false, false),
        (RegistrationPolicy::OperatorInvites, false, true, true),
        (RegistrationPolicy::MemberInvites, false, true, true),
        (RegistrationPolicy::Open, true, true, false),
    ] {
        let TestEnv { state, base: _base } = backend.setup().registration(policy).await;
        let code = create_registration_invite(&state).await;

        let direct_status = post_register(&state, "direct", None).await;
        assert_eq!(
            direct_status == StatusCode::OK,
            direct_succeeds,
            "{policy:?} direct registration status: {direct_status}"
        );

        let invite_status = post_register(&state, "invited", Some(code.as_ref())).await;
        assert_eq!(
            invite_status == StatusCode::OK,
            invite_succeeds,
            "{policy:?} invite registration status: {invite_status}"
        );

        let invite = state
            .invites
            .list_invites()
            .await
            .expect("list fixture invite")
            .into_iter()
            .find(|invite| invite.code.as_ref() == code.as_ref())
            .expect("fixture invite remains listed");
        assert_eq!(
            invite.used_at.is_some(),
            invite_consumed,
            "{policy:?} invite consumption"
        );
    }
}

// M2.9.12: `login` with correct password sets its cookie-only session.
#[apply(backends)]
#[tokio::test]
async fn login_correct_password_sets_session_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("eve", "password123", None),
        None,
        true,
    )
    .await;

    let (status, set_cookie, body) = post_server_fn_with_secure_flag(
        &state,
        &login_input("eve", "password123", None),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let cookie = set_cookie.expect("Set-Cookie header should be present on login");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");

    let cookie_token = token_from_set_cookie(&cookie);
    assert_body_carries_no_token("login", &body, &cookie_token);
}

// #591: login returns a complete marker (flash-free first-login chrome) without
// exposing its cookie credential.
#[apply(backends)]
#[tokio::test]
async fn login_returns_session_user_without_token(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    let (status, set_cookie, body) = post_server_fn_with_secure_flag(
        &state,
        &login_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let cookie = set_cookie.expect("Set-Cookie header should be present on login");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");
    let token = token_from_set_cookie(&cookie);
    assert_body_carries_no_token("login", &body, &token);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body.trim()).expect("valid login JSON body"),
        serde_json::json!({"Confirmed": {"username": "alice", "is_operator": false}}),
    );
}

#[apply(backends)]
#[tokio::test]
async fn login_unknown_user_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _, _) = post_server_fn_with_secure_flag(
        &state,
        &login_input("nobody", "password123", None),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn login_nested_request_maps_distinct_fields(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    let (status, set_cookie, _body) = post_server_fn_with_secure_flag(
        &state,
        &login_input("alice", "password123", Some("Issue 417 device")),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = session_token_of(set_cookie);
    let sessions = std::sync::Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.authenticate(transaction, &raw_token).await })
        })
        .await
        .unwrap();
    let record = storage::test_support::confirmed_for(outcome, "session authentication");
    assert_eq!(record.label, "Issue 417 device");
}

#[apply(backends)]
#[tokio::test]
async fn login_nested_request_without_label_uses_user_agent(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    let (status, set_cookie, _body) = post_server_fn_with_ua(
        &state,
        &login_input("alice", "password123", None),
        None,
        "Issue 417 browser",
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = session_token_of(set_cookie);
    let sessions = std::sync::Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.authenticate(transaction, &raw_token).await })
        })
        .await
        .unwrap();
    let record = storage::test_support::confirmed_for(outcome, "session authentication");
    assert_eq!(record.label, "Issue 417 browser");
}

#[apply(backends)]
#[tokio::test]
async fn login_rejects_whitespace_only_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    // A whitespace-only label is rejected at the typed-wire-arg decode
    // (SessionLabel's FromStr trims, then rejects empty) — it must not fall
    // through to the User-Agent branch and is a malformed client request.
    let (status, _, body) = post_server_fn_request_fixture_with_secure_flag::<web::auth::Login, _>(
        &state,
        &LoginDecodeFixture {
            username: "alice",
            password: "password123",
            label: Some("  "),
        },
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Decode fails before the handler body runs, so no session is minted.
    assert!(!body.contains("\"token\""), "token minted: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn login_rejects_overlong_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    // Past MAX_SESSION_LABEL_CHARS (255) the label is rejected at decode rather
    // than silently truncated, matching create_app_password_rejects_overlong_label.
    let overlong = "a".repeat(256);
    let (status, _, body) = post_server_fn_request_fixture_with_secure_flag::<web::auth::Login, _>(
        &state,
        &LoginDecodeFixture {
            username: "alice",
            password: "password123",
            label: Some(&overlong),
        },
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body.contains("\"token\""), "token minted: {body}");
}

// A long User-Agent is bounded by MAX_SESSION_LABEL_CHARS (255), the newtype's own
// cap, rather than a second hand-rolled 200-char cap in `login` (#685).
#[apply(backends)]
#[tokio::test]
async fn login_bounds_long_user_agent_at_session_label_cap(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    // 250 < 255, so this UA survives intact rather than being truncated.
    let long_ua = "a".repeat(250);

    let (status, set_cookie, _body) = post_server_fn_with_ua(
        &state,
        &login_input("alice", "password123", None),
        None,
        &long_ua,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = session_token_of(set_cookie);
    let sessions = std::sync::Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.authenticate(transaction, &raw_token).await })
        })
        .await
        .unwrap();
    let record = storage::test_support::confirmed_for(outcome, "session authentication");
    assert_eq!(record.label, "a".repeat(250).as_str());
}

// Past the cap, the UA is truncated (not rejected): it is an internally derived
// value, so it goes through the lossy door (ADR-0063 §2), unlike a submitted label.
#[apply(backends)]
#[tokio::test]
async fn login_truncates_user_agent_past_session_label_cap(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;

    let long_ua = "a".repeat(300);

    let (status, set_cookie, _body) = post_server_fn_with_ua(
        &state,
        &login_input("alice", "password123", None),
        None,
        &long_ua,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = session_token_of(set_cookie);
    let sessions = std::sync::Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.authenticate(transaction, &raw_token).await })
        })
        .await
        .unwrap();
    let record = storage::test_support::confirmed_for(outcome, "session authentication");
    assert_eq!(record.label.chars().count(), MAX_SESSION_LABEL_CHARS);
}

// M2.9.13: `login` with wrong password returns error.
#[apply(backends)]
#[tokio::test]
async fn login_wrong_password_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("frank", "correctpassword", None),
        None,
        true,
    )
    .await;

    let (status, _set_cookie, _body) = post_server_fn_with_secure_flag(
        &state,
        &login_input("frank", "wrongpassword", None),
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M2.9.14: `logout` revokes session and clears cookie.
#[apply(backends)]
#[tokio::test]
async fn logout_revokes_session_and_clears_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    // Create a user and a session directly, bypassing the HTTP layer so we
    // have the raw token without needing to parse the register response.
    let session = create_user_and_session(&state).await;

    let sessions_before = state.sessions.list_sessions(session.user_id).await.unwrap();
    assert_eq!(
        sessions_before.len(),
        1,
        "one session should exist before logout"
    );

    let cookie_header = session.cookie();
    let (status, set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::auth::Logout as ServerFn>::PATH,
        "",
        Some(&cookie_header),
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let clear_cookie = set_cookie.expect("Set-Cookie header should be present on logout");
    assert!(
        clear_cookie.contains("Max-Age=0"),
        "logout should clear cookie via Max-Age=0, got: {clear_cookie}"
    );

    let sessions_after = state.sessions.list_sessions(session.user_id).await.unwrap();
    assert!(
        sessions_after.is_empty(),
        "session should be revoked after logout"
    );
}

// register() with a username containing a space (invalid after lowercase parse) returns error.
#[apply(backends)]
#[tokio::test]
async fn register_invalid_username_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // "alice doe" lowercases to "alice doe" which fails Username parse
    // because Username only allows [a-z0-9_-]+.
    let (status, _set_cookie, _body) =
        post_server_fn_request_fixture_with_secure_flag::<web::registration::Register, _>(
            &state,
            &RegistrationDecodeFixture {
                username: "alice doe",
                password: "password123",
            },
            None,
            true,
        )
        .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "register with space in username should fail"
    );
}

// register() with a password shorter than 8 characters returns error and creates no user.
#[apply(backends)]
#[tokio::test]
async fn register_short_password_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _set_cookie, _body) =
        post_server_fn_request_fixture_with_secure_flag::<web::registration::Register, _>(
            &state,
            &RegistrationDecodeFixture {
                username: "alice",
                password: "short",
            },
            None,
            true,
        )
        .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "register with short password should fail"
    );

    let user = state
        .users
        .get_user_by_username(&"alice".parse::<Username>().expect("valid username"))
        .await
        .expect("database query failed");
    assert!(
        user.is_none(),
        "user should not be created when password is too short"
    );
}

// login() with a username containing a space (invalid parse) returns error.
#[apply(backends)]
#[tokio::test]
async fn login_nested_request_rejects_invalid_username_before_handler(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;
    let user = state
        .users
        .get_user_by_username(&"alice".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let sessions_before = state
        .sessions
        .list_sessions(user.user_id)
        .await
        .unwrap()
        .len();

    let (status, _set_cookie, body) =
        post_server_fn_request_fixture_with_secure_flag::<web::auth::Login, _>(
            &state,
            &LoginDecodeFixture {
                username: "alice doe",
                password: "password123",
                label: None,
            },
            None,
            true,
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("server_function"), "body: {body}");
    assert_eq!(
        state
            .sessions
            .list_sessions(user.user_id)
            .await
            .unwrap()
            .len(),
        sessions_before,
        "decode rejection must not create a session"
    );
}

#[apply(backends)]
#[tokio::test]
async fn login_nested_request_rejects_short_password_before_handler(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    post_server_fn_with_secure_flag(
        &state,
        &register_input("alice", "password123", None),
        None,
        true,
    )
    .await;
    let user = state
        .users
        .get_user_by_username(&"alice".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let sessions_before = state
        .sessions
        .list_sessions(user.user_id)
        .await
        .unwrap()
        .len();

    let (status, set_cookie, body) =
        post_server_fn_request_fixture_with_secure_flag::<web::auth::Login, _>(
            &state,
            &LoginDecodeFixture {
                username: "alice",
                password: "short",
                label: None,
            },
            None,
            true,
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(set_cookie.is_none(), "decode rejection minted a session");
    assert!(body.contains("server_function"), "body: {body}");
    assert_eq!(
        state
            .sessions
            .list_sessions(user.user_id)
            .await
            .unwrap()
            .len(),
        sessions_before,
        "decode rejection must not create a session"
    );
}

// logout() via Authorization: Bearer <token> revokes the session and clears the cookie.
#[apply(backends)]
#[tokio::test]
async fn logout_with_bearer_token_revokes_session(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Create a user and session directly so we have the raw token.
    let session = create_user_and_session(&state).await;

    let sessions_before = state
        .sessions
        .list_sessions(session.user_id)
        .await
        .expect("failed to list sessions");
    assert_eq!(
        sessions_before.len(),
        1,
        "one session should exist before logout"
    );

    // POST to /api/auth/logout with Bearer token instead of a cookie.
    let (status, set_cookie, _body) = post_form_with_bearer(
        &state,
        <web::auth::Logout as ServerFn>::PATH,
        "",
        session.token.as_ref(),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "logout with bearer token should succeed"
    );

    let clear_cookie = set_cookie.expect("Set-Cookie header should be present on logout");
    assert!(
        clear_cookie.contains("Max-Age=0"),
        "logout should clear cookie via Max-Age=0, got: {clear_cookie}"
    );

    let sessions_after = state
        .sessions
        .list_sessions(session.user_id)
        .await
        .expect("failed to list sessions after logout");
    assert!(
        sessions_after.is_empty(),
        "session should be revoked after bearer-token logout"
    );
}
#[apply(backends)]
#[tokio::test]
async fn explicit_auth_set_cookie_appends_to_handler_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let authorization = format!("Bearer {}", session.token);

    let response = post_form_with_credentials(
        &state,
        <web::auth::Logout as ServerFn>::PATH,
        "",
        Some(&session.cookie()),
        Some(&authorization),
        true,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.set_cookies.len(), 2);
    assert!(
        response.set_cookies.iter().all(|value| {
            value == "session=; HttpOnly; SameSite=Lax; Path=/; Secure; Max-Age=0"
        })
    );
}

#[apply(backends)]
#[tokio::test]
async fn optional_auth_endpoints_reject_explicit_auth_failure(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    for path in [
        <web::auth::GetSession as ServerFn>::PATH,
        <web::auth::Logout as ServerFn>::PATH,
        <web::backup::IsWarningVisible as ServerFn>::PATH,
        <web::site::IsBaseUrlWarningVisible as ServerFn>::PATH,
    ] {
        let response = post_form_with_credentials(
            &state,
            path,
            "",
            Some(&session.cookie()),
            Some("Malformed"),
            true,
        )
        .await;

        assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR, "{path}");
        assert!(response.set_cookies.is_empty(), "{path}");
    }
}

// Unauthenticated logout: no session cookie → skips revoke, still clears cookie.
#[apply(backends)]
#[tokio::test]
async fn logout_without_session_still_clears_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::auth::Logout as ServerFn>::PATH,
        "",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let clear_cookie = set_cookie.expect("Set-Cookie header should be present on logout");
    assert!(
        clear_cookie.contains("Max-Age=0"),
        "logout should clear cookie via Max-Age=0, got: {clear_cookie}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn debug_api_routes_exist(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Send a request with no body to /api/registration/register — if route exists we get
    // something other than 404 (probably a 400/422 for missing fields).
    let (status, _, _) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "",
        None,
        true,
    )
    .await;
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "/api/registration/register route not registered (got 404)"
    );
}

#[apply(backends)]
#[tokio::test]
async fn get_registration_policy_returns_correct_value(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let site_config = std::sync::Arc::clone(&state.site_config);
    storage::test_support::confirmed_for(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    site_config
                        .set_registration_policy(transaction, RegistrationPolicy::OperatorInvites)
                        .await
                })
            })
            .await
            .unwrap(),
        "set operator-invites registration policy",
    );

    // Server functions are POST by default.
    let (status, _, body) = post_form_with_secure_flag(
        &state,
        <web::registration::GetPolicy as ServerFn>::PATH,
        "",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "\"operator_invites\"");
}

// Shape B — `get_profile()` requires `auth::User`; both an invalid token and a
// missing token must fail extraction with INTERNAL_SERVER_ERROR. Identical
// setup + assertion; only the supplied cookie varies.
#[apply(backends_matrix)]
#[case::invalid_token(Some("session=invalidtoken"))]
#[case::missing(None)]
#[tokio::test]
async fn auth_user_extraction_fails(backend: Backend, #[case] cookie: Option<&str>) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _, _) = post_form_with_secure_flag(
        &state,
        <web::profile::Get as ServerFn>::PATH,
        "",
        cookie,
        true,
    )
    .await;

    // Leptos server functions return 500 for ServerFnError.
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn logout_clears_cookie_without_secure_attribute_when_disabled(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie_header = create_user_and_session(&state).await.cookie();
    let (status, set_cookie, _) = post_form_with_secure_flag(
        &state,
        <web::auth::Logout as ServerFn>::PATH,
        "",
        Some(&cookie_header),
        false,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let clear_cookie = set_cookie.expect("Set-Cookie header should be present");
    assert!(clear_cookie.contains("Max-Age=0"));
    assert!(!clear_cookie.contains("Secure"));
}

#[apply(backends)]
#[tokio::test]
async fn register_sets_cookie_without_secure_attribute_when_disabled(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, set_cookie, _) = post_server_fn_with_secure_flag(
        &state,
        &register_input("insecure", "password123", None),
        None,
        false,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let cookie = set_cookie.expect("Set-Cookie header should be present");
    assert!(cookie.contains("session="));
    assert!(!cookie.contains("Secure"));
}
