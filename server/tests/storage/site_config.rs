use host::config_key::SiteConfigKey;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, TestEnv, backends};

// --- build_mailer tests ---
#[apply(backends)]
#[tokio::test]
async fn build_mailer_returns_noop_when_smtp_not_configured(#[case] backend: Backend) {
    let env = backend.setup().await;
    let mailer = jaunder::mailer::build_mailer(env.state.site_config.as_ref(), None)
        .await
        .expect("absent SMTP selects the no-op mailer");

    let msg = common::mailer::EmailMessage {
        from: None,
        to: vec!["alice@example.com".parse().unwrap()],
        subject: "Test".to_string(),
        body_text: "Hello".to_string(),
    };
    let result = mailer.send_email(&msg).await;
    assert!(
        matches!(result, Err(common::mailer::MailError::NotConfigured)),
        "expected NotConfigured, got {result:?}"
    );
}

/// The primitives are reached only through the closed [`SiteConfigKey`] registry
/// (#687): a key that is not in the registry cannot be named at all, so the
/// round-trip is stated over real keys.
#[apply(backends)]
#[tokio::test]
async fn site_config_round_trips_through_typed_keys(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteTitle, "My Site")
        .await
        .unwrap();
    assert_eq!(
        state
            .site_config
            .get_raw(SiteConfigKey::SiteTitle)
            .await
            .unwrap()
            .as_deref(),
        Some("My Site")
    );
    assert_eq!(
        state
            .site_config
            .get_raw(SiteConfigKey::FeedsMinDays)
            .await
            .unwrap(),
        None
    );
    assert!(
        state
            .site_config
            .delete(SiteConfigKey::SiteTitle)
            .await
            .unwrap()
    );
    assert_eq!(
        state
            .site_config
            .get_raw(SiteConfigKey::SiteTitle)
            .await
            .unwrap(),
        None
    );
}

#[apply(backends)]
#[tokio::test]
async fn site_config_operations(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let value = state.site_config.get_raw(SiteConfigKey::SiteBaseUrl).await;
    match value {
        Ok(None) => {}
        other => panic!("Expected Ok(None), got {other:?}"),
    }

    state
        .site_config
        .set(SiteConfigKey::SiteTitle, "test.value")
        .await
        .expect("set failed");
    let value = state.site_config.get_raw(SiteConfigKey::SiteTitle).await;
    match value {
        Ok(Some(v)) => assert_eq!(v, "test.value"),
        other => panic!("Expected Ok(Some), got {other:?}"),
    }

    state
        .site_config
        .set(SiteConfigKey::SiteTitle, "updated.value")
        .await
        .expect("set update failed");
    let value = state.site_config.get_raw(SiteConfigKey::SiteTitle).await;
    match value {
        Ok(Some(v)) => assert_eq!(v, "updated.value"),
        other => panic!("Expected updated value, got {other:?}"),
    }
}
