use std::net::SocketAddr;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use leptos::prelude::LeptosOptions;
use server::cli::StorageArgs;
use server::commands::{cmd_init, cmd_serve, cmd_user_create, cmd_user_invite};
use server::password::Password;
use server::storage::{open_database, open_existing_database, DbConnectOptions};
use server::username::Username;
use tempfile::TempDir;
use tower::ServiceExt;

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

// M1.5.4: cmd_serve fails with an appropriate error when the storage path has
// not been initialized.
#[tokio::test]
async fn cmd_serve_fails_when_not_initialized() {
    let base = TempDir::new().unwrap();
    let args = storage_args(&base);
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let result = cmd_serve(&args, bind).await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("jaunder init"),
        "expected error to mention 'jaunder init', got: {msg}"
    );
}

// M1.5.5: after cmd_init, the server responds to a simple health-check request.
// Uses open_existing_database (the path cmd_serve takes) to build the router.
#[tokio::test]
async fn after_init_server_responds_to_health_check() {
    let base = TempDir::new().unwrap();
    let args = storage_args(&base);

    cmd_init(&args, false).await.unwrap();

    let db = open_existing_database(&args.db).await.unwrap();
    let leptos_options = LeptosOptions::builder().output_name("test").build();
    let router = server::create_router(leptos_options, db);

    let response = router
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// Covers cmd_serve's success path: open DB, build options, create router, bind,
// and start serving.
#[tokio::test]
async fn cmd_serve_starts_and_accepts_connections() {
    let base = TempDir::new().unwrap();
    let args = storage_args(&base);
    cmd_init(&args, false).await.unwrap();

    // Pre-bind port 0 to let the OS assign a free port, then release it so
    // cmd_serve can bind to the same address.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bind = probe.local_addr().unwrap();
    drop(probe);

    let storage_path = args.storage_path.clone();
    let db = args.db.clone();
    let task = tokio::spawn(async move {
        let storage = StorageArgs { storage_path, db };
        let _ = cmd_serve(&storage, bind).await;
    });

    // Poll until the server accepts TCP connections (up to 1 s).
    let mut connected = false;
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(bind).await.is_ok() {
            connected = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    task.abort();
    assert!(connected, "server did not start within 1s");
}

#[tokio::test]
async fn cmd_user_create_creates_retrievable_user() {
    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);
    cmd_init(&args, false).await.expect("init");

    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    cmd_user_create(&args, &username, Some(password), None)
        .await
        .expect("user create");

    let state = open_existing_database(&args.db).await.expect("open db");
    let user = state
        .users
        .get_user_by_username(&username)
        .await
        .expect("db query");
    assert!(user.is_some(), "user should exist after creation");
    assert_eq!(user.expect("user present").username.as_str(), "alice");
}

#[tokio::test]
async fn cmd_user_invite_creates_retrievable_invite() {
    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);
    cmd_init(&args, false).await.expect("init");

    cmd_user_invite(&args, Some(48)).await.expect("user invite");

    let state = open_existing_database(&args.db).await.expect("open db");
    let invites = state.invites.list_invites().await.expect("list invites");
    assert_eq!(invites.len(), 1, "exactly one invite should exist");
}
