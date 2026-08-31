use std::sync::Arc;

use host::config_key::SiteConfigKey;
use storage::AppState;

/// Persists a site-config fixture through the same caller-owned write boundary as production.
pub async fn set_site_config(
    state: &Arc<AppState>,
    key: SiteConfigKey,
    value: &str,
) -> anyhow::Result<()> {
    let site_config = Arc::clone(&state.site_config);
    let value = value.to_owned();
    storage::test_support::confirmed(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { site_config.set(transaction, key, &value).await })
            })
            .await?,
    );
    Ok(())
}

/// Deletes a site-config fixture through the same caller-owned write boundary as production.
pub async fn delete_site_config(state: &Arc<AppState>, key: SiteConfigKey) -> anyhow::Result<bool> {
    let site_config = Arc::clone(&state.site_config);
    Ok(storage::test_support::confirmed(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { site_config.delete(transaction, key).await })
            })
            .await?,
    ))
}
