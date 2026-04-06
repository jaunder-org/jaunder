use std::{io, net::SocketAddr};

use crate::cli::StorageArgs;
use crate::password::Password;
use crate::storage::{init_storage, open_database, open_existing_database};
use crate::username::Username;

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

pub async fn cmd_user_create(
    storage: &StorageArgs,
    username: &Username,
    password: Option<Password>,
    display_name: Option<&str>,
) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let password = match password {
        Some(p) => p,
        None => {
            let p1 = rpassword::prompt_password("Password: ")?;
            let p2 = rpassword::prompt_password("Confirm password: ")?;
            if p1 != p2 {
                return Err(anyhow::anyhow!("passwords do not match"));
            }
            p1.parse::<Password>().map_err(|e| anyhow::anyhow!("{e}"))?
        }
    };

    let user_id = state
        .users
        .create_user(username, &password, display_name)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Created user '{}' with id {user_id}", username);
    Ok(())
}

pub async fn cmd_user_invite(storage: &StorageArgs, expires_in: Option<u64>) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let hours_u64 = expires_in.unwrap_or(168);
    let hours = i64::try_from(hours_u64)
        .map_err(|_| anyhow::anyhow!("--expires-in value {hours_u64} is too large"))?;
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(hours);

    let code = state.invites.create_invite(expires_at).await?;
    println!("{code}");
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
