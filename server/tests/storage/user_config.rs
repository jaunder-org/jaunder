use host::config_key::UserConfigKey;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, backends};

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
