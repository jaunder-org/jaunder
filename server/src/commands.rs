use std::{io, net::SocketAddr};

use crate::cli::StorageArgs;
use crate::storage::{init_storage, open_database, open_existing_database};

pub async fn cmd_init(storage: &StorageArgs, skip_if_exists: bool) -> anyhow::Result<()> {
    match init_storage(&storage.storage_path) {
        Ok(()) => {}
        Err(e) if skip_if_exists && e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    open_database(&storage.db).await?;
    println!(
        "Initialized: storage={} db={}",
        storage.storage_path.display(),
        storage.db,
    );
    Ok(())
}

pub async fn cmd_serve(storage: &StorageArgs, bind: SocketAddr) -> anyhow::Result<()> {
    let db = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let conf = leptos::config::get_configuration(None)
        .map_err(|e| anyhow::anyhow!("failed to read Leptos configuration: {e}"))?;
    let mut leptos_options = conf.leptos_options;
    leptos_options.site_addr = bind;

    let router = crate::create_router(leptos_options, db);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
