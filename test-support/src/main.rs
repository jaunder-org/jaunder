//! `test-support` — out-of-process test/e2e helpers that link jaunder's real
//! crates (see `lib.rs`). Never shipped in the `jaunder` production binary.

use clap::{Parser, Subcommand};
use common::display_name::DisplayName;
use host::capture;
use storage::DbConnectOptions;

use test_support::{
    create_session_for_user, create_user, reset_mail, seed_posts_for_user, seed_user,
};

#[derive(Parser)]
#[command(
    name = "test-support",
    about = "Out-of-process test/e2e helpers (never shipped in jaunder)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Seed N posts for a user through the real storage path.
    SeedPosts {
        /// Database URL (`sqlite:...` or `postgres://...`) — the server's `--db`.
        #[arg(long, env = "JAUNDER_DB")]
        db: DbConnectOptions,
        /// The (already-registered) user to attribute the posts to.
        #[arg(long)]
        username: String,
        /// How many posts to seed.
        #[arg(long)]
        count: usize,
        /// Body/slug prefix; post `i` renders an H1 of "<prefix> i".
        #[arg(long)]
        body_prefix: String,
        /// Publish immediately (else the posts are left as drafts).
        #[arg(long)]
        published: bool,
    },
    /// Create a fixture user through the real storage path.
    CreateUser {
        /// Database URL (`sqlite:...` or `postgres://...`) — the server's `--db`.
        #[arg(long, env = "JAUNDER_DB")]
        db: DbConnectOptions,
        /// The username to create.
        #[arg(long)]
        username: String,
        /// The account password.
        #[arg(long)]
        password: String,
        /// Optional display name.
        #[arg(long)]
        display_name: Option<DisplayName>,
        /// Grant operator (admin) privileges.
        #[arg(long)]
        operator: bool,
    },
    /// Create a fixture user AND a session in one DB open; prints the seed
    /// record (cookie + marker) as one line of JSON on stdout.
    SeedUser {
        /// Database URL (`sqlite:...` or `postgres://...`) — the server's `--db`.
        #[arg(long, env = "JAUNDER_DB")]
        db: DbConnectOptions,
        /// The username to create.
        #[arg(long)]
        username: String,
        /// The account password.
        #[arg(long)]
        password: String,
        /// Session label (default "E2E seed").
        #[arg(long)]
        label: Option<String>,
    },
    /// Create a session for an EXISTING user; prints the seed record as one
    /// line of JSON on stdout.
    CreateSession {
        /// Database URL (`sqlite:...` or `postgres://...`) — the server's `--db`.
        #[arg(long, env = "JAUNDER_DB")]
        db: DbConnectOptions,
        /// The existing user's username.
        #[arg(long)]
        username: String,
        /// Session label (default "E2E seed").
        #[arg(long)]
        label: Option<String>,
    },
    /// Reset the mail-capture file (delete it; missing is fine). Derives
    /// `<JAUNDER_CAPTURE_DIR>/mail.jsonl`; errors if the capture dir is unset.
    ResetMail,
    /// Print the resolved capture-file path for a stream (`mail`/`websub`/`diag`),
    /// derived from `JAUNDER_CAPTURE_DIR`. Errors on an unset dir or unknown stream.
    CapturePath {
        /// The capture stream key.
        stream: String,
    },
    /// Fail if the scoped diagnostic stream or required server log records a Rust panic.
    VerifyNoPanics {
        /// E2E capture directory; the command projects its diagnostic leaf using
        /// `host::capture`'s canonical stream filename.
        #[arg(long)]
        capture_dir: std::path::PathBuf,
        /// Required VM-journal or host-stderr capture.
        #[arg(long)]
        server_log: std::path::PathBuf,
    },
}

fn inherited(name: &str) -> Result<Option<String>, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error),
    }
}

fn telemetry_config() -> host::telemetry::TelemetryConfig {
    host::telemetry::TelemetryConfig::from_raw(
        false,
        host::telemetry::TelemetryRawConfig {
            log_filter: inherited("JAUNDER_LOG_FILTER"),
            rust_log: inherited("RUST_LOG"),
            log_format: inherited("JAUNDER_LOG_FORMAT"),
            jaunder_otlp_endpoint: inherited("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT"),
            otlp_endpoint: inherited("OTEL_EXPORTER_OTLP_ENDPOINT"),
            slow_op_ms: inherited("JAUNDER_SLOW_OP_MS"),
            e2e_seed_process: inherited("JAUNDER_E2E_SEED_PROCESS"),
        },
    )
}

fn storage_runtime_config(
    db: &DbConnectOptions,
) -> Result<storage::StorageRuntimeConfig, storage::PostgresPasswordError> {
    let (password_file, password) = match db {
        DbConnectOptions::Sqlite(_) => (Ok(None), Ok(None)),
        DbConnectOptions::Postgres { .. } => {
            let password_file = match std::env::var("JAUNDER_DB_PASSWORD_FILE") {
                Ok(path) => Ok(Some(std::fs::read_to_string(path))),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(error) => Err(error),
            };
            (password_file, inherited("JAUNDER_DB_PASSWORD"))
        }
    };
    storage::StorageRuntimeConfig::from_raw(
        inherited("JAUNDER_SQL_SLOW_MS"),
        password_file,
        password,
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Cli::parse()).await
}

/// Dispatch the parsed subcommand to its handler. A flat match: each arm
/// evaluates to the handler's `Result<()>`, so `main` stays a thin shell and each
/// command is a small, individually-covered unit (#232).
async fn run(cli: Cli) -> anyhow::Result<()> {
    let telemetry = telemetry_config();
    let _telemetry = host::telemetry::init_tracing(&telemetry);
    match cli.command {
        Commands::SeedPosts {
            db,
            username,
            count,
            body_prefix,
            published,
        } => {
            let storage_runtime = storage_runtime_config(&db)?;
            cmd_seed_posts(
                &db,
                &storage_runtime,
                &username,
                count,
                &body_prefix,
                published,
            )
            .await
        }
        Commands::CreateUser {
            db,
            username,
            password,
            display_name,
            operator,
        } => {
            let storage_runtime = storage_runtime_config(&db)?;
            cmd_create_user(
                &db,
                &storage_runtime,
                &username,
                &password,
                display_name.as_ref(),
                operator,
            )
            .await
        }
        Commands::ResetMail => {
            let mail_path = capture_directory()?.path(capture::Stream::Mail);
            cmd_reset_mail(&mail_path)
        }
        Commands::SeedUser {
            db,
            username,
            password,
            label,
        } => {
            let storage_runtime = storage_runtime_config(&db)?;
            cmd_seed_user(
                &db,
                &storage_runtime,
                &username,
                &password,
                label.as_deref(),
            )
            .await
        }
        Commands::CreateSession {
            db,
            username,
            label,
        } => {
            let storage_runtime = storage_runtime_config(&db)?;
            cmd_create_session(&db, &storage_runtime, &username, label.as_deref()).await
        }
        Commands::CapturePath { stream } => {
            let stream = capture::Stream::parse(&stream)
                .ok_or_else(|| anyhow::anyhow!("unknown capture stream {stream:?}"))?;
            let path = capture_directory()?.path(stream);
            cmd_capture_path(&path);
            Ok(())
        }
        Commands::VerifyNoPanics {
            capture_dir,
            server_log,
        } => {
            let diag_path = capture_dir.join(capture::Stream::Diag.filename());
            test_support::panic_gate::verify_no_panics(&diag_path, &server_log)
        }
    }
}

/// Resolves the capture directory only for commands that consume capture paths.
fn capture_directory() -> anyhow::Result<capture::CaptureDirectory> {
    capture::CaptureDirectory::from_raw(std::env::var_os(capture::DIR_ENV))?
        .ok_or_else(|| anyhow::anyhow!("JAUNDER_CAPTURE_DIR is not set"))
}

/// Seed `count` posts for `username` through the real storage path.
async fn cmd_seed_posts(
    db: &DbConnectOptions,
    runtime: &storage::StorageRuntimeConfig,
    username: &str,
    count: usize,
    body_prefix: &str,
    published: bool,
) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db, runtime).await?;
    let ids = seed_posts_for_user(&state, username, count, published, body_prefix).await?;
    eprintln!("seeded {} posts for {username}", ids.len());
    Ok(())
}

/// Create a fixture user through the real storage path.
async fn cmd_create_user(
    db: &DbConnectOptions,
    runtime: &storage::StorageRuntimeConfig,
    username: &str,
    password: &str,
    display_name: Option<&DisplayName>,
    operator: bool,
) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db, runtime).await?;
    let id = create_user(&state, username, password, display_name, operator).await?;
    eprintln!("created user {username} with id {}", i64::from(id));
    Ok(())
}

/// Create a fixture user and a session; print the seed record as JSON.
async fn cmd_seed_user(
    db: &DbConnectOptions,
    runtime: &storage::StorageRuntimeConfig,
    username: &str,
    password: &str,
    label: Option<&str>,
) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db, runtime).await?;
    let record = seed_user(&state, username, password, label).await?;
    println!("{}", serde_json::to_string(&record)?);
    Ok(())
}

async fn cmd_create_session(
    db: &DbConnectOptions,
    runtime: &storage::StorageRuntimeConfig,
    username: &str,
    label: Option<&str>,
) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db, runtime).await?;
    let record = create_session_for_user(&state, username, label).await?;
    println!("{}", serde_json::to_string(&record)?);
    Ok(())
}

/// Reset the mail-capture file (delete it; missing is fine).
fn cmd_reset_mail(mail_path: &std::path::Path) -> anyhow::Result<()> {
    reset_mail(mail_path)?;
    eprintln!("reset mail-capture file {}", mail_path.display());
    Ok(())
}

/// Print a resolved capture-file path.
fn cmd_capture_path(path: &std::path::Path) {
    println!("{}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt as _;
    use storage::test_support::sqlite_url;
    use tempfile::TempDir;

    fn cli(command: Commands) -> Cli {
        Cli { command }
    }

    /// A temp `SQLite` DB, created + migrated. The migrating pool is dropped before
    /// return (unbound temporary), so each `run` below opens its own connection.
    /// The returned `TempDir` must outlive the test — dropping it unlinks the file.
    async fn temp_db() -> (TempDir, DbConnectOptions) {
        let dir = TempDir::new().unwrap();
        let db = sqlite_url(&dir);
        storage::open_database(&db, &storage::StorageRuntimeConfig::default())
            .await
            .unwrap();
        (dir, db)
    }

    #[test]
    fn subprocess_classifies_inherited_configuration_inputs() {
        const SCENARIO: &str = "JAUNDER_TEST_SUPPORT_CONFIG_SCENARIO";
        if let Some(scenario) = std::env::var_os(SCENARIO) {
            let _telemetry = telemetry_config();
            let db: DbConnectOptions = "postgres://app@localhost/jaunder"
                .parse()
                .expect("PostgreSQL URL");
            let result = storage_runtime_config(&db);
            match scenario.to_string_lossy().as_ref() {
                "file" | "password" | "absent" => {
                    result.expect("valid inherited configuration");
                }
                "invalid-file-variable" => {
                    assert!(matches!(
                        result,
                        Err(storage::PostgresPasswordError::FileVariable(_))
                    ));
                }
                _ => unreachable!("parent supplies a closed configuration scenario set"),
            }
            return;
        }

        let dir = TempDir::new().expect("password directory");
        let password_file = dir.path().join("password");
        std::fs::write(&password_file, "from-file\n").expect("password fixture");
        for scenario in ["file", "password", "absent", "invalid-file-variable"] {
            let mut command =
                std::process::Command::new(std::env::current_exe().expect("test executable"));
            command.args([
                "--exact",
                "tests::subprocess_classifies_inherited_configuration_inputs",
                "--nocapture",
            ]);
            command.env(SCENARIO, scenario);
            for name in [
                "JAUNDER_LOG_FILTER",
                "RUST_LOG",
                "JAUNDER_LOG_FORMAT",
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                "JAUNDER_SLOW_OP_MS",
                "JAUNDER_E2E_SEED_PROCESS",
                "JAUNDER_SQL_SLOW_MS",
                "JAUNDER_DB_PASSWORD_FILE",
                "JAUNDER_DB_PASSWORD",
            ] {
                command.env_remove(name);
            }
            match scenario {
                "file" => {
                    command.env("JAUNDER_DB_PASSWORD_FILE", &password_file);
                }
                "password" => {
                    command.env("JAUNDER_DB_PASSWORD", "from-variable");
                }
                "invalid-file-variable" => {
                    command.env(
                        "JAUNDER_DB_PASSWORD_FILE",
                        std::ffi::OsString::from_vec(vec![0xff]),
                    );
                    command.env(
                        "JAUNDER_LOG_FILTER",
                        std::ffi::OsString::from_vec(vec![0xff]),
                    );
                }
                "absent" => {}
                _ => unreachable!("closed parent scenario set"),
            }
            assert!(
                command
                    .status()
                    .expect("spawn configuration child")
                    .success(),
                "configuration child scenario {scenario} must succeed"
            );
        }
    }

    /// Capture setup belongs only to the two commands that consume its paths.
    /// A child process supplies an impossible directory so this test never
    /// mutates the test runner's inherited environment.
    #[test]
    fn subprocess_scopes_broken_capture_configuration_to_capture_commands() {
        const SCENARIO: &str = "JAUNDER_TEST_SUPPORT_CAPTURE_SCOPE_SCENARIO";
        if std::env::var_os(SCENARIO).is_some() {
            let runtime = tokio::runtime::Runtime::new().expect("test runtime");
            runtime.block_on(async {
                let (_dir, db) = temp_db().await;
                run(cli(Commands::CreateUser {
                    db: db.clone(),
                    username: "alice".to_owned(),
                    password: "password123".to_owned(),
                    display_name: None,
                    operator: false,
                }))
                .await
                .expect("create-user ignores capture configuration");
                run(cli(Commands::SeedPosts {
                    db: db.clone(),
                    username: "alice".to_owned(),
                    count: 1,
                    body_prefix: "Post".to_owned(),
                    published: true,
                }))
                .await
                .expect("seed-posts ignores capture configuration");
                run(cli(Commands::SeedUser {
                    db: db.clone(),
                    username: "bob".to_owned(),
                    password: "password123".to_owned(),
                    label: None,
                }))
                .await
                .expect("seed-user ignores capture configuration");
                run(cli(Commands::CreateSession {
                    db,
                    username: "bob".to_owned(),
                    label: None,
                }))
                .await
                .expect("create-session ignores capture configuration");

                let capture_dir = TempDir::new().expect("panic-gate capture directory");
                let server_log = capture_dir.path().join("server.log");
                std::fs::write(&server_log, b"clean stderr\n").expect("server log");
                run(cli(Commands::VerifyNoPanics {
                    capture_dir: capture_dir.path().to_owned(),
                    server_log,
                }))
                .await
                .expect("verify-no-panics ignores capture configuration");

                let diag_path = capture_dir.path().join(capture::Stream::Diag.filename());
                std::fs::write(&diag_path, b"thread panicked at src/diag.rs:1:1\n")
                    .expect("diagnostic log");
                let server_log = capture_dir.path().join("server-after-diag.log");
                std::fs::write(&server_log, b"clean stderr\n").expect("server log");
                let error = run(cli(Commands::VerifyNoPanics {
                    capture_dir: capture_dir.path().to_owned(),
                    server_log,
                }))
                .await
                .expect_err("verify-no-panics reads the conventional diagnostic leaf");
                assert!(error.to_string().contains("src/diag.rs:1:1"), "{error}");
            });
            return;
        }

        let dir = TempDir::new().expect("capture configuration directory");
        let blocking_file = dir.path().join("not-a-directory");
        std::fs::write(&blocking_file, b"file").expect("blocking file");
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "tests::subprocess_scopes_broken_capture_configuration_to_capture_commands",
                "--nocapture",
            ])
            .env(SCENARIO, "1")
            .env(capture::DIR_ENV, blocking_file.join("capture"))
            .status()
            .expect("spawn capture-scope child");

        assert!(
            status.success(),
            "unrelated commands must ignore a broken capture directory"
        );
    }

    #[tokio::test]
    async fn run_dispatches_db_commands_against_a_temp_db() {
        let (_dir, db) = temp_db().await;

        run(cli(Commands::CreateUser {
            db: db.clone(),
            username: "alice".to_owned(),
            password: "password123".to_owned(),
            display_name: None,
            operator: false,
        }))
        .await
        .expect("create-user should dispatch and succeed");

        run(cli(Commands::SeedPosts {
            db: db.clone(),
            username: "alice".to_owned(),
            count: 1,
            body_prefix: "Post".to_owned(),
            published: true,
        }))
        .await
        .expect("seed-posts should dispatch and succeed");

        run(cli(Commands::SeedUser {
            db: db.clone(),
            username: "bob".to_owned(),
            password: "password123".to_owned(),
            label: None,
        }))
        .await
        .expect("seed-user should dispatch and succeed");

        run(cli(Commands::CreateSession {
            db: db.clone(),
            username: "bob".to_owned(),
            label: Some("CI bot".to_owned()),
        }))
        .await
        .expect("create-session should dispatch and succeed");

        // Read back through a fresh connection to prove the dispatch wired each
        // command's arguments through to storage (not merely returned Ok): the
        // seeded post is published and attributed to alice.
        let state = storage::open_existing_database(&db, &storage::StorageRuntimeConfig::default())
            .await
            .unwrap();
        let published = state
            .posts
            .list_published_by_user(
                &"alice".parse().unwrap(),
                None,
                common::test_support::parse_row_limit("10"),
                &common::visibility::ViewerIdentity::Anonymous,
                common::time::UtcInstant::now(),
            )
            .await
            .expect("list ok");
        assert_eq!(
            published.len(),
            1,
            "seed-posts should publish 1 post for alice"
        );

        // Same read-back proof for the session commands: bob exists and holds
        // two sessions (seed-user's plus create-session's explicitly-labelled
        // one), so both commands' arguments reached storage.
        let bob = state
            .users
            .get_user_by_username(&"bob".parse().unwrap())
            .await
            .expect("lookup ok")
            .expect("bob created");
        let sessions = state
            .sessions
            .list_sessions(bob.user_id)
            .await
            .expect("list sessions ok");
        assert_eq!(
            sessions.len(),
            2,
            "seed-user + create-session = two sessions"
        );
        assert!(
            sessions.iter().any(|s| s.label == "CI bot"),
            "the --label argument should reach the stored session"
        );
    }
}
