use common::MutationOutcome;
use host::config_key::UserConfigKey;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, backends};

async fn assert_confirmed_write(
    state: &storage::AppState,
    operation: impl for<'scope> FnOnce(
        &'scope mut storage::WriteTransaction,
    ) -> futures_util::future::BoxFuture<'scope, sqlx::Result<()>>,
) {
    assert!(matches!(
        state.write_scope.run(operation).await.unwrap(),
        MutationOutcome::Confirmed(())
    ));
}

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
    let config_for_write = state.user_config.clone();
    let config_for_read = state.user_config.clone();
    assert_confirmed_write(state, move |transaction| {
        Box::pin(async move {
            config_for_write
                .set(
                    transaction,
                    user_id,
                    UserConfigKey::DefaultPostFormat,
                    "markdown",
                )
                .await
        })
    })
    .await;
    let val = config_for_read
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
    let config_for_write = state.user_config.clone();
    let config_for_read = state.user_config.clone();
    assert_confirmed_write(state, move |transaction| {
        Box::pin(async move {
            config_for_write
                .set(
                    transaction,
                    user_id,
                    UserConfigKey::DefaultPostFormat,
                    "org",
                )
                .await
        })
    })
    .await;
    let val = config_for_read
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
    let config_for_initial_write = state.user_config.clone();
    assert_confirmed_write(state, move |transaction| {
        Box::pin(async move {
            config_for_initial_write
                .set(
                    transaction,
                    user_id,
                    UserConfigKey::DefaultPostFormat,
                    "markdown",
                )
                .await
        })
    })
    .await;
    let config_for_overwrite = state.user_config.clone();
    let config_for_read = state.user_config.clone();
    assert_confirmed_write(state, move |transaction| {
        Box::pin(async move {
            config_for_overwrite
                .set(
                    transaction,
                    user_id,
                    UserConfigKey::DefaultPostFormat,
                    "org",
                )
                .await
        })
    })
    .await;
    let val = config_for_read
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
    let config_for_initial_write = state.user_config.clone();
    assert_confirmed_write(state, move |transaction| {
        Box::pin(async move {
            config_for_initial_write
                .set(
                    transaction,
                    user_id,
                    UserConfigKey::DefaultPostFormat,
                    "org",
                )
                .await
        })
    })
    .await;
    let config_for_delete = state.user_config.clone();
    let config_for_read = state.user_config.clone();
    assert_confirmed_write(state, move |transaction| {
        Box::pin(async move {
            config_for_delete
                .delete(transaction, user_id, UserConfigKey::DefaultPostFormat)
                .await
        })
    })
    .await;
    let val = config_for_read
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert_eq!(val, None);
}

#[apply(backends)]
#[tokio::test]
async fn user_config_delete_nonexistent_is_ok(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;
    let config_for_delete = state.user_config.clone();
    assert_confirmed_write(state, move |transaction| {
        Box::pin(async move {
            config_for_delete
                .delete(transaction, user_id, UserConfigKey::DefaultPostFormat)
                .await
        })
    })
    .await;
}
