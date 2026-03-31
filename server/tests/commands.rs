use server::cli::StorageArgs;
use server::commands::cmd_init;
use server::storage::{open_database, DbConnectOptions};
use tempfile::TempDir;

fn storage_args(base: &TempDir) -> StorageArgs {
    let storage_path = base.path().join("storage");
    let db: DbConnectOptions = format!("sqlite:{}", base.path().join("jaunder.db").display())
        .parse()
        .unwrap();
    StorageArgs { storage_path, db }
}

#[tokio::test]
async fn cmd_init_on_fresh_dir_creates_structure_and_valid_db() {
    let base = TempDir::new().unwrap();
    let args = storage_args(&base);

    cmd_init(&args, false).await.unwrap();

    assert!(args.storage_path.is_dir());
    assert!(args.storage_path.join("media").is_dir());
    assert!(args.storage_path.join("backups").is_dir());
    open_database(&args.db).await.unwrap();
}

#[tokio::test]
async fn cmd_init_second_time_returns_error() {
    let base = TempDir::new().unwrap();
    let args = storage_args(&base);

    cmd_init(&args, false).await.unwrap();
    let result = cmd_init(&args, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn cmd_init_skip_if_exists_succeeds_on_already_initialized() {
    let base = TempDir::new().unwrap();
    let args = storage_args(&base);

    cmd_init(&args, false).await.unwrap();
    cmd_init(&args, true).await.unwrap();
}
