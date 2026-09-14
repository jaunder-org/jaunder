use std::sync::Arc;

use axum::http::StatusCode;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use common::MutationOutcome;
use host::config_key::SiteConfigKey;
use rstest::*;
use rstest_reuse::*;
use serde_json::json;
use server_fn::ServerFn;
use storage::test_support::{
    Backend, backends, confirmed, passkey_credential_fixture,
    write_scope_with_commit_acknowledgement_loss,
};
use storage::{PasskeyCredentialId, RawPasskeyCeremonyHandle};

use crate::helpers::{
    TestHttpResponse, create_session_for, create_user_and_session, delete_site_config, make_app,
    post_form, post_form_with_credentials, post_json, post_json_with_credentials, set_site_config,
};

async fn enroll(env: &storage::test_support::TestEnv, user_id: common::ids::UserId) -> String {
    let credential = passkey_credential_fixture();
    let id = PasskeyCredentialId::from_credential(&credential)
        .expose_to_browser()
        .to_owned();
    let label = "HTTP test Passkey".parse().expect("valid passkey label");
    let passkeys = env.passkeys();
    confirmed(
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    passkeys
                        .insert_credential(transaction, user_id, &label, &credential)
                        .await
                })
            })
            .await
            .expect("enroll passkey fixture"),
    );
    id
}

async fn credential_ids(
    env: &storage::test_support::TestEnv,
    user_id: common::ids::UserId,
) -> Vec<String> {
    env.passkeys()
        .list_credentials(user_id)
        .await
        .expect("list passkey credentials")
        .into_iter()
        .map(|credential| credential.id.expose_to_browser().to_owned())
        .collect()
}

async fn authentication_start(app: axum::Router) -> web::passkeys::AuthenticationStart {
    let (status, body) = post_form(
        app,
        <web::passkeys::StartAuthentication as ServerFn>::PATH,
        "",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "authentication start body: {body}");
    match serde_json::from_str::<MutationOutcome<web::passkeys::AuthenticationStart>>(&body)
        .expect("decode authentication start")
    {
        MutationOutcome::Confirmed(start) | MutationOutcome::CommitIndeterminate(start) => start,
    }
}

async fn user_handle(env: &storage::test_support::TestEnv, user_id: common::ids::UserId) -> String {
    let handle = env
        .passkeys()
        .user_handle(user_id)
        .await
        .expect("load user handle")
        .expect("fixture user has a passkey handle")
        .adapter_handle()
        .expect("stored user handle is valid");
    URL_SAFE_NO_PAD.encode(handle.as_bytes())
}

/// Identification precedes assertion verification, so these protocol-shaped responses reach
/// user/credential lookup without pretending that fixture-only tests can forge a signature.
fn identified_assertion(credential_id: &str, user_handle: &str) -> serde_json::Value {
    json!({
        "id": credential_id,
        "rawId": credential_id,
        "type": "public-key",
        "response": {
            "authenticatorData": URL_SAFE_NO_PAD.encode([0_u8; 37]),
            "clientDataJSON": URL_SAFE_NO_PAD.encode(
                br#"{"type":"webauthn.get","challenge":"","origin":"http://localhost"}"#
            ),
            "signature": URL_SAFE_NO_PAD.encode([0_u8; 64]),
            "userHandle": user_handle,
        },
        "clientExtensionResults": {},
    })
}

async fn start_registration(app: axum::Router, cookie: &str) -> web::passkeys::RegistrationStart {
    let (status, body) = post_form(
        app,
        <web::passkeys::StartRegistration as ServerFn>::PATH,
        "label=HTTP+ceremony&password=password123",
        Some(cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "registration start body: {body}");
    match serde_json::from_str::<MutationOutcome<web::passkeys::RegistrationStart>>(&body)
        .expect("decode registration start")
    {
        MutationOutcome::Confirmed(start) | MutationOutcome::CommitIndeterminate(start) => start,
    }
}

/// Availability is deployment configuration, not an authenticated account signal.
#[apply(backends)]
#[tokio::test]
async fn availability_is_public_and_returns_json(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) = post_form(
        app.clone(),
        <web::passkeys::Availability as ServerFn>::PATH,
        "",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "availability body: {body}");
    assert!(serde_json::from_str::<bool>(&body).expect("availability is a JSON boolean"));

    assert!(
        delete_site_config(
            Arc::clone(&env.site_config()),
            env.write_scope(),
            SiteConfigKey::SiteBaseUrl,
        )
        .await
        .expect("delete test base URL"),
        "test base URL exists",
    );

    let (status, body) = post_form(
        app,
        <web::passkeys::Availability as ServerFn>::PATH,
        "",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "availability without base URL body: {body}"
    );
    assert!(
        !serde_json::from_str::<bool>(&body).expect("availability without base URL is JSON"),
        "availability is false without a base URL",
    );
}

/// Authenticated owners can retrieve only the browser-safe credential fields.
#[apply(backends)]
#[tokio::test]
async fn list_returns_owner_visible_credential_fields(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let credential_id = enroll(&env, owner.user_id).await;
    let fixture = env
        .passkeys()
        .list_credentials(owner.user_id)
        .await
        .expect("load fixture credential")
        .pop()
        .expect("fixture credential");

    let (status, body) = post_form(
        app,
        <web::passkeys::List as ServerFn>::PATH,
        "",
        Some(&owner.cookie()),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "passkey list body: {body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).expect("passkey list is JSON"),
        json!([{
            "id": credential_id,
            "label": "HTTP test Passkey",
            "created_at": fixture.created_at.to_string(),
            "last_used_at": fixture.last_used_at.map(|time| time.to_string()),
        }]),
    );
}

/// Invalid wire-only identifiers are rejected before a ceremony can be claimed or created.
#[apply(backends)]
#[tokio::test]
async fn malformed_passkey_payloads_are_rejected_at_http_boundary(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let user = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    for (path, body, cookie) in [
        (
            <web::passkeys::StartRegistration as ServerFn>::PATH,
            "label=+++&password=password123",
            Some(user.cookie()),
        ),
        (
            <web::passkeys::Delete as ServerFn>::PATH,
            "credential_id=not-base64&password=password123",
            Some(user.cookie()),
        ),
    ] {
        let (status, response) = post_form(app.clone(), path, body, cookie.as_deref()).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} malformed-input status: {response}"
        );
        assert!(
            !response.is_empty(),
            "{path} rejection must carry an HTTP error body"
        );
    }
    for (path, cookie, expected_status, expected_message) in [
        (
            <web::passkeys::FinishRegistration as ServerFn>::PATH,
            Some(user.cookie()),
            StatusCode::BAD_REQUEST,
            "invalid ceremony handle",
        ),
        (
            <web::passkeys::FinishAuthentication as ServerFn>::PATH,
            None,
            StatusCode::INTERNAL_SERVER_ERROR,
            "authentication failed",
        ),
    ] {
        let (status, response) = post_json(
            app.clone(),
            path,
            json!({"handle": "not-a-ceremony", "response": {}}),
            cookie.as_deref(),
        )
        .await;
        assert_eq!(
            status, expected_status,
            "{path} malformed-input status: {response}"
        );
        assert!(
            response.contains(expected_message),
            "{path} rejection body: {response}"
        );
    }
}

/// Cookie-only management routes deliberately reject Authorization rather than falling back to a
/// simultaneously supplied browser cookie.
#[apply(backends)]
#[tokio::test]
async fn cookie_only_passkey_routes_reject_explicit_authorization(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let user = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let credential_id = enroll(&env, user.user_id).await;
    let cookie = user.cookie();

    let requests = [
        (
            <web::passkeys::StartRegistration as ServerFn>::PATH,
            "label=Phone&password=password123".to_owned(),
        ),
        (<web::passkeys::List as ServerFn>::PATH, String::new()),
        (
            <web::passkeys::Delete as ServerFn>::PATH,
            format!("credential_id={credential_id}&password=password123"),
        ),
    ];
    for (path, body) in requests {
        for authorization in [
            format!("Bearer {}", user.token),
            format!("Basic {}", user.token),
        ] {
            let TestHttpResponse {
                status,
                set_cookies,
                body: response,
            } = post_form_with_credentials(
                app.clone(),
                path,
                body.clone(),
                Some(&cookie),
                Some(&authorization),
                false,
            )
            .await;
            assert_ne!(
                status,
                StatusCode::OK,
                "{path} accepted {authorization}: {response}"
            );
            assert!(
                set_cookies.is_empty(),
                "{path} altered cookie state for {authorization}"
            );
        }
    }

    let registration = start_registration(app.clone(), &cookie).await;
    for authorization in [
        format!("Bearer {}", user.token),
        format!("Basic {}", user.token),
    ] {
        let TestHttpResponse {
            status,
            set_cookies,
            body: response,
        } = post_json_with_credentials(
            app.clone(),
            <web::passkeys::FinishRegistration as ServerFn>::PATH,
            json!({"handle": registration.handle.clone(), "response": {}}),
            Some(&cookie),
            Some(&authorization),
            false,
        )
        .await;
        assert_ne!(
            status,
            StatusCode::OK,
            "finish registration accepted {authorization}: {response}"
        );
        assert!(
            set_cookies.is_empty(),
            "finish registration altered cookie state for {authorization}"
        );
    }

    assert_eq!(credential_ids(&env, user.user_id).await, [credential_id]);
}

#[apply(backends)]
#[tokio::test]
async fn registration_wrong_password_is_rejected_without_creating_a_credential(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let user = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let (status, body) = post_form(
        app,
        <web::passkeys::StartRegistration as ServerFn>::PATH,
        "label=Phone&password=wrong-password",
        Some(&user.cookie()),
    )
    .await;

    assert_ne!(status, StatusCode::OK, "wrong password response: {body}");
    assert!(credential_ids(&env, user.user_id).await.is_empty());
}

/// A registration claim is bound to the session that created it and remains consumed when that
/// binding check fails. Changing the RP configuration after start is likewise a durable failure.
#[apply(backends)]
#[tokio::test]
async fn registration_finish_consumes_tampered_bound_or_config_changed_ceremonies(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base; secure_cookies = true);
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let other = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let bound = start_registration(app.clone(), &owner.cookie()).await;
    let changed = start_registration(app.clone(), &owner.cookie()).await;
    set_site_config(
        Arc::clone(&env.site_config()),
        env.write_scope(),
        SiteConfigKey::SiteBaseUrl,
        "https://changed-passkey-rp.example/",
    )
    .await
    .expect("change test RP configuration");

    for (handle, cookie) in [
        (bound.handle, other.cookie()),
        (changed.handle, owner.cookie()),
    ] {
        let body = json!({"handle": handle.clone(), "response": {}});
        for _ in 0..2 {
            let TestHttpResponse {
                status,
                set_cookies,
                body: response,
            } = post_json_with_credentials(
                app.clone(),
                <web::passkeys::FinishRegistration as ServerFn>::PATH,
                body.clone(),
                Some(&cookie),
                None,
                true,
            )
            .await;
            assert_ne!(
                status,
                StatusCode::OK,
                "invalid registration finish: {response}"
            );
            assert!(
                set_cookies.is_empty(),
                "registration finish must not alter session cookie"
            );
            assert!(
                response.contains("invalid ceremony"),
                "failure and replay must stay neutral: {response}"
            );
            assert!(
                !response.contains(&handle),
                "failure leaked ceremony handle: {response}"
            );
        }
    }
    assert!(credential_ids(&env, owner.user_id).await.is_empty());
}
/// A lost acknowledgement while claiming a ceremony is neutral to the browser: it sets no cookie
/// and the durable claim prevents a later retry from reaching verification.
#[apply(backends)]
#[tokio::test]
async fn commit_indeterminate_ceremony_claim_is_neutral_and_durably_consumed(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let normal_app = make_app!(&env, &env.base);
    let user = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let registration = start_registration(normal_app.clone(), &user.cookie()).await;
    let (auth_start_status, auth_start_body) = post_form(
        normal_app.clone(),
        <web::passkeys::StartAuthentication as ServerFn>::PATH,
        "",
        None,
    )
    .await;
    assert_eq!(
        auth_start_status,
        StatusCode::OK,
        "authentication start: {auth_start_body}"
    );
    let authentication = match serde_json::from_str::<
        MutationOutcome<web::passkeys::AuthenticationStart>,
    >(&auth_start_body)
    .expect("decode authentication start")
    {
        MutationOutcome::Confirmed(start) | MutationOutcome::CommitIndeterminate(start) => start,
    };
    let acknowledgement_lost = write_scope_with_commit_acknowledgement_loss(&env.write_scope());
    let indeterminate_app = make_app!(&env, &env.base; override_write_scope = acknowledgement_lost);

    for (path, body, cookie, neutral_message) in [
        (
            <web::passkeys::FinishRegistration as ServerFn>::PATH,
            json!({"handle": registration.handle.clone(), "response": {}}),
            Some(user.cookie()),
            "invalid ceremony",
        ),
        (
            <web::passkeys::FinishAuthentication as ServerFn>::PATH,
            json!({"handle": authentication.handle.clone(), "response": {}}),
            None,
            "authentication failed",
        ),
    ] {
        let response = post_json_with_credentials(
            indeterminate_app.clone(),
            path,
            body.clone(),
            cookie.as_deref(),
            None,
            false,
        )
        .await;
        assert_ne!(
            response.status,
            StatusCode::OK,
            "claim loss response: {}",
            response.body
        );
        assert!(
            response.set_cookies.is_empty(),
            "claim loss must not set a cookie"
        );
        assert!(
            response.body.contains(neutral_message),
            "claim loss must be neutral: {}",
            response.body
        );

        let replay = post_json_with_credentials(
            normal_app.clone(),
            path,
            body,
            cookie.as_deref(),
            None,
            false,
        )
        .await;
        assert_ne!(
            replay.status,
            StatusCode::OK,
            "replay response: {}",
            replay.body
        );
        assert!(
            replay.set_cookies.is_empty(),
            "replay must not set a cookie"
        );
        assert!(
            replay.body.contains(neutral_message),
            "replay must be neutral: {}",
            replay.body
        );
    }
    assert!(credential_ids(&env, user.user_id).await.is_empty());
    assert!(
        env.sessions()
            .list_sessions(user.user_id)
            .await
            .expect("list sessions")
            .iter()
            .all(|session| session.token_hash
                == host::token::hash(&user.token).expect("hash session"))
    );
}
/// Authentication failure remains neutral and never establishes a browser session. A signed
/// assertion cannot be synthesized from the persisted adapter fixture: it has no private key or
/// authenticator counter state. This still covers the HTTP neutral-error and replay boundaries.
#[apply(backends)]
#[tokio::test]
async fn authentication_failure_is_neutral_and_sets_no_cookie(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base; secure_cookies = true);

    let (start_status, start_body) = post_form(
        app.clone(),
        <web::passkeys::StartAuthentication as ServerFn>::PATH,
        "",
        None,
    )
    .await;
    assert_eq!(start_status, StatusCode::OK, "start body: {start_body}");
    let started: MutationOutcome<web::passkeys::AuthenticationStart> =
        serde_json::from_str(&start_body).expect("decode authentication start");
    let handle = match started {
        MutationOutcome::Confirmed(started) | MutationOutcome::CommitIndeterminate(started) => {
            started.handle
        }
    };
    let body = json!({"handle": handle.clone(), "response": {}});

    for _ in 0..2 {
        let TestHttpResponse {
            status,
            set_cookies,
            body: response,
        } = post_json_with_credentials(
            app.clone(),
            <web::passkeys::FinishAuthentication as ServerFn>::PATH,
            body.clone(),
            None,
            None,
            true,
        )
        .await;
        assert_ne!(
            status,
            StatusCode::OK,
            "invalid assertion response: {response}"
        );
        assert!(
            set_cookies.is_empty(),
            "failed authentication must not set a cookie"
        );
        assert!(
            response.contains("authentication failed"),
            "failure must remain neutral: {response}"
        );
        assert!(
            !response.contains(&handle),
            "failure leaked ceremony handle: {response}"
        );
    }
}

/// A syntactically valid but unissued handle must be indistinguishable from every other failed
/// ceremony and must not create a browser session.
#[apply(backends)]
#[tokio::test]
async fn unknown_ceremony_handles_are_neutral_and_cookie_free(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base; secure_cookies = true);
    let user = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    for (path, cookie, neutral_message) in [
        (
            <web::passkeys::FinishRegistration as ServerFn>::PATH,
            Some(user.cookie()),
            "invalid ceremony",
        ),
        (
            <web::passkeys::FinishAuthentication as ServerFn>::PATH,
            None,
            "authentication failed",
        ),
    ] {
        let handle = RawPasskeyCeremonyHandle::generate()
            .expose_to_browser()
            .to_owned();
        let response = post_json_with_credentials(
            app.clone(),
            path,
            json!({"handle": &handle, "response": {}}),
            cookie.as_deref(),
            None,
            true,
        )
        .await;
        assert_ne!(
            response.status,
            StatusCode::OK,
            "unknown handle: {}",
            response.body
        );
        assert!(
            response.set_cookies.is_empty(),
            "unknown handle set a cookie"
        );
        assert!(
            response.body.contains(neutral_message),
            "unknown handle response: {}",
            response.body
        );
        assert!(
            !response.body.contains(&handle),
            "unknown handle leaked in response"
        );
    }
}
/// Every public authentication rejection is indistinguishable, including wire malformedness and
/// post-identification identities. The fixture has no private key, so lookup cases use a
/// protocol-shaped assertion: `identify` can read its credential and user handle, while signature
/// verification is deliberately unreachable for these rejected pairings.
#[apply(backends)]
#[tokio::test]
async fn authentication_invalidities_share_one_neutral_http_failure(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base; secure_cookies = true);
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let other = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let owner_credential = enroll(&env, owner.user_id).await;
    let owner_handle = user_handle(&env, owner.user_id).await;
    let other_handle = user_handle(&env, other.user_id).await;
    let owner_credentials = credential_ids(&env, owner.user_id).await;
    let other_credentials = credential_ids(&env, other.user_id).await;
    let owner_sessions_before = env
        .sessions()
        .list_sessions(owner.user_id)
        .await
        .expect("list owner sessions")
        .into_iter()
        .map(|session| session.token_hash)
        .collect::<Vec<_>>();
    let other_sessions_before = env
        .sessions()
        .list_sessions(other.user_id)
        .await
        .expect("list other sessions")
        .into_iter()
        .map(|session| session.token_hash)
        .collect::<Vec<_>>();

    let cases = vec![
        ("malformed handle", "not-a-ceremony".to_owned(), json!({})),
        (
            "unknown handle",
            RawPasskeyCeremonyHandle::generate()
                .expose_to_browser()
                .to_owned(),
            json!({}),
        ),
        (
            "unknown user handle",
            authentication_start(app.clone()).await.handle,
            identified_assertion(&owner_credential, &URL_SAFE_NO_PAD.encode([0_u8; 16])),
        ),
        (
            "unknown credential ID",
            authentication_start(app.clone()).await.handle,
            identified_assertion(&URL_SAFE_NO_PAD.encode([0xff_u8; 32]), &owner_handle),
        ),
        (
            "cross-user handle and credential",
            authentication_start(app.clone()).await.handle,
            identified_assertion(&owner_credential, &other_handle),
        ),
    ];
    let mut neutral = None;
    for (name, handle, assertion) in cases {
        let response = post_json_with_credentials(
            app.clone(),
            <web::passkeys::FinishAuthentication as ServerFn>::PATH,
            json!({"handle": handle, "response": assertion}),
            None,
            None,
            true,
        )
        .await;
        assert_ne!(
            response.status,
            StatusCode::OK,
            "{name} unexpectedly authenticated: {}",
            response.body
        );
        assert!(
            response.set_cookies.is_empty(),
            "{name} set a session cookie"
        );
        let rejection = (response.status, response.body);
        if let Some(expected) = &neutral {
            assert_eq!(&rejection, expected, "{name} leaked a distinct failure");
        } else {
            neutral = Some(rejection);
        }
    }

    let replay_handle = authentication_start(app.clone()).await.handle;
    let first = post_json_with_credentials(
        app.clone(),
        <web::passkeys::FinishAuthentication as ServerFn>::PATH,
        json!({"handle": replay_handle, "response": {}}),
        None,
        None,
        true,
    )
    .await;
    let replay = post_json_with_credentials(
        app.clone(),
        <web::passkeys::FinishAuthentication as ServerFn>::PATH,
        json!({"handle": replay_handle, "response": {}}),
        None,
        None,
        true,
    )
    .await;
    for (name, response) in [("failed assertion", first), ("replayed handle", replay)] {
        assert_eq!(
            (response.status, response.body),
            neutral.clone().expect("capture neutral rejection"),
            "{name} leaked a distinct failure"
        );
        assert!(
            response.set_cookies.is_empty(),
            "{name} set a session cookie"
        );
    }
    assert_eq!(credential_ids(&env, owner.user_id).await, owner_credentials);
    assert_eq!(credential_ids(&env, other.user_id).await, other_credentials);
    assert_eq!(
        env.sessions()
            .list_sessions(owner.user_id)
            .await
            .expect("list owner sessions")
            .into_iter()
            .map(|session| session.token_hash)
            .collect::<Vec<_>>(),
        owner_sessions_before
    );
    assert_eq!(
        env.sessions()
            .list_sessions(other.user_id)
            .await
            .expect("list other sessions")
            .into_iter()
            .map(|session| session.token_hash)
            .collect::<Vec<_>>(),
        other_sessions_before
    );
}

/// Configuration may change after a ceremony begins. A now-insecure RP must not expose its
/// deployment state through authentication finish.
#[apply(backends)]
#[tokio::test]
async fn authentication_config_drift_is_neutral(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base; secure_cookies = true);
    let malformed = post_json_with_credentials(
        app.clone(),
        <web::passkeys::FinishAuthentication as ServerFn>::PATH,
        json!({"handle": "not-a-ceremony", "response": {}}),
        None,
        None,
        true,
    )
    .await;
    let started = authentication_start(app.clone()).await;
    set_site_config(
        Arc::clone(&env.site_config()),
        env.write_scope(),
        SiteConfigKey::SiteBaseUrl,
        "http://insecure-passkey-rp.example/",
    )
    .await
    .expect("set insecure RP configuration");
    let drifted = post_json_with_credentials(
        app,
        <web::passkeys::FinishAuthentication as ServerFn>::PATH,
        json!({"handle": started.handle, "response": {}}),
        None,
        None,
        true,
    )
    .await;
    assert_eq!(
        (drifted.status, drifted.body),
        (malformed.status, malformed.body),
        "configuration drift leaked a distinct authentication failure"
    );
    assert!(drifted.set_cookies.is_empty());
    assert!(malformed.set_cookies.is_empty());
}
#[apply(backends)]
#[tokio::test]
async fn delete_rejects_invalid_or_foreign_requests_without_changing_credentials_or_sessions(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let revoked = create_session_for(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
        owner.user_id,
    )
    .await;
    let foreign = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let credential_id = enroll(&env, owner.user_id).await;
    let sessions = Arc::clone(&env.sessions());
    let owner_token_hash = host::token::hash(&owner.token).expect("hash current token");
    confirmed(
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    sessions
                        .revoke_all_for_user_except(transaction, owner.user_id, &owner_token_hash)
                        .await
                })
            })
            .await
            .expect("revoke fixture session"),
    );
    let session_hashes_before = env
        .sessions()
        .list_sessions(owner.user_id)
        .await
        .expect("list sessions")
        .into_iter()
        .map(|session| session.token_hash)
        .collect::<Vec<_>>();
    let credentials_before = credential_ids(&env, owner.user_id).await;

    let mut neutral_rejection = None;
    let cases = [
        (
            Some(owner.cookie()),
            None,
            format!("credential_id={credential_id}&password=wrongpass123"),
        ),
        (
            None,
            None,
            format!("credential_id={credential_id}&password=password123"),
        ),
        (
            Some(owner.cookie()),
            Some(format!("Bearer {}", owner.token)),
            format!("credential_id={credential_id}&password=password123"),
        ),
        (
            Some(revoked.cookie()),
            None,
            format!("credential_id={credential_id}&password=password123"),
        ),
        (
            Some(foreign.cookie()),
            None,
            format!("credential_id={credential_id}&password=password123"),
        ),
    ];
    for (cookie, authorization, body) in cases {
        let response = post_form_with_credentials(
            app.clone(),
            <web::passkeys::Delete as ServerFn>::PATH,
            body,
            cookie.as_deref(),
            authorization.as_deref(),
            false,
        )
        .await;
        assert_ne!(
            response.status,
            StatusCode::OK,
            "delete unexpectedly succeeded: {}",
            response.body
        );
        let rejection = (response.status, response.body.clone());
        if let Some(expected) = &neutral_rejection {
            assert_eq!(
                &rejection, expected,
                "all rejected deletions expose one neutral response"
            );
        } else {
            neutral_rejection = Some(rejection);
        }
        assert!(response.set_cookies.is_empty());
        assert_eq!(
            credential_ids(&env, owner.user_id).await,
            credentials_before
        );
        let session_hashes_after = env
            .sessions()
            .list_sessions(owner.user_id)
            .await
            .expect("list sessions")
            .into_iter()
            .map(|session| session.token_hash)
            .collect::<Vec<_>>();
        assert_eq!(session_hashes_after, session_hashes_before);
    }
}

#[apply(backends)]
#[tokio::test]
async fn delete_preserves_current_session_and_revokes_siblings(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let owner = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let sibling = create_session_for(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
        owner.user_id,
    )
    .await;
    let credential_id = enroll(&env, owner.user_id).await;

    let (status, body) = post_form(
        app,
        <web::passkeys::Delete as ServerFn>::PATH,
        format!("credential_id={credential_id}&password=password123"),
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    let _: MutationOutcome<()> = serde_json::from_str(&body).expect("decode delete result");
    assert!(credential_ids(&env, owner.user_id).await.is_empty());

    let sessions = env
        .sessions()
        .list_sessions(owner.user_id)
        .await
        .expect("list sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].token_hash,
        host::token::hash(&owner.token).expect("hash current token")
    );
    assert_ne!(
        sessions[0].token_hash,
        host::token::hash(&sibling.token).expect("hash sibling token")
    );
}
