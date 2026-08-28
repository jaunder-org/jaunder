use std::fmt::Write as _;
use std::net::SocketAddr;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use clap::Parser as _;
use common::test_support::{parse_email, parse_invite_ttl_hours, parse_session_label};
use common::username::Username;
use host::config_key::SiteConfigKey;
use host::password::Password;
use jaunder::cli::{Cli, Commands, StorageArgs};
use jaunder::commands::{
    ServeCapturePaths, app_password_create, cmd_app_password_create, cmd_backup, cmd_init,
    cmd_restore, cmd_serve, cmd_smtp_test, cmd_user_create, cmd_user_invite, prepare_server,
};
use storage::{
    BackupError, BackupManifest, BackupMode, OpenedDatabase, open_database, open_existing_database,
    open_existing_database_with_observer,
};
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::misc::backup_fixture::{
    assert_backup_fixture_restored, assert_target_unmodified, populate_backup_fixture,
};
use storage::test_support::{
    Backend, PostgresDbGuard, PostgresTestConfig, SeedUser, backends, nonexistent_postgres_url,
    noop_mailer, raw_media_filename_exists, rewrite_media_filename_in_backup, sqlite_url,
    unique_postgres_url,
};

fn default_host_config() -> (host::telemetry::TelemetryConfig, Option<ServeCapturePaths>) {
    (
        host::telemetry::TelemetryConfig::from_raw(
            false,
            host::telemetry::TelemetryRawConfig {
                log_filter: Ok(None),
                rust_log: Ok(None),
                log_format: Ok(None),
                jaunder_otlp_endpoint: Ok(None),
                otlp_endpoint: Ok(None),
                slow_op_ms: Ok(None),
                e2e_seed_process: Ok(None),
            },
        ),
        None,
    )
}

async fn storage_args(backend: Backend, base: &TempDir) -> (StorageArgs, Option<PostgresDbGuard>) {
    let storage_path = base.path().join("storage");
    let (db, guard) = match backend {
        Backend::Sqlite => (sqlite_url(base), None),
        Backend::Postgres => {
            let config = PostgresTestConfig::from_env();
            let (db, guard) = unique_postgres_url(&config).await;
            (db, Some(guard))
        }
    };
    (StorageArgs { storage_path, db }, guard)
}

fn uninitialized_storage_args(backend: Backend, base: &TempDir) -> StorageArgs {
    let storage_path = base.path().join("storage");
    let db = match backend {
        Backend::Sqlite => sqlite_url(base),
        Backend::Postgres => {
            let config = PostgresTestConfig::from_env();
            nonexistent_postgres_url(&config)
        }
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
    open_database(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .unwrap();
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

// A non-Postgres bootstrap URL is unrepresentable — `PgBootstrapArgs.bootstrap_db`
// is a `PgConnectOptions`, so clap rejects it at argument parsing; pinned by
// `create_pg_db_rejects_a_non_postgres_bootstrap_url` in `server/src/cli.rs`.

// M1.5.4: cmd_serve fails with an appropriate error when the storage path has
// not been initialized.
#[apply(backends)]
#[tokio::test]
async fn cmd_serve_fails_when_not_initialized(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let expected = match &backend {
        Backend::Sqlite => "jaunder init",
        Backend::Postgres => "jaunder create-pg-db",
    };
    let args = uninitialized_storage_args(backend, &base);
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let (telemetry, capture) = default_host_config();
    let result = cmd_serve(&args, bind, true, None, &telemetry, capture.as_ref()).await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains(expected),
        "expected error to mention '{expected}', got: {msg}"
    );
}

fn assert_database_open_source(error: &anyhow::Error, command: &str) {
    assert!(
        error.to_string().contains("run `jaunder init` first"),
        "{command} must retain initialization guidance: {error:#}"
    );
    assert!(
        error
            .chain()
            .any(|source| source.downcast_ref::<sqlx::Error>().is_some()),
        "{command} must retain its typed SQLx source: {error:#}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn command_source_chain_cmd_user_create_open(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let args = uninitialized_storage_args(backend, &base);
    let username: Username = "alice".parse().expect("username");
    let password: Password = "password123".parse().expect("password");

    let error = cmd_user_create(&args, &username, Some(password), None, false)
        .await
        .unwrap_err();

    assert_database_open_source(&error, "cmd_user_create");
}

#[apply(backends)]
#[tokio::test]
async fn command_source_chain_cmd_app_password_create_open(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let args = uninitialized_storage_args(backend, &base);
    let username: Username = "alice".parse().expect("username");

    let error = cmd_app_password_create(&args, &username, &parse_session_label("integration"))
        .await
        .unwrap_err();

    assert_database_open_source(&error, "cmd_app_password_create");
}

#[apply(backends)]
#[tokio::test]
async fn command_source_chain_cmd_user_invite_open(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let args = uninitialized_storage_args(backend, &base);

    let error = cmd_user_invite(&args, None).await.unwrap_err();

    assert_database_open_source(&error, "cmd_user_invite");
}

#[apply(backends)]
#[tokio::test]
async fn command_source_chain_cmd_smtp_test_open(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let args = uninitialized_storage_args(backend, &base);

    let error = cmd_smtp_test(&args, &parse_email("to@example.com"))
        .await
        .unwrap_err();

    assert_database_open_source(&error, "cmd_smtp_test");
}

#[apply(backends)]
#[tokio::test]
async fn command_source_chain_cmd_smtp_test_quoted_sender_reaches_send(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("initialize");
    let state = open_existing_database(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .expect("open");
    state
        .site_config
        .set(SiteConfigKey::SmtpHost, "mail.example.com")
        .await
        .expect("set host");
    state
        .site_config
        .set(SiteConfigKey::SmtpSender, "Acme, Inc <noreply@example.com>")
        .await
        .expect("set sender");

    let error = cmd_smtp_test(&args, &parse_email("to@example.com"))
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "failed to send test email");
    assert!(
        error.chain().any(|source| source
            .downcast_ref::<lettre::transport::smtp::Error>()
            .is_some()),
        "quoted display sender must build and reach SMTP send: {error:#}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn command_source_chain_cmd_smtp_test_send(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("initialize");
    let state = open_existing_database(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .expect("open");
    for (key, value) in [
        (SiteConfigKey::SmtpHost, "127.0.0.1"),
        (SiteConfigKey::SmtpPort, "1"),
        (SiteConfigKey::SmtpTlsMode, "plain"),
    ] {
        state
            .site_config
            .set(key, value)
            .await
            .expect("set SMTP config");
    }

    let error = cmd_smtp_test(&args, &parse_email("to@example.com"))
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "failed to send test email");
    assert!(
        error.chain().any(|source| source
            .downcast_ref::<lettre::transport::smtp::Error>()
            .is_some()),
        "cmd_smtp_test must retain the send source: {error:#}"
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

    let OpenedDatabase {
        state, instance_id, ..
    } = open_existing_database_with_observer(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .unwrap();
    let router = jaunder::create_router(
        state,
        instance_id,
        noop_mailer(),
        true,
        args.storage_path.clone(),
    )
    .expect("canonical instance identity is an HTTP header");

    let response = router
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
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

    let (telemetry, capture) = default_host_config();
    let prepared = prepare_server(&args, bind, true, None, &telemetry, capture.as_ref())
        .await
        .expect("prepare_server should succeed after init");
    assert_eq!(
        prepared.listener.local_addr().unwrap(),
        bind,
        "listener should be bound to the requested address"
    );

    // The router serves; drive it directly (no real socket needed).
    let response = prepared
        .router
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
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
    let (telemetry, capture) = default_host_config();
    let prepared = prepare_server(
        &args,
        bind,
        true,
        Some(rt_path.clone()),
        &telemetry,
        capture.as_ref(),
    )
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

    cmd_app_password_create(&args, &username, &parse_session_label("ert"))
        .await
        .expect("minting an app password for an existing user should succeed");
}

/// The `--label` default is applied by **clap**, so nothing below
/// `cmd_app_password_create` can observe it — the function only ever sees a label that
/// someone already chose. Drive argv through the parser, hand the parsed label to the
/// command, and read the session back: that is what pins the default end to end, as
/// opposed to merely asserting the literal parses (`cli.rs`' test does that).
#[apply(backends)]
#[tokio::test]
async fn app_password_create_records_the_default_label(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.unwrap();
    let username: Username = "alice".parse().unwrap();
    let password: Password = "password123".parse().unwrap();
    cmd_user_create(&args, &username, Some(password), None, false)
        .await
        .unwrap();

    // No `--label`: clap supplies the default.
    let cli = Cli::try_parse_from(["jaunder", "app-password-create", "--username", "alice"])
        .expect("app-password-create must parse without --label");
    let Commands::AppPasswordCreate { label, .. } = cli.command.expect("subcommand") else {
        unreachable!("parse yields Commands::AppPasswordCreate")
    };

    cmd_app_password_create(&args, &username, &label)
        .await
        .expect("minting with the default label should succeed");

    let state = open_existing_database(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .expect("reopen");
    let user = state
        .users
        .get_user_by_username(&username)
        .await
        .unwrap()
        .expect("alice exists");
    let sessions = state.sessions.list_sessions(user.user_id).await.unwrap();
    assert_eq!(sessions.len(), 1, "one app password was minted");
    assert_eq!(sessions[0].label, "app-password");
}

#[apply(backends)]
#[tokio::test]
async fn cmd_app_password_create_errors_for_unknown_user(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.unwrap();
    let username: Username = "ghost".parse().unwrap();

    assert!(
        cmd_app_password_create(&args, &username, &parse_session_label("ert"))
            .await
            .is_err()
    );
}

#[apply(backends)]
#[tokio::test]
async fn typed_account_command_source_app_password_lookup(#[case] backend: Backend) {
    let env = backend.setup().await;
    env.base.close_pool().await;
    let username: Username = "alice".parse().expect("valid username");
    let label = parse_session_label("CLI");

    let error = app_password_create(&env.state, &username, &label)
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "failed to look up user");
    assert!(
        error
            .chain()
            .any(|source| source.downcast_ref::<sqlx::Error>().is_some())
    );
}

#[apply(backends)]
#[tokio::test]
async fn typed_account_command_source_app_password_session_create(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new().seed(&env.state).await;
    env.base
        .pool()
        .execute("ALTER TABLE sessions RENAME TO sessions_broken")
        .await
        .unwrap();
    let label = parse_session_label("CLI");

    let error = app_password_create(&env.state, &user.username, &label)
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "failed to create app password");
    assert!(
        error
            .chain()
            .any(|source| source.downcast_ref::<sqlx::Error>().is_some())
    );
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

    let state = open_existing_database(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .expect("open db");
    let user = state
        .users
        .get_user_by_username(&username)
        .await
        .expect("db query");
    assert!(user.is_some(), "user should exist after creation");
    assert_eq!(user.expect("user present").username, "alice");
}

#[apply(backends)]
#[tokio::test]
async fn typed_account_command_source_cmd_user_create(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "force-hash-error-for-test-coverage"
        .parse()
        .expect("valid password");

    let error = cmd_user_create(&args, &username, Some(password), None, false)
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "failed to create user");
    assert_eq!(
        error
            .chain()
            .find_map(|source| source.downcast_ref::<argon2::password_hash::Error>()),
        Some(&argon2::password_hash::Error::Version)
    );
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

    let state = open_existing_database(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .expect("open db");
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

    let state = open_existing_database(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .expect("open db");
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

    let state = open_existing_database(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .expect("open db");
    let invites = state.invites.list_invites().await.expect("list invites");
    assert_eq!(invites.len(), 1, "exactly one invite should exist");
}

// #582: an out-of-range `--expires-in` is refused by `InviteTtlHours`' clap
// `FromStr` parse, upstream of `cmd_user_invite`, and is covered by the newtype's
// unit test. A valid explicit TTL is covered by
// `cmd_user_invite_creates_retrievable_invite`.

// ADR-0064: every live table is included unless the format denylist deliberately
// excludes it. This exercises the public command seam on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_backup_covers_every_table_or_deliberately_excludes_it(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    let backup_path = base.path().join("backup");
    cmd_backup(&args, BackupMode::Directory, Some(backup_path.clone()))
        .await
        .expect("backup");

    let mut tables = serde_json::from_str::<BackupManifest>(
        &std::fs::read_to_string(backup_path.join("manifest.json")).expect("read manifest"),
    )
    .expect("parse manifest")
    .tables;
    tables.sort();
    assert_eq!(
        tables,
        [
            "audience_members",
            "audiences",
            "channels",
            "email_verifications",
            "feed_events",
            "idempotency_keys",
            "instance_identity",
            "invites",
            "media",
            "password_resets",
            "post_audiences",
            "post_media",
            "post_revisions",
            "post_tags",
            "posts",
            "sessions",
            "site_config",
            "subscription_statuses",
            "subscriptions",
            "tags",
            "target_kinds",
            "user_config",
            "users",
        ]
    );
}

// A pristine restore replaces the target's bootstrap identity with the identity
// from the source backup on both supported storage backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_adopts_backup_instance_identity(#[case] backend: Backend) {
    let source_base = TempDir::new().expect("source temp dir");
    let (source_args, _pg_source) = storage_args(backend, &source_base).await;
    cmd_init(&source_args, false).await.expect("init source");
    let source_identity = open_existing_database_with_observer(
        &source_args.db,
        &storage::StorageRuntimeConfig::default(),
    )
    .await
    .expect("open source")
    .instance_id;
    let backup_path = source_base.path().join("backup");
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
    let bootstrap_identity = open_existing_database_with_observer(
        &target_args.db,
        &storage::StorageRuntimeConfig::default(),
    )
    .await
    .expect("open target")
    .instance_id;
    assert_ne!(source_identity, bootstrap_identity);

    cmd_restore(&target_args, &backup_path)
        .await
        .expect("restore");
    let restored_identity = open_existing_database_with_observer(
        &target_args.db,
        &storage::StorageRuntimeConfig::default(),
    )
    .await
    .expect("reopen target")
    .instance_id;
    assert_eq!(restored_identity, source_identity);
}

#[apply(backends)]
#[tokio::test]
async fn cmd_backup_propagates_media_mirror_failure(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.expect("init");
    let media_path = args.storage_path.join("media");
    std::fs::remove_dir_all(&media_path).expect("remove media directory");
    std::fs::write(&media_path, "not a directory").expect("replace media with file");

    let error = cmd_backup(
        &args,
        BackupMode::Directory,
        Some(base.path().join("backup")),
    )
    .await
    .expect_err("backup propagates media traversal failure");
    assert!(matches!(
        error.downcast_ref::<BackupError>(),
        Some(BackupError::Io(_))
    ));
}

// Migration 0026 archives predate instance_identity. They restore successfully
// by bootstrapping a new identity instead of copying a missing row.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_pre_identity_backup_bootstraps_new_identity(#[case] backend: Backend) {
    let source_base = TempDir::new().expect("source temp dir");
    let (source_args, _pg_source) = storage_args(backend, &source_base).await;
    cmd_init(&source_args, false).await.expect("init source");
    let backup_path = source_base.path().join("backup");
    cmd_backup(
        &source_args,
        BackupMode::Directory,
        Some(backup_path.clone()),
    )
    .await
    .expect("backup");

    let manifest_path = backup_path.join("manifest.json");
    let mut manifest: BackupManifest =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    manifest.schema_version = 26;
    manifest.tables.retain(|table| table != "instance_identity");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    std::fs::remove_file(backup_path.join("db").join("instance_identity.ndjson"))
        .expect("remove instance identity export");

    let target_base = TempDir::new().expect("target temp dir");
    let (target_args, _pg_target) = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");
    let bootstrap_identity = open_existing_database_with_observer(
        &target_args.db,
        &storage::StorageRuntimeConfig::default(),
    )
    .await
    .expect("open target")
    .instance_id;
    cmd_restore(&target_args, &backup_path)
        .await
        .expect("restore pre-identity backup");
    let restored_identity = open_existing_database_with_observer(
        &target_args.db,
        &storage::StorageRuntimeConfig::default(),
    )
    .await
    .expect("reopen target")
    .instance_id;
    assert_ne!(restored_identity, bootstrap_identity);
}

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
    let outcome = cmd_restore(&target_args, &backup_path)
        .await
        .expect("restore");
    assert!(
        outcome.validation_report.is_empty(),
        "canonical encoded media.filename should not report validation issues: {:?}",
        outcome.validation_report.issues()
    );

    assert_backup_fixture_restored(&target_args, &ids).await;
}

#[apply(backends)]
#[tokio::test]
async fn cmd_restore_reports_invalid_media_filename_without_rolling_back(#[case] backend: Backend) {
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
    rewrite_media_filename_in_backup(&backup_path, "my photo.jpg");

    let target_base = TempDir::new().expect("target temp dir");
    let (target_args, _pg_target) = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");

    let outcome = cmd_restore(&target_args, &backup_path)
        .await
        .expect("restore with diagnostics");
    assert!(
        outcome.validation_report.issues().iter().any(|issue| {
            issue.table == "media"
                && issue.column == "filename"
                && issue.value_class == "filename"
                && issue.reason.contains("canonical percent-encoded")
        }),
        "restore report should name media.filename canonicity: {:?}",
        outcome.validation_report.issues()
    );
    assert!(
        raw_media_filename_exists(&target_args.db, "my photo.jpg").await,
        "invalid backup media row should still be restored"
    );
    assert_eq!(
        std::fs::read_to_string(target_args.storage_path.join("media").join("avatar.txt"))
            .expect("read restored media"),
        "media"
    );
}

// #857 / ADR-0054: restore re-enters through the live schema, so a zero-length
// subscriber reference is a uniform constraint violation and the import stays
// transactional on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_rejects_zero_length_subscriber_ref(#[case] backend: Backend) {
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

    let subscriptions = backup_path.join("db").join("subscriptions.ndjson");
    let mut rows = std::fs::read_to_string(&subscriptions)
        .expect("read exported subscriptions")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(line)
                .expect("parse exported subscription")
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "fixture exports one subscription");
    rows[0].insert(
        "subscriber_ref".to_owned(),
        serde_json::Value::String(String::new()),
    );
    let corrupted = rows
        .iter()
        .map(|row| serde_json::to_string(row).expect("serialize corrupted subscription"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&subscriptions, corrupted).expect("write corrupted subscriptions");

    let target_base = TempDir::new().expect("target temp dir");
    let (target_args, _pg_target) = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");

    let error = cmd_restore(&target_args, &backup_path)
        .await
        .expect_err("restore rejects a zero-length subscriber reference");
    assert!(
        matches!(
            error.downcast_ref::<BackupError>(),
            Some(BackupError::ConstraintViolation(_))
        ),
        "expected BackupError::ConstraintViolation, got: {error:#}"
    );
    assert_target_unmodified(&target_args).await;
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

    let result = cmd_smtp_test(&args, &parse_email("alice@example.com")).await;
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

    let result = cmd_smtp_test(&args, &parse_email("alice@example.com")).await;
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

    let state = open_existing_database(&args.db, &storage::StorageRuntimeConfig::default())
        .await
        .expect("open db");
    state
        .site_config
        .set(SiteConfigKey::SmtpHost, &server.host().to_string())
        .await
        .expect("set host");
    state
        .site_config
        .set(SiteConfigKey::SmtpPort, &server.port().to_string())
        .await
        .expect("set port");
    state
        .site_config
        .set(SiteConfigKey::SmtpTlsMode, "plain")
        .await
        .expect("set tls_mode");
    state
        .site_config
        .set(SiteConfigKey::SmtpSender, "noreply@example.com")
        .await
        .expect("set sender");
    state
        .site_config
        .set(SiteConfigKey::SmtpUsername, "user")
        .await
        .expect("set username");
    state
        .site_config
        .set(SiteConfigKey::SmtpPassword, "password")
        .await
        .expect("set password");

    cmd_smtp_test(&args, &parse_email("alice@example.com"))
        .await
        .expect("smtp test should succeed");

    let assertion = maik::MailAssertion::new().recipients_are(["alice@example.com"]);
    assert!(server.assert(assertion));
}
