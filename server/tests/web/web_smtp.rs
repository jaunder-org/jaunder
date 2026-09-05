use axum::http::StatusCode;
use common::smtp_password::ProfferedSmtpPassword;
use host::config_key::SiteConfigKey;
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;

use crate::helpers::{
    create_operator_and_session, create_user_and_session, delete_site_config, post_form,
    post_server_fn, post_server_fn_request_fixture, set_site_config,
};
use storage::test_support::{Backend, TestEnv, backends};

fn update_request(
    enabled: bool,
    authentication_enabled: bool,
    username: Option<&str>,
    password: Option<&str>,
) -> web::smtp::UpdateSettingsRequest {
    web::smtp::UpdateSettingsRequest {
        enabled,
        host: enabled.then(|| "relay.example.com".parse().unwrap()),
        port: "2525".parse().unwrap(),
        tls_mode: "tls".parse().unwrap(),
        sender: "Jaunder <mail@example.com>".parse().unwrap(),
        authentication_enabled,
        username: username.map(|value| value.parse().unwrap()),
        password: password.map(|value| value.parse::<ProfferedSmtpPassword>().unwrap()),
    }
}

async fn raw(state: &std::sync::Arc<storage::AppState>, key: SiteConfigKey) -> Option<String> {
    state.site_config.get_raw(key).await.unwrap()
}

#[apply(backends)]
#[tokio::test]
async fn both_smtp_functions_require_operator_authorization(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let member_cookie = create_user_and_session(&state).await.cookie();

    for cookie in [None, Some(member_cookie.as_str())] {
        let (status, body) = post_form(
            &state,
            <web::smtp::GetSettings as ServerFn>::PATH,
            "",
            cookie,
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
        assert!(body.contains("unauthorized"), "{body}");

        let input = web::smtp::UpdateSettings {
            request: update_request(false, false, None, None),
        };
        let (status, body) = post_server_fn(&state, &input, cookie).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
        assert!(body.contains("unauthorized"), "{body}");
    }
}

#[apply(backends)]
#[tokio::test]
async fn disabled_settings_return_exact_secret_free_defaults(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state).await.cookie();
    let (status, body) = post_form(
        &state,
        <web::smtp::GetSettings as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let settings: web::smtp::Settings = serde_json::from_str(&body).unwrap();
    assert_eq!(settings, web::smtp::Settings::default());
    assert_eq!(settings.port.to_string(), "587");
    assert_eq!(settings.tls_mode.to_string(), "starttls");
    assert_eq!(settings.sender.to_string(), "Jaunder <noreply@localhost>");
    assert!(!body.contains("ProfferedSmtpPassword"), "{body}");
    assert!(!body.contains("SmtpPassword"), "{body}");
}

#[apply(backends)]
#[tokio::test]
async fn configured_and_legacy_reads_expose_only_password_presence(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    set_site_config(&state, SiteConfigKey::SmtpHost, "legacy-relay")
        .await
        .unwrap();
    set_site_config(&state, SiteConfigKey::SmtpPassword, "never-return-this")
        .await
        .unwrap();
    let cookie = create_operator_and_session(&state).await.cookie();

    let (status, body) = post_form(
        &state,
        <web::smtp::GetSettings as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let settings: web::smtp::Settings = serde_json::from_str(&body).unwrap();
    assert!(settings.enabled);
    assert!(settings.authentication_enabled);
    assert!(settings.username.is_none());
    assert!(settings.password_configured);
    assert!(!body.contains("never-return-this"), "{body}");
    let object = serde_json::from_str::<serde_json::Value>(&body).unwrap();
    assert!(object.get("password").is_none(), "{body}");

    set_site_config(&state, SiteConfigKey::SmtpUsername, "relay-user")
        .await
        .unwrap();
    let (status, body) = post_form(
        &state,
        <web::smtp::GetSettings as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let settings: web::smtp::Settings = serde_json::from_str(&body).unwrap();
    assert_eq!(settings.username.as_deref(), Some("relay-user"));
    assert!(settings.password_configured);
    assert!(settings.authentication_enabled);
    assert!(!body.contains("never-return-this"), "{body}");

    delete_site_config(&state, SiteConfigKey::SmtpPassword)
        .await
        .unwrap();
    let (status, body) = post_form(
        &state,
        <web::smtp::GetSettings as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let settings: web::smtp::Settings = serde_json::from_str(&body).unwrap();
    assert!(settings.authentication_enabled);
    assert!(!settings.password_configured);
    assert_eq!(settings.username.as_deref(), Some("relay-user"));

    delete_site_config(&state, SiteConfigKey::SmtpUsername)
        .await
        .unwrap();
    let (status, body) = post_form(
        &state,
        <web::smtp::GetSettings as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let settings: web::smtp::Settings = serde_json::from_str(&body).unwrap();
    assert!(!settings.authentication_enabled);
    assert!(!settings.password_configured);
    assert!(settings.username.is_none());
}

#[derive(serde::Serialize)]
struct InvalidUpdateFixture<'a> {
    enabled: bool,
    host: &'a str,
    port: &'a str,
    tls_mode: &'a str,
    sender: &'a str,
    authentication_enabled: bool,
    username: &'a str,
    password: &'a str,
}

#[apply(backends)]
#[tokio::test]
async fn malformed_typed_request_is_rejected_before_mutation(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state).await.cookie();
    let fixture = InvalidUpdateFixture {
        enabled: true,
        host: "",
        port: "not-a-port",
        tls_mode: "ssl",
        sender: "not-a-mailbox",
        authentication_enabled: true,
        username: "",
        password: "replacement",
    };

    let (status, body) = post_server_fn_request_fixture::<web::smtp::UpdateSettings, _>(
        &state,
        &fixture,
        Some(&cookie),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "{body}");
    assert_eq!(raw(&state, SiteConfigKey::SmtpHost).await, None);
}

#[apply(backends)]
#[tokio::test]
async fn operator_can_replace_keep_clear_and_fully_disable_atomically(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state).await.cookie();

    let replace = web::smtp::UpdateSettings {
        request: update_request(true, true, Some("relay-user"), Some("first-secret")),
    };
    let (status, body) = post_server_fn(&state, &replace, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpHost).await.as_deref(),
        Some("relay.example.com")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpPort).await.as_deref(),
        Some("2525")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpTlsMode).await.as_deref(),
        Some("tls")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpSender).await.as_deref(),
        Some("Jaunder <mail@example.com>")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpUsername).await.as_deref(),
        Some("relay-user")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpPassword).await.as_deref(),
        Some("first-secret")
    );
    assert!(!body.contains("first-secret"), "{body}");

    let keep = web::smtp::UpdateSettings {
        request: update_request(true, true, Some("renamed-user"), None),
    };
    let (status, body) = post_server_fn(&state, &keep, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpUsername).await.as_deref(),
        Some("renamed-user")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpPassword).await.as_deref(),
        Some("first-secret")
    );

    let replace = web::smtp::UpdateSettings {
        request: update_request(true, true, Some("renamed-user"), Some("second-secret")),
    };
    let (status, body) = post_server_fn(&state, &replace, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpPassword).await.as_deref(),
        Some("second-secret")
    );
    assert!(!body.contains("second-secret"), "{body}");

    let clear = web::smtp::UpdateSettings {
        request: update_request(true, false, None, None),
    };
    let (status, body) = post_server_fn(&state, &clear, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(raw(&state, SiteConfigKey::SmtpUsername).await, None);
    assert_eq!(raw(&state, SiteConfigKey::SmtpPassword).await, None);
    assert!(raw(&state, SiteConfigKey::SmtpHost).await.is_some());

    let disable = web::smtp::UpdateSettings {
        request: update_request(false, false, None, None),
    };
    let (status, body) = post_server_fn(&state, &disable, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for key in [
        SiteConfigKey::SmtpHost,
        SiteConfigKey::SmtpPort,
        SiteConfigKey::SmtpTlsMode,
        SiteConfigKey::SmtpSender,
        SiteConfigKey::SmtpUsername,
        SiteConfigKey::SmtpPassword,
    ] {
        assert_eq!(raw(&state, key).await, None, "{key:?}");
    }
}

#[apply(backends)]
#[tokio::test]
async fn stale_password_keep_conflicts_and_rolls_back_every_field(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    for (key, value) in [
        (SiteConfigKey::SmtpHost, "old-relay"),
        (SiteConfigKey::SmtpPort, "587"),
        (SiteConfigKey::SmtpTlsMode, "starttls"),
        (SiteConfigKey::SmtpSender, "old@example.com"),
        (SiteConfigKey::SmtpUsername, "old-user"),
    ] {
        set_site_config(&state, key, value).await.unwrap();
    }
    let cookie = create_operator_and_session(&state).await.cookie();
    let keep = web::smtp::UpdateSettings {
        request: update_request(true, true, Some("new-user"), None),
    };

    let (status, body) = post_server_fn(&state, &keep, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(body.contains("SMTP authentication changed"), "{body}");
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpHost).await.as_deref(),
        Some("old-relay")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpPort).await.as_deref(),
        Some("587")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpTlsMode).await.as_deref(),
        Some("starttls")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpSender).await.as_deref(),
        Some("old@example.com")
    );
    assert_eq!(
        raw(&state, SiteConfigKey::SmtpUsername).await.as_deref(),
        Some("old-user")
    );
    assert_eq!(raw(&state, SiteConfigKey::SmtpPassword).await, None);
}

#[apply(backends)]
#[tokio::test]
async fn contradictory_secret_request_is_valueless_and_writes_nothing(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state).await.cookie();
    let input = web::smtp::UpdateSettings {
        request: update_request(false, false, None, Some("must-not-leak")),
    };

    let (status, body) = post_server_fn(&state, &input, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        body.contains("cannot be supplied while SMTP is disabled"),
        "{body}"
    );
    assert!(!body.contains("must-not-leak"), "{body}");
    assert_eq!(raw(&state, SiteConfigKey::SmtpHost).await, None);
    assert_eq!(raw(&state, SiteConfigKey::SmtpPassword).await, None);

    let input = web::smtp::UpdateSettings {
        request: update_request(true, false, None, Some("also-must-not-leak")),
    };
    let (status, body) = post_server_fn(&state, &input, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        body.contains("cannot be supplied while authentication is disabled"),
        "{body}"
    );
    assert!(!body.contains("also-must-not-leak"), "{body}");
    assert_eq!(raw(&state, SiteConfigKey::SmtpHost).await, None);
    assert_eq!(raw(&state, SiteConfigKey::SmtpPassword).await, None);
}
