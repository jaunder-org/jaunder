use axum::http::StatusCode;
use common::{MutationOutcome, site::SiteIdentity};
use host::feed::FeedPath;
use server_fn::ServerFn;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_operator_and_session, create_user_and_session, make_app, post_form};
use storage::test_support::{
    Backend, SeedFeedCache, TestEnv, backends, confirmed, passkey_credential_fixture,
    write_scope_with_commit_acknowledgement_loss,
};

async fn enroll_passkey(env: &TestEnv, user_id: common::ids::UserId) {
    let passkeys = env.passkeys();
    let credential = passkey_credential_fixture();
    let label = "Web test Passkey".parse().expect("valid passkey label");
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
            .expect("passkey enrollment succeeds"),
    );
}

#[apply(backends)]
#[tokio::test]
async fn get_site_identity_requires_operator(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let anonymous_cookie = None;
    let member_cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (anon_status, anon_body) = post_form(
        app.clone(),
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        anonymous_cookie,
    )
    .await;
    assert_eq!(
        anon_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {anon_body}"
    );
    assert!(anon_body.contains("unauthorized"), "body: {anon_body}");

    let (member_status, member_body) = post_form(
        app.clone(),
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        Some(&member_cookie),
    )
    .await;
    assert_eq!(
        member_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {member_body}"
    );
    assert!(member_body.contains("unauthorized"), "body: {member_body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_site_identity_returns_defaults_when_unconfigured(#[case] backend: Backend) {
    let env = backend.setup().base_url(None).await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app.clone(),
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let identity: SiteIdentity = serde_json::from_str(&body).expect("json");
    assert_eq!(identity.title, "Jaunder");
    assert_eq!(identity.base_url, None);
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_round_trips_via_get(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let update_body = "request[title]=My+Blog&request[base_url]=https%3A%2F%2Fexample.com%2F";
    let (update_status, update_body_resp) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        update_body,
        Some(&cookie),
    )
    .await;
    assert_eq!(update_status, StatusCode::OK, "body: {update_body_resp}");

    let (get_status, get_body) = post_form(
        app.clone(),
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "body: {get_body}");
    let identity: SiteIdentity = serde_json::from_str(&get_body).expect("json");
    assert_eq!(identity.title, "My Blog");
    assert_eq!(identity.base_url.as_deref(), Some("https://example.com/"));
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_clears_tagline_when_omitted(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=My+Blog&request[tagline]=A+tagline&request[base_url]=https%3A%2F%2Fexample.com%2F",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // The direct-bound optional field dispatches `None` by omitting `tagline`,
    // which clears the aggregate identity value rather than retaining it.
    let (status, body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=My+Blog&request[base_url]=https%3A%2F%2Fexample.com%2F",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let (status, body) = post_form(
        app,
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let identity: SiteIdentity = serde_json::from_str(&body).unwrap();
    assert_eq!(identity.tagline, None);
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_preserves_loaded_tagline(#[case] backend: Backend) {
    let env = backend.setup().await;
    let site_config = env.site_config();
    let configured_tagline = "Existing tagline";
    confirmed(
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    site_config
                        .set(
                            transaction,
                            host::config_key::SiteConfigKey::SiteTagline,
                            configured_tagline,
                        )
                        .await
                })
            })
            .await
            .unwrap(),
    );
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=Renamed&request[tagline]=Existing+tagline&request[base_url]=https%3A%2F%2Fexample.com%2F",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let (status, body) = post_form(
        app,
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let identity: SiteIdentity = serde_json::from_str(&body).unwrap();
    assert_eq!(identity.title, "Renamed");
    assert_eq!(identity.tagline.as_deref(), Some(configured_tagline));
}

#[apply(backends)]
#[tokio::test]
async fn malformed_site_tagline_wire_rejection_preserves_the_aggregate_snapshot(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let (status, body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=Prior+Site&request[tagline]=Prior+tagline&request[base_url]=https%3A%2F%2Fprior.example.test%2F",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let cache_path: FeedPath = "/feed.rss".parse().expect("valid feed path");
    let cached = SeedFeedCache::new(cache_path.clone())
        .seed(env.feed_cache(), env.write_scope())
        .await;
    let before = env
        .publisher()
        .snapshot()
        .await
        .expect("publisher snapshot");

    for body in [
        format!(
            "request[title]=Changed+Site&request[tagline]={}&request[base_url]=https%3A%2F%2Fchanged.example.test%2F",
            "x".repeat(281)
        ),
        "request[title]=Changed+Site&request[tagline]=before%0Aafter&request[base_url]=https%3A%2F%2Fchanged.example.test%2F".to_owned(),
    ] {
        let (status, response) = post_form(
            app.clone(),
            <web::site::UpdateIdentity as ServerFn>::PATH,
            &body,
            Some(&cookie),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "typed wire validation must reject the malformed tagline: {response}"
        );
        assert!(
            response.contains("site tagline cannot"),
            "the client-validation wire rejection retains its validation detail: {response}"
        );
        assert_eq!(
            env.publisher().snapshot().await.expect("publisher snapshot"),
            before,
            "wire validation rejects before identity or publisher writes"
        );
        assert_eq!(
            env.feed_cache()
                .get(&cache_path)
                .await
                .expect("cached feed lookup"),
            Some(cached.clone()),
            "wire validation leaves cached feeds untouched"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn aggregate_identity_commit_acknowledgement_loss_reaches_the_wire(#[case] backend: Backend) {
    let env = backend.setup().await;
    let write_scope = write_scope_with_commit_acknowledgement_loss(&env.write_scope());
    let app = make_app!(&env, &env.base; override_write_scope = write_scope);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app,
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=Uncertain+Site&request[tagline]=Uncertain+tagline&request[base_url]=https%3A%2F%2Funcertain.example.test%2F",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(matches!(
        serde_json::from_str::<MutationOutcome<()>>(&body).expect("mutation outcome"),
        MutationOutcome::CommitIndeterminate(())
    ));
    let identity = env
        .site_config()
        .get_identity()
        .await
        .expect("identity read");
    assert_eq!(identity.title, "Uncertain Site");
    assert_eq!(identity.tagline.as_deref(), Some("Uncertain tagline"));
    assert_eq!(
        identity.base_url.as_deref(),
        Some("https://uncertain.example.test/")
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_rejects_empty_title(#[case] backend: Backend) {
    // A whitespace-only `title` fails at typed-arg decode — the validating serde
    // bridge for `SiteTitle` rejects an empty/whitespace-only value, a non-OK server
    // -function error rather than a specific in-body Validation message (ADR-0065).
    // The client's disable-until-valid gate keeps a real browser from reaching this;
    // a raw POST is the malformed-client path.
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=+++&request[base_url]=https%3A%2F%2Fexample.com",
        Some(&cookie),
    )
    .await;

    assert_ne!(status, StatusCode::OK, "empty title should fail: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_rejects_non_http_base_url(#[case] backend: Backend) {
    // A non-http(s) `base_url` fails at typed-arg decode — the validating serde
    // bridge for `Option<BaseUrl>` rejects it, a non-OK server-function error
    // rather than a specific Validation message (ADR-0065). The client's
    // disable-until-valid gate keeps a real browser from reaching this; a raw POST
    // is the malformed-client path.
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=My+Blog&request[base_url]=ftp%3A%2F%2Fexample.com",
        Some(&cookie),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "non-http base_url should fail: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_rejects_malformed_base_url(#[case] backend: Backend) {
    // A syntactically malformed `base_url` (not a URL at all) also fails at
    // typed-arg decode — same non-OK path as the non-http case (ADR-0065).
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=My+Blog&request[base_url]=not-a-url",
        Some(&cookie),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "malformed base_url should fail: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_omits_base_url_as_none(#[case] backend: Backend) {
    // Clearing the base URL is the dispatch-`None` path: the typed
    // `Option<BaseUrl>` wire arg is *omitted* (serde decodes a missing Option
    // field to `None`); an empty `base_url=` would instead fail to parse.
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (update_status, update_body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=My+Blog",
        Some(&cookie),
    )
    .await;
    assert_eq!(update_status, StatusCode::OK, "body: {update_body}");

    let (get_status, get_body) = post_form(
        app.clone(),
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "body: {get_body}");
    let identity: SiteIdentity = serde_json::from_str(&get_body).expect("json");
    assert_eq!(identity.base_url, None);
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_requires_operator(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let anonymous_cookie = None;
    let member_cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let body = "request[title]=My+Blog&request[base_url]=https%3A%2F%2Fexample.com";

    let (anon_status, anon_body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        body,
        anonymous_cookie,
    )
    .await;
    assert_eq!(
        anon_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {anon_body}"
    );
    assert!(anon_body.contains("unauthorized"), "body: {anon_body}");

    let (member_status, member_body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        body,
        Some(&member_cookie),
    )
    .await;
    assert_eq!(
        member_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {member_body}"
    );
    assert!(member_body.contains("unauthorized"), "body: {member_body}");
}

#[apply(backends)]
#[tokio::test]
async fn media_upload_capability_requires_operator(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let member_cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    for (path, body) in [
        (<web::site::GetMediaUploadsEnabled as ServerFn>::PATH, ""),
        (
            <web::site::UpdateMediaUploadsEnabled as ServerFn>::PATH,
            "uploads_enabled=false",
        ),
    ] {
        let (status, response) = post_form(app.clone(), path, body, Some(&member_cookie)).await;
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "body: {response}"
        );
        assert!(response.contains("unauthorized"), "body: {response}");
    }
}

#[apply(backends)]
#[tokio::test]
async fn media_upload_capability_defaults_enabled_and_round_trips(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (initial_status, initial_body) = post_form(
        app.clone(),
        <web::site::GetMediaUploadsEnabled as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(initial_status, StatusCode::OK, "body: {initial_body}");
    assert!(serde_json::from_str::<bool>(&initial_body).expect("boolean response"));

    for (body, expected) in [
        ("uploads_enabled=false", false),
        ("uploads_enabled=true", true),
    ] {
        let (update_status, update_body) = post_form(
            app.clone(),
            <web::site::UpdateMediaUploadsEnabled as ServerFn>::PATH,
            body,
            Some(&cookie),
        )
        .await;
        assert_eq!(update_status, StatusCode::OK, "body: {update_body}");

        let (get_status, get_body) = post_form(
            app.clone(),
            <web::site::GetMediaUploadsEnabled as ServerFn>::PATH,
            "",
            Some(&cookie),
        )
        .await;
        assert_eq!(get_status, StatusCode::OK, "body: {get_body}");
        assert_eq!(
            serde_json::from_str::<bool>(&get_body).expect("boolean response"),
            expected
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn media_upload_capability_and_site_identity_save_independently(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let identity_body =
        "request[title]=Independent+Site&request[base_url]=https%3A%2F%2Fexample.com";
    let (identity_status, identity_response) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        identity_body,
        Some(&cookie),
    )
    .await;
    assert_eq!(identity_status, StatusCode::OK, "body: {identity_response}");

    let (capability_status, capability_response) = post_form(
        app.clone(),
        <web::site::UpdateMediaUploadsEnabled as ServerFn>::PATH,
        "uploads_enabled=false",
        Some(&cookie),
    )
    .await;
    assert_eq!(
        capability_status,
        StatusCode::OK,
        "body: {capability_response}"
    );

    let (identity_status, identity_response) = post_form(
        app.clone(),
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(identity_status, StatusCode::OK, "body: {identity_response}");
    let identity: SiteIdentity = serde_json::from_str(&identity_response).expect("identity");
    assert_eq!(identity.title, "Independent Site");
    assert_eq!(identity.base_url.as_deref(), Some("https://example.com/"));

    let (identity_update_status, identity_update_response) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=Renamed+Site",
        Some(&cookie),
    )
    .await;
    assert_eq!(
        identity_update_status,
        StatusCode::OK,
        "body: {identity_update_response}"
    );

    let (capability_status, capability_response) = post_form(
        app.clone(),
        <web::site::GetMediaUploadsEnabled as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(
        capability_status,
        StatusCode::OK,
        "body: {capability_response}"
    );
    assert!(!serde_json::from_str::<bool>(&capability_response).expect("boolean response"));
}

// #575 base-URL warning banner endpoint. Mirrors web_backup.rs's
// `backup_warning_*` tests: a soft operator check (`Ok(false)`, never an error,
// for non-operators) over whether `SiteIdentity.base_url` is unset. The visible
// case explicitly omits the test fixture's default base URL.
#[apply(backends)]
#[tokio::test]
async fn base_url_warning_visible_for_operator_when_unset(#[case] backend: Backend) {
    let env = backend.setup().base_url(None).await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let (status, body) = post_form(
        app.clone(),
        <web::site::IsBaseUrlWarningVisible as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "true");
}

#[apply(backends)]
#[tokio::test]
async fn base_url_warning_hidden_when_base_url_configured(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let (up, up_body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=My+Blog&request[base_url]=https%3A%2F%2Fexample.com%2F",
        Some(&cookie),
    )
    .await;
    assert_eq!(up, StatusCode::OK, "body: {up_body}");
    let (status, body) = post_form(
        app.clone(),
        <web::site::IsBaseUrlWarningVisible as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}

#[apply(backends)]
#[tokio::test]
async fn base_url_warning_hidden_for_non_operator(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let (status, body) = post_form(
        app.clone(),
        <web::site::IsBaseUrlWarningVisible as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}

#[apply(backends)]
#[tokio::test]
async fn base_url_warning_hidden_without_authentication(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let (status, body) = post_form(
        app.clone(),
        <web::site::IsBaseUrlWarningVisible as ServerFn>::PATH,
        "",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}

// Covers the Err(non-Auth) branch of the endpoint body — required because #[server]
// bodies stay coverage-measured. Mirrors web_backup.rs's
// backup_warning_visible_propagates_storage_error_during_auth: close the pool after
// session creation so authenticate() returns Internal (not Auth) → 500.
#[apply(backends)]
#[tokio::test]
async fn base_url_warning_propagates_storage_error_during_auth(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    env.base.close_pool().await;
    let (status, _body) = post_form(
        app.clone(),
        <web::site::IsBaseUrlWarningVisible as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_rejects_replacing_or_clearing_enrolled_rp_host(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let operator = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    enroll_passkey(&env, operator.user_id).await;
    let cookie = operator.cookie();

    let (replacement_status, replacement_body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=My+Blog&request[base_url]=https%3A%2F%2Freplacement.example.test%2F",
        Some(&cookie),
    )
    .await;
    assert_eq!(
        replacement_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {replacement_body}"
    );
    assert!(
        replacement_body.contains("conflict"),
        "RP-host lock must retain its conflict wire error: {replacement_body}"
    );

    let (clear_status, clear_body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=My+Blog",
        Some(&cookie),
    )
    .await;
    assert_eq!(
        clear_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {clear_body}"
    );
    assert!(
        clear_body.contains("conflict"),
        "RP-host lock must retain its conflict wire error: {clear_body}"
    );
    let (get_status, get_body) = post_form(
        app.clone(),
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "body: {get_body}");
    let identity: SiteIdentity = serde_json::from_str(&get_body).expect("identity");
    assert_eq!(
        identity.base_url.as_deref(),
        Some("https://example.com/"),
        "rejected mutations preserve the enrolled RP host"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_allows_scheme_and_port_changes_for_enrolled_rp_host(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let operator = create_operator_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    enroll_passkey(&env, operator.user_id).await;
    let cookie = operator.cookie();

    let (update_status, update_body) = post_form(
        app.clone(),
        <web::site::UpdateIdentity as ServerFn>::PATH,
        "request[title]=My+Blog&request[base_url]=http%3A%2F%2Fexample.com%3A8080%2F",
        Some(&cookie),
    )
    .await;
    assert_eq!(update_status, StatusCode::OK, "body: {update_body}");

    let (get_status, get_body) = post_form(
        app.clone(),
        <web::site::GetIdentity as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "body: {get_body}");
    let identity: SiteIdentity = serde_json::from_str(&get_body).expect("identity");
    assert_eq!(
        identity.base_url.as_deref(),
        Some("http://example.com:8080/"),
        "the enrolled RP host permits an HTTPS-to-HTTP scheme and port change"
    );
}
