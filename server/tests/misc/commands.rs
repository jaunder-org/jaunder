use std::fmt::Write as _;
use std::net::SocketAddr;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::password::Password;
use common::test_support::parse_invite_ttl_hours;
use common::username::Username;
use jaunder::cli::StorageArgs;
use jaunder::commands::{
    cmd_app_password_create, cmd_backup, cmd_create_pg_db, cmd_init, cmd_restore, cmd_serve,
    cmd_smtp_test, cmd_user_create, cmd_user_invite, prepare_server,
};
use leptos::prelude::LeptosOptions;
use storage::{open_database, open_existing_database, BackupMode};
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::misc::backup_fixture::{
    assert_backup_fixture_restored, assert_target_unmodified, populate_backup_fixture,
};
use storage::test_support::{
    backends, nonexistent_postgres_url, noop_mailer, sqlite_url, unique_postgres_url, Backend,
    PostgresDbGuard,
};

async fn storage_args(backend: Backend, base: &TempDir) -> (StorageArgs, Option<PostgresDbGuard>) {
    let storage_path = base.path().join("storage");
    let (db, guard) = match backend {
        Backend::Sqlite => (sqlite_url(base), None),
        Backend::Postgres => {
            let (db, guard) = unique_postgres_url().await;
            (db, Some(guard))
        }
    };
    (StorageArgs { storage_path, db }, guard)
}

fn uninitialized_storage_args(backend: Backend, base: &TempDir) -> StorageArgs {
    let storage_path = base.path().join("storage");
    let db = match backend {
        Backend::Sqlite => sqlite_url(base),
        Backend::Postgres => nonexistent_postgres_url(),
    };
    StorageArgs { storage_path, db }
}

#[apply(backends)]
#[tokio::test]
async fn cmd_init_on_fresh_dir_creates_structure_and_valid_db(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;

    cmd_init(&args, false).await.unwrap();

    assert!(args.storage_path.is_dir());
    assert!(args.storage_path.join("media").is_dir());
    assert!(args.storage_path.join("backups").is_dir());
    open_database(&args.db).await.unwrap();
}

#[apply(backends)]
#[tokio::test]
async fn cmd_init_second_time_returns_error(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;

    cmd_init(&args, false).await.unwrap();
    let result = cmd_init(&args, false).await;
    assert!(result.is_err());
}

#[apply(backends)]
#[tokio::test]
async fn cmd_init_skip_if_exists_succeeds_on_already_initialized(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;

    cmd_init(&args, false).await.unwrap();
    cmd_init(&args, true).await.unwrap();
}

#[apply(backends)]
#[tokio::test]
async fn cmd_init_fails_on_invalid_path(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;
    // A storage path under a non-existent parent makes directory creation fail.
    let args = StorageArgs {
        storage_path: base.path().join("nonexistent").join("storage"),
        db: args.db,
    };

    let result = cmd_init(&args, false).await;
    assert!(result.is_err());
}

// guard:no-backend — rejects a non-Postgres bootstrap URL before any DB work; a
// pure URL-validation error path that never provisions or connects.
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

// M1.5.4: cmd_serve fails with an appropriate error when the storage path has
// not been initialized.
#[apply(backends)]
#[tokio::test]
async fn cmd_serve_fails_when_not_initialized(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let args = uninitialized_storage_args(backend, &base);
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let result = cmd_serve(&args, bind, true, None).await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("jaunder init"),
        "expected error to mention 'jaunder init', got: {msg}"
    );
}

// M1.5.5: after cmd_init, the server responds to a simple health-check request.
// Uses open_existing_database (the path cmd_serve takes) to build the router.
#[apply(backends)]
#[tokio::test]
async fn after_init_server_responds_to_health_check(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;

    cmd_init(&args, false).await.unwrap();

    let db = open_existing_database(&args.db).await.unwrap();
    let leptos_options = LeptosOptions::builder().output_name("test").build();
    let router = jaunder::create_router(
        leptos_options,
        db,
        noop_mailer(),
        true,
        args.storage_path.clone(),
    );

    // Wrap the request in a LocalSet so Leptos's SSR rendering (which spawns
    // resource fetchers via `tokio::task::spawn_local` for `<Suspense>`)
    // doesn't panic with "spawn_local called from outside of a task::LocalSet".
    // The production serving path provides this via leptos-axum's setup; bare
    // `router.oneshot` on the default multi-thread runtime does not.
    let local = tokio::task::LocalSet::new();
    let response = local
        .run_until(async move {
            router
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap()
        })
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

// Covers cmd_serve's setup path deterministically: open DB, start the backup +
// feed workers, build the router, and bind the listener. The blocking
// `axum::serve` loop is the only line cmd_serve adds on top, so we exercise the
// setup via `prepare_server` directly rather than spawning cmd_serve and
// aborting it mid-flight (whose async-region coverage was nondeterministic —
// jaunder-uox1).
#[apply(backends)]
#[tokio::test]
async fn prepare_server_binds_and_builds_serving_router(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.unwrap();

    // Pre-bind port 0 for a free port, then release it so prepare_server can
    // bind the same address.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bind = probe.local_addr().unwrap();
    drop(probe);

    let prepared = prepare_server(&args, bind, true, None)
        .await
        .expect("prepare_server should succeed after init");
    assert_eq!(
        prepared.listener.local_addr().unwrap(),
        bind,
        "listener should be bound to the requested address"
    );

    // The router serves; drive it directly (no real socket needed). Wrap in a
    // LocalSet for Leptos SSR's spawn_local, as in after_init_server_responds_to_health_check.
    let local = tokio::task::LocalSet::new();
    let response = local
        .run_until(async move {
            prepared
                .router
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap()
        })
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

// prepare_server writes the runtime-info file with the bound address, and the
// guard removes it when the PreparedServer is dropped (ADR-0035).
#[apply(backends)]
#[tokio::test]
async fn prepare_server_writes_then_removes_runtime_file(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.unwrap();

    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bind = probe.local_addr().unwrap();
    drop(probe);

    let rt_path = base.path().join("runtime.json");
    let prepared = prepare_server(&args, bind, true, Some(rt_path.clone()))
        .await
        .expect("prepare_server should succeed after init");

    assert!(
        rt_path.exists(),
        "prepare_server should write the runtime file"
    );
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rt_path).unwrap()).unwrap();
    assert_eq!(v["port"], bind.port());

    drop(prepared);
    assert!(
        !rt_path.exists(),
        "dropping PreparedServer should remove the runtime file"
    );
}

// app-password-create mints a usable token for an existing user, and errors for
// an unknown user (covers both branches of app_password_create via the wrapper).
#[apply(backends)]
#[tokio::test]
async fn cmd_app_password_create_succeeds_for_existing_user(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.unwrap();
    let username: Username = "alice".parse().unwrap();
    let password: Password = "password123".parse().unwrap();
    cmd_user_create(&args, &username, Some(password), None, false)
        .await
        .unwrap();

    cmd_app_password_create(&args, &username, "ert")
        .await
        .expect("minting an app password for an existing user should succeed");
}

#[apply(backends)]
#[tokio::test]
async fn cmd_app_password_create_errors_for_unknown_user(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.unwrap();
    let username: Username = "ghost".parse().unwrap();

    assert!(cmd_app_password_create(&args, &username, "ert")
        .await
        .is_err());
}

#[apply(backends)]
#[tokio::test]
async fn cmd_user_create_creates_retrievable_user(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    cmd_user_create(&args, &username, Some(password), None, false)
        .await
        .expect("user create");

    let state = open_existing_database(&args.db).await.expect("open db");
    let user = state
        .users
        .get_user_by_username(&username)
        .await
        .expect("db query");
    assert!(user.is_some(), "user should exist after creation");
    assert_eq!(user.expect("user present").username, "alice");
}

// M6.1.7: creating a user with --operator sets is_operator to true.
#[apply(backends)]
#[tokio::test]
async fn cmd_user_create_with_operator_flag_sets_is_operator(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    let username: Username = "admin".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    cmd_user_create(&args, &username, Some(password), None, true)
        .await
        .expect("user create");

    let state = open_existing_database(&args.db).await.expect("open db");
    let user = state
        .users
        .get_user_by_username(&username)
        .await
        .expect("db query")
        .expect("user should exist");
    assert!(
        user.is_operator,
        "is_operator should be true for operator user"
    );
}

#[apply(backends)]
#[tokio::test]
async fn cmd_user_invite_creates_retrievable_invite(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    cmd_user_invite(&args, Some(parse_invite_ttl_hours("48")))
        .await
        .expect("user invite");

    let state = open_existing_database(&args.db).await.expect("open db");
    let invites = state.invites.list_invites().await.expect("list invites");
    assert_eq!(invites.len(), 1, "exactly one invite should exist");
}

#[apply(backends)]
#[tokio::test]
async fn cmd_user_invite_default_expires_in(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    cmd_user_invite(&args, None).await.expect("user invite");

    let state = open_existing_database(&args.db).await.expect("open db");
    let invites = state.invites.list_invites().await.expect("list invites");
    assert_eq!(invites.len(), 1, "exactly one invite should exist");
}

// #582: the out-of-range rejection that this test used to exercise (in-body `i64::try_from`)
// moved into `InviteTtlHours` — an out-of-range `--expires-in` is now refused by clap's
// `FromStr` parse, upstream of `cmd_user_invite`, and is covered by the newtype's unit test.
// A valid explicit TTL is covered by `cmd_user_invite_creates_retrievable_invite`.

// M6.3.2: backup command writes a directory-mode backup.
#[apply(backends)]
#[tokio::test]
async fn cmd_backup_writes_directory_backup(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    let username: Username = "backupuser".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    cmd_user_create(&args, &username, Some(password), None, false)
        .await
        .expect("user create");

    let media_path = args.storage_path.join("media");
    std::fs::write(media_path.join("avatar.txt"), "media").expect("write media");

    let backup_path = base.path().join("manual-backup");
    let written_path = cmd_backup(&args, BackupMode::Directory, Some(backup_path.clone()))
        .await
        .expect("backup");

    assert_eq!(written_path, backup_path);
    assert!(written_path.join("manifest.json").is_file());
    assert!(written_path.join("db").join("users.ndjson").is_file());
    assert_eq!(
        std::fs::read_to_string(written_path.join("media").join("avatar.txt")).expect("read media"),
        "media"
    );
}

// M6.3.2: backup command defaults to storage/backups.
#[apply(backends)]
#[tokio::test]
async fn cmd_backup_without_path_writes_under_storage_backups(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    let written_path = cmd_backup(&args, BackupMode::Directory, None)
        .await
        .expect("backup");

    assert!(written_path.starts_with(args.storage_path.join("backups")));
    assert!(written_path.join("manifest.json").is_file());
}

// M6.3.3: restore refuses missing backup paths before checking target state.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_refuses_missing_backup_path(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    let err = cmd_restore(&args, &base.path().join("missing"))
        .await
        .expect_err("restore fails");

    assert!(err.to_string().contains("backup path does not exist"));
}

// M6.3.3: restore refuses to run if the target database is populated.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_refuses_populated_database(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    let username: Username = "restoreuser".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    cmd_user_create(&args, &username, Some(password), None, false)
        .await
        .expect("user create");

    let backup_path = base.path().join("backup");
    std::fs::create_dir(&backup_path).expect("backup dir");
    let err = cmd_restore(&args, &backup_path)
        .await
        .expect_err("restore fails");

    assert!(err.to_string().contains("non-empty database"));
}

// M6.3.3: restore refuses to run if the media directory contains files.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_refuses_nonempty_media_directory(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    std::fs::write(args.storage_path.join("media").join("file.txt"), "media").expect("write media");

    let backup_path = base.path().join("backup");
    std::fs::create_dir(&backup_path).expect("backup dir");
    let err = cmd_restore(&args, &backup_path)
        .await
        .expect_err("restore fails");

    assert!(err.to_string().contains("non-empty media directory"));
}

// M6.3.3: an empty target passes safety checks and validates the backup layout.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_empty_target_rejects_invalid_backup(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    let backup_path = base.path().join("backup");
    std::fs::create_dir(&backup_path).expect("backup dir");
    let err = cmd_restore(&args, &backup_path)
        .await
        .expect_err("restore fails");

    assert!(err.to_string().contains("missing manifest"));
}

// M6.6.1: backup/restore round-trips database records and media.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_restores_directory_backup(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (source_args, _pg_source) = storage_args(backend, &base).await;
    cmd_init(&source_args, false).await.expect("init source");
    let ids = populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("backup");
    cmd_backup(
        &source_args,
        BackupMode::Directory,
        Some(backup_path.clone()),
    )
    .await
    .expect("backup");

    let target_base = TempDir::new().expect("target temp dir");
    let (target_args, _pg_target) = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");
    cmd_restore(&target_args, &backup_path)
        .await
        .expect("restore");

    assert_backup_fixture_restored(&target_args, &ids).await;
}

// #136: a backup with a dangling foreign key is rejected uniformly (DEC-C) —
// ConstraintViolation + target unmodified, on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_rejects_dangling_foreign_key(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (source_args, _pg_source) = storage_args(backend, &base).await;
    cmd_init(&source_args, false).await.expect("init source");
    let ids = populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("backup");
    cmd_backup(
        &source_args,
        BackupMode::Directory,
        Some(backup_path.clone()),
    )
    .await
    .expect("backup");

    // Append a post_tags row referencing a nonexistent tag_id → dangling FK. The row
    // MUST carry every column of the real exported row (post_id, tag_id, and the
    // NOT NULL tag_display) — import_table derives its column set from the first row
    // and rejects a row missing a column with InvalidBackup *before* inserting, which
    // would mask the FK violation.
    let post_tags = backup_path.join("db").join("post_tags.ndjson");
    let mut contents = std::fs::read_to_string(&post_tags).expect("read post_tags");
    writeln!(
        contents,
        "{{\"post_id\":{},\"tag_id\":999999,\"tag_display\":\"Dangling\"}}",
        ids.public_post
    )
    .expect("append dangling row");
    std::fs::write(&post_tags, contents).expect("write tampered post_tags");

    let target_base = TempDir::new().expect("target temp dir");
    let (target_args, _pg_target) = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");

    let err = cmd_restore(&target_args, &backup_path)
        .await
        .expect_err("restore rejects dangling FK");
    assert!(
        err.to_string().contains("failed constraint validation"),
        "expected ConstraintViolation, got: {err}"
    );

    // Rollback: nothing from the backup landed in the target.
    assert_target_unmodified(&target_args).await;
}

// #136: a backup with a malformed row is rejected and rolls back cleanly on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_rolls_back_on_malformed_row(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (source_args, _pg_source) = storage_args(backend, &base).await;
    cmd_init(&source_args, false).await.expect("init source");
    populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("backup");
    cmd_backup(
        &source_args,
        BackupMode::Directory,
        Some(backup_path.clone()),
    )
    .await
    .expect("backup");

    // Corrupt a NON-first table (posts, export index 6) with a non-object row, so an
    // earlier table (users, index 1) is inserted before the read fails — proving the
    // transaction rolls the earlier inserts back.
    let posts = backup_path.join("db").join("posts.ndjson");
    let mut contents = std::fs::read_to_string(&posts).expect("read posts");
    contents.push_str("[1, 2, 3]\n");
    std::fs::write(&posts, contents).expect("write tampered posts");

    let target_base = TempDir::new().expect("target temp dir");
    let (target_args, _pg_target) = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");

    let err = cmd_restore(&target_args, &backup_path)
        .await
        .expect_err("restore rejects malformed row");
    assert!(
        err.to_string().contains("non-object row"),
        "expected InvalidBackup, got: {err}"
    );

    assert_target_unmodified(&target_args).await;
}

// #136: a backup missing its db/ directory is rejected (InvalidBackup) on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_rejects_missing_db_directory(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (source_args, _pg_source) = storage_args(backend, &base).await;
    cmd_init(&source_args, false).await.expect("init source");
    populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("backup");
    cmd_backup(
        &source_args,
        BackupMode::Directory,
        Some(backup_path.clone()),
    )
    .await
    .expect("backup");

    std::fs::remove_dir_all(backup_path.join("db")).expect("remove db dir");

    let target_base = TempDir::new().expect("target temp dir");
    let (target_args, _pg_target) = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");

    let err = cmd_restore(&target_args, &backup_path)
        .await
        .expect_err("restore rejects missing db dir");
    assert!(
        err.to_string().contains("missing db directory"),
        "expected InvalidBackup, got: {err}"
    );
}

// #136: backup/restore round-trips in Archive mode on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_restores_archive_backup(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (source_args, _pg_source) = storage_args(backend, &base).await;
    cmd_init(&source_args, false).await.expect("init source");
    let ids = populate_backup_fixture(&source_args).await;

    let archive_path = base.path().join("backup.tar.gz");
    cmd_backup(
        &source_args,
        BackupMode::Archive,
        Some(archive_path.clone()),
    )
    .await
    .expect("backup");
    assert!(archive_path.is_file(), "archive backup is a single file");

    let target_base = TempDir::new().expect("target temp dir");
    let (target_args, _pg_target) = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");
    cmd_restore(&target_args, &archive_path)
        .await
        .expect("restore");

    assert_backup_fixture_restored(&target_args, &ids).await;
}

#[apply(backends)]
#[tokio::test]
async fn cmd_smtp_test_fails_when_not_initialized(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let args = uninitialized_storage_args(backend, &base);

    let result = cmd_smtp_test(&args, "alice@example.com").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("jaunder init"),
        "expected error to mention 'jaunder init', got: {msg}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn cmd_smtp_test_fails_when_smtp_not_configured(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    let result = cmd_smtp_test(&args, "alice@example.com").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("SMTP is not configured"),
        "expected 'SMTP is not configured', got: {msg}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn cmd_smtp_test_succeeds_with_mock_server(#[case] backend: Backend) {
    let server = maik::MockServer::builder()
        .no_verify_credentials()
        .assert_after_n_emails(1)
        .build();
    server.start();

    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
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

#[apply(backends)]
#[tokio::test]
async fn cmd_smtp_test_fails_on_invalid_to_address(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
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
        msg.contains("not-an-email") && msg.contains("invalid email address"),
        "expected the offending input and the typed parse reason, got: {msg}"
    );
}
