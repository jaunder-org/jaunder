use std::sync::Arc;

use host::config_key::SiteConfigKey;
use storage::{SiteConfigStorage, WriteScope};

/// Persists a site-config fixture through the same caller-owned write boundary as production.
pub async fn set_site_config(
    site_config: Arc<dyn SiteConfigStorage>,
    write_scope: WriteScope,
    key: SiteConfigKey,
    value: &str,
) -> anyhow::Result<()> {
    let value = value.to_owned();
    storage::test_support::confirmed(
        write_scope
            .run(move |transaction| {
                Box::pin(async move { site_config.set(transaction, key, &value).await })
            })
            .await?,
    );
    Ok(())
}

/// Deletes a site-config fixture through the same caller-owned write boundary as production.
pub async fn delete_site_config(
    site_config: Arc<dyn SiteConfigStorage>,
    write_scope: WriteScope,
    key: SiteConfigKey,
) -> anyhow::Result<bool> {
    Ok(storage::test_support::confirmed(
        write_scope
            .run(move |transaction| {
                Box::pin(async move { site_config.delete(transaction, key).await })
            })
            .await?,
    ))
}
