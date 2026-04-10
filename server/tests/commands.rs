use std::net::SocketAddr;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use jaunder::cli::StorageArgs;
use jaunder::commands::{
    cmd_create_pg_db, cmd_init, cmd_serve, cmd_smtp_test, cmd_user_create, cmd_user_invite,
};
use jaunder::password::Password;
use jaunder::storage::{open_database, open_existing_database, DbConnectOptions};
use jaunder::username::Username;
use leptos::prelude::LeptosOptions;
use sqlx::Connection;
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

#[tokio::test]
async fn cmd_init_fails_on_invalid_path() {
    let base = TempDir::new().unwrap();
    let args = storage_args(&base);
    // Create a file where the storage directory should be, so create_dir fails
    // with something other than AlreadyExists (actually it might be AlreadyExists or NotADirectory).
    // Actually, let's use a path in a non-existent directory.
    let args = StorageArgs {
        storage_path: base.path().join("nonexistent").join("storage"),
        db: args.db,
    };

    let result = cmd_init(&args, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn cmd_create_pg_db_rejects_non_postgres_urls() {
    let err = cmd_create_pg_db(
        "sqlite:/tmp/bootstrap.db",
        "postgres://jaunder@localhost/jaunder",
        "secret",
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("PostgreSQL URL"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn cmd_create_pg_db_provisions_role_and_database() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let role_name = format!("jaunder_role_{suffix}");
    let db_name = format!("jaunder_db_{suffix}");

    let bootstrap = "postgres://postgres@127.0.0.1:55432/postgres".to_owned();
    let app = format!("postgres://{role_name}@127.0.0.1:55432/{db_name}");

    let mut admin_conn =
        sqlx::PgConnection::connect("postgres://postgres@127.0.0.1:55432/postgres")
            .await
            .unwrap();
    sqlx::query(&format!("DROP DATABASE IF EXISTS \"{db_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
    sqlx::query(&format!("DROP ROLE IF EXISTS \"{role_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();

    cmd_create_pg_db(&bootstrap, &app, "bootstrap-secret")
        .await
        .unwrap();

    let role_exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(&role_name)
            .fetch_one(&mut admin_conn)
            .await
            .unwrap();
    assert!(role_exists);

    let owner = sqlx::query_scalar::<_, Option<String>>(
        "SELECT owner.rolname
         FROM pg_database db
         JOIN pg_roles owner ON owner.oid = db.datdba
         WHERE db.datname = $1",
    )
    .bind(&db_name)
    .fetch_optional(&mut admin_conn)
    .await
    .unwrap()
    .flatten();
    assert_eq!(owner.as_deref(), Some(role_name.as_str()));

    let storage_path = TempDir::new().unwrap();
    let args = StorageArgs {
        storage_path: storage_path.path().join("storage"),
        db: app.parse().unwrap(),
    };
    cmd_init(&args, false).await.unwrap();

    sqlx::query(&format!("DROP DATABASE IF EXISTS \"{db_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
    sqlx::query(&format!("DROP ROLE IF EXISTS \"{role_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn cmd_create_pg_db_fails_if_role_already_exists() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let role_name = format!("jaunder_role_{suffix}");
    let db_name = format!("jaunder_db_{suffix}");

    let bootstrap = "postgres://postgres@127.0.0.1:55432/postgres".to_owned();
    let app = format!("postgres://{role_name}@127.0.0.1:55432/{db_name}");

    let mut admin_conn =
        sqlx::PgConnection::connect("postgres://postgres@127.0.0.1:55432/postgres")
            .await
            .unwrap();
    sqlx::query(&format!("DROP DATABASE IF EXISTS \"{db_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
    sqlx::query(&format!("DROP ROLE IF EXISTS \"{role_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE ROLE \"{role_name}\" LOGIN"))
        .execute(&mut admin_conn)
        .await
        .unwrap();

    let err = cmd_create_pg_db(&bootstrap, &app, "bootstrap-secret")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already exists"));

    let db_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
    )
    .bind(&db_name)
    .fetch_one(&mut admin_conn)
    .await
    .unwrap();
    assert!(!db_exists);

    sqlx::query(&format!("DROP ROLE IF EXISTS \"{role_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
}

// M1.5.4: cmd_serve fails with an appropriate error when the storage path has
// not been initialized.
#[tokio::test]
async fn cmd_serve_fails_when_not_initialized() {
    let base = TempDir::new().unwrap();
    let args = storage_args(&base);
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let result = cmd_serve(&args, bind, true).await;
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
    let router = jaunder::create_router(leptos_options, db, true);

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
        let _ = cmd_serve(&storage, bind, true).await;
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

#[tokio::test]
async fn cmd_user_invite_default_expires_in() {
    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);
    cmd_init(&args, false).await.expect("init");

    cmd_user_invite(&args, None).await.expect("user invite");

    let state = open_existing_database(&args.db).await.expect("open db");
    let invites = state.invites.list_invites().await.expect("list invites");
    assert_eq!(invites.len(), 1, "exactly one invite should exist");
}

#[tokio::test]
async fn cmd_user_invite_too_large_expires_in_returns_error() {
    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);
    cmd_init(&args, false).await.expect("init");

    // u64::MAX is definitely too large for i64
    let result = cmd_user_invite(&args, Some(u64::MAX)).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("too large"));
}

#[tokio::test]
async fn cmd_smtp_test_fails_when_not_initialized() {
    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);

    let result = cmd_smtp_test(&args, "alice@example.com").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("jaunder init"),
        "expected error to mention 'jaunder init', got: {msg}"
    );
}

#[tokio::test]
async fn cmd_smtp_test_fails_when_smtp_not_configured() {
    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);
    cmd_init(&args, false).await.expect("init");

    let result = cmd_smtp_test(&args, "alice@example.com").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("SMTP is not configured"),
        "expected 'SMTP is not configured', got: {msg}"
    );
}

#[tokio::test]
async fn cmd_smtp_test_succeeds_with_mock_server() {
    let server = maik::MockServer::builder()
        .no_verify_credentials()
        .assert_after_n_emails(1)
        .build();
    server.start();

    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);
    cmd_init(&args, false).await.expect("init");

    let state = open_existing_database(&args.db).await.expect("open db");
    state
        .site_config
        .set("smtp.host", &server.host().to_string())
        .await
        .expect("set host");
    state
        .site_config
        .set("smtp.port", &server.port().to_string())
        .await
        .expect("set port");
    state
        .site_config
        .set("smtp.tls_mode", "plain")
        .await
        .expect("set tls_mode");
    state
        .site_config
        .set("smtp.sender", "noreply@example.com")
        .await
        .expect("set sender");
    state
        .site_config
        .set("smtp.username", "user")
        .await
        .expect("set username");
    state
        .site_config
        .set("smtp.password", "password")
        .await
        .expect("set password");

    cmd_smtp_test(&args, "alice@example.com")
        .await
        .expect("smtp test should succeed");

    let assertion = maik::MailAssertion::new().recipients_are(["alice@example.com"]);
    assert!(server.assert(assertion));
}

#[tokio::test]
async fn cmd_smtp_test_fails_on_invalid_to_address() {
    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);
    cmd_init(&args, false).await.expect("init");

    // Configure SMTP so we get past the "not configured" check.
    let state = open_existing_database(&args.db).await.expect("open db");
    state
        .site_config
        .set("smtp.host", "mail.example.com")
        .await
        .expect("set smtp.host");
    state
        .site_config
        .set("smtp.port", "587")
        .await
        .expect("set smtp.port");
    state
        .site_config
        .set("smtp.tls_mode", "plain")
        .await
        .expect("set smtp.tls_mode");
    state
        .site_config
        .set("smtp.sender", "noreply@example.com")
        .await
        .expect("set smtp.sender");

    let result = cmd_smtp_test(&args, "not-an-email").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Invalid email address"),
        "expected 'Invalid email address', got: {msg}"
    );
}
