use clap::Parser;
use jaunder::cli::Cli;

fn inherited(name: &str) -> Result<Option<String>, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error),
    }
}

fn telemetry_config(verbose: bool) -> host::telemetry::TelemetryConfig {
    host::telemetry::TelemetryConfig::from_raw(
        verbose,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Fail-closed: a production binary must never link host password hashing
    // with cheap test KDF parameters. Feature isolation keeps this false in
    // production; if it is ever true, refuse to start rather than hash weakly.
    if host::CHEAP_KDF_ENABLED {
        eprintln!(
            "FATAL: jaunder built with cheap-kdf (test-only password hashing); refusing to start"
        );
        std::process::exit(1);
    } // cov:ignore process::exit(1) above diverges, so this closing brace is unreachable
    // cov:ignore-start
    let cli = Cli::parse();
    run(cli).await
    // cov:ignore-stop
}

/// Executes the application logic based on the provided CLI arguments.
///
/// # Errors
///
/// Returns an error if the command execution fails.
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let Some(command) = cli.command else {
        // `jaunder` with no subcommand is not runnable — re-parse to trigger
        // clap's built-in help/usage, which prints and exits.
        // cov:ignore-start
        Cli::parse_from(["jaunder", "--help"]);
        // cov:ignore-stop
        unreachable!("Cli::parse_from([\"jaunder\", \"--help\"]) prints help and exits the process")
    };
    let telemetry = telemetry_config(cli.verbose);
    if command.is_serve() {
        let capture =
            host::capture::CaptureDirectory::from_raw(std::env::var_os(host::capture::DIR_ENV))?;
        let diag_path = capture
            .as_ref()
            .map(|directory| directory.path(host::capture::Stream::Diag));
        let capture_paths = capture.map(|directory| jaunder::commands::ServeCapturePaths {
            mail: directory.path(host::capture::Stream::Mail),
            websub: directory.path(host::capture::Stream::WebSub),
        });
        let _telemetry = jaunder::observability::init_server_tracing(&telemetry, diag_path);
        command.execute(&telemetry, capture_paths).await.map(drop)
    } else {
        let _telemetry = host::telemetry::init_tracing(&telemetry);
        command.execute(&telemetry, None).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::{parse_email, parse_session_label};
    use host::config_key::SiteConfigKey;
    use jaunder::cli::{
        Cli, CliBackupMode, Commands, PgBootstrapArgs, SiteConfigAction, StorageArgs,
    };
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt as _;
    use tempfile::TempDir;

    fn test_storage_args(base: &TempDir) -> StorageArgs {
        StorageArgs {
            storage_path: base.path().join("storage"),
            db: format!("sqlite:{}", base.path().join("test.db").display())
                .parse()
                .unwrap(),
        }
    }
    fn test_cli(command: Commands) -> Cli {
        Cli {
            command: Some(command),
            verbose: false,
        }
    }

    async fn initialized_storage(base: &TempDir) -> StorageArgs {
        let storage = test_storage_args(base);
        run(test_cli(Commands::Init {
            storage: storage.clone(),
            skip_if_exists: false,
        }))
        .await
        .unwrap();
        storage
    }

    #[tokio::test]
    async fn run_init_triggers_tracing() {
        let base = TempDir::new().unwrap();
        let cli = Cli {
            command: Some(Commands::Init {
                storage: test_storage_args(&base),
                skip_if_exists: false,
            }),
            verbose: true,
        };
        run(cli).await.unwrap();
    }

    #[tokio::test]
    async fn run_user_create() {
        let base = TempDir::new().unwrap();
        let storage = initialized_storage(&base).await;

        let cli = test_cli(Commands::UserCreate {
            storage,
            username: "alice".parse().unwrap(),
            password: Some(host::test_support::parse_password("password123")),
            display_name: None,
            operator: false,
        });
        run(cli).await.unwrap();
    }

    #[tokio::test]
    async fn run_user_invite() {
        let base = TempDir::new().unwrap();
        let storage = initialized_storage(&base).await;
        run(test_cli(Commands::SiteConfig {
            action: SiteConfigAction::Set {
                storage: storage.clone(),
                key: SiteConfigKey::SiteRegistrationPolicy,
                value: "operator_invites".to_string(),
            },
        }))
        .await
        .expect("enable operator-issued invitations");

        let cli = test_cli(Commands::UserInvite {
            storage,
            expires_in: None,
        });
        run(cli).await.unwrap();
    }

    #[tokio::test]
    async fn run_site_config_set_get_list_unset_dispatch() {
        let base = TempDir::new().unwrap();
        let storage = initialized_storage(&base).await;

        // set dispatches and upserts through the real storage path.
        run(test_cli(Commands::SiteConfig {
            action: SiteConfigAction::Set {
                storage: storage.clone(),
                key: SiteConfigKey::SiteRegistrationPolicy,
                value: "open".to_string(),
            },
        }))
        .await
        .unwrap();

        // get of a set key dispatches and succeeds.
        run(test_cli(Commands::SiteConfig {
            action: SiteConfigAction::Get {
                storage: storage.clone(),
                key: SiteConfigKey::SiteRegistrationPolicy,
            },
        }))
        .await
        .expect("get of a set key succeeds");

        // get of an unset key dispatches and errors (→ non-zero exit). Every key is a
        // registry variant, so "unset" means "no row written", not "not a key".
        let missing = run(test_cli(Commands::SiteConfig {
            action: SiteConfigAction::Get {
                storage: storage.clone(),
                key: SiteConfigKey::SiteTitle,
            },
        }))
        .await;
        assert!(missing.is_err(), "get of an unset key must error");

        // list dispatches and succeeds.
        run(test_cli(Commands::SiteConfig {
            action: SiteConfigAction::List {
                storage: storage.clone(),
            },
        }))
        .await
        .expect("list succeeds");

        // unset of a present key dispatches and removes it (the removed branch).
        run(test_cli(Commands::SiteConfig {
            action: SiteConfigAction::Unset {
                storage: storage.clone(),
                key: SiteConfigKey::SiteRegistrationPolicy,
            },
        }))
        .await
        .expect("unset of a present key succeeds");

        // get now errors: the key is gone.
        let after_unset = run(test_cli(Commands::SiteConfig {
            action: SiteConfigAction::Get {
                storage: storage.clone(),
                key: SiteConfigKey::SiteRegistrationPolicy,
            },
        }))
        .await;
        assert!(after_unset.is_err(), "unset key must read as unset");

        // unset of an absent key is an idempotent no-op (the no-op branch).
        run(test_cli(Commands::SiteConfig {
            action: SiteConfigAction::Unset {
                storage,
                key: SiteConfigKey::SiteRegistrationPolicy,
            },
        }))
        .await
        .expect("unset of an absent key is a no-op success");
    }

    #[test]
    fn run_site_config_ignores_broken_capture_configuration() {
        const CHILD: &str = "JAUNDER_TEST_SITE_CONFIG_TELEMETRY_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let base = TempDir::new().expect("db dir");
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(async {
                    let storage = initialized_storage(&base).await;
                    run(test_cli(Commands::SiteConfig {
                        action: SiteConfigAction::Set {
                            storage,
                            key: SiteConfigKey::SiteRegistrationPolicy,
                            value: "open".to_string(),
                        },
                    }))
                    .await
                    .expect("site-config set");
                });
            println!("MAIN_TEST_CHILD_COMPLETED");
            return;
        }

        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "tests::run_site_config_ignores_broken_capture_configuration",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(
                host::capture::DIR_ENV,
                std::ffi::OsString::from_vec(vec![0xff]),
            )
            .env(
                "JAUNDER_LOG_FILTER",
                std::ffi::OsString::from_vec(vec![0xff]),
            )
            .env(
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
                "not a valid endpoint",
            )
            .output()
            .expect("run isolated root-wiring test");
        assert!(
            output.status.success(),
            "child status: {}; stderr: {}",
            output.status,
            // The root-wiring contract requires child success; this is diagnostic-only.
            String::from_utf8_lossy(&output.stderr) // cov:ignore
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("MAIN_TEST_CHILD_COMPLETED"),
            "child did not complete root wiring: {}",
            // A successful child always emits the projection; this is diagnostic-only.
            String::from_utf8_lossy(&output.stdout) // cov:ignore
        );
    }
    #[cfg(unix)]
    #[test]
    fn run_resolves_capture_only_for_serve_and_fails_fast_when_configured() {
        const CHILD: &str = "JAUNDER_TEST_SERVE_CAPTURE_CHILD";
        if let Some(scenario) = std::env::var_os(CHILD) {
            let base = TempDir::new().expect("storage dir");
            let storage = test_storage_args(&base);
            let result = tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(run(test_cli(Commands::Serve {
                    storage,
                    bind: "127.0.0.1:0".parse().expect("bind"),
                    environment: jaunder::cli::DeploymentEnv::Prod,
                })));
            match scenario.to_string_lossy().as_ref() {
                "absent" | "valid" => assert!(
                    result
                        .is_err_and(|error| error.to_string().contains("run `jaunder init` first")),
                    "capture must reach ordinary server startup"
                ),
                "nonunicode" | "file" => assert!(
                    result.is_err_and(|error| error.to_string().contains("capture directory")),
                    "configured capture must fail before server startup"
                ),
                _ => unreachable!("parent supplies a closed capture scenario set"),
            }
            return;
        }

        let file = tempfile::NamedTempFile::new().expect("capture file");
        let capture_root = TempDir::new().expect("capture root");
        let valid_capture = capture_root.path().join("capture");
        for scenario in ["absent", "nonunicode", "file", "valid"] {
            let mut command =
                std::process::Command::new(std::env::current_exe().expect("test executable"));
            command.args([
                "--exact",
                "tests::run_resolves_capture_only_for_serve_and_fails_fast_when_configured",
                "--nocapture",
            ]);
            command
                .env(CHILD, scenario)
                .env_remove(host::capture::DIR_ENV);
            match scenario {
                "nonunicode" => {
                    command.env(
                        host::capture::DIR_ENV,
                        std::ffi::OsString::from_vec(vec![0xff]),
                    );
                }
                "file" => {
                    command.env(host::capture::DIR_ENV, file.path());
                }
                "absent" => {}
                "valid" => {
                    command.env(host::capture::DIR_ENV, &valid_capture);
                }
                _ => unreachable!("closed capture scenario set"),
            }
            assert!(
                command
                    .status()
                    .expect("spawn capture configuration child")
                    .success(),
                "capture child scenario {scenario} must succeed"
            );
        }
        assert!(
            valid_capture.is_dir(),
            "valid capture directory must be prepared"
        );
        assert!(
            valid_capture.join("diag.log").is_file(),
            "serve must project the diagnostic capture leaf"
        );
    }

    #[tokio::test]
    async fn run_smtp_test_fails_when_smtp_not_configured() {
        let base = TempDir::new().unwrap();
        let storage = initialized_storage(&base).await;

        let cli = test_cli(Commands::SmtpTest {
            storage,
            to: parse_email("alice@example.com"),
        });
        let result = run(cli).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("SMTP is not configured"),
            "expected 'SMTP is not configured', got: {msg}"
        );
    }

    #[tokio::test]
    async fn run_app_password_create_mints_for_existing_user() {
        let base = TempDir::new().unwrap();
        let storage = initialized_storage(&base).await;
        run(test_cli(Commands::UserCreate {
            storage: storage.clone(),
            username: "alice".parse().unwrap(),
            password: Some(host::test_support::parse_password("password123")),
            display_name: None,
            operator: false,
        }))
        .await
        .unwrap();

        run(test_cli(Commands::AppPasswordCreate {
            storage,
            username: "alice".parse().unwrap(),
            label: parse_session_label("ert"),
        }))
        .await
        .expect("app-password-create should succeed for an existing user");
    }

    #[tokio::test]
    async fn run_serve() {
        let base = TempDir::new().unwrap();
        let storage = initialized_storage(&base).await;

        let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let cli = test_cli(Commands::Serve {
            storage,
            bind,
            environment: jaunder::cli::DeploymentEnv::Dev,
        });

        // Spawn-and-abort: this pins the dispatch arm, not the serve loop.
        let task = tokio::spawn(async move {
            let _ = run(cli).await;
        }); // cov:ignore

        // Wait a bit for it to start.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        task.abort();
    }

    // Covers the `Commands::CreatePgDb` dispatch arm.
    //
    // The arguments are *valid* (an invalid bootstrap URL is unrepresentable —
    // `BootstrapDb` rejects a non-PostgreSQL scheme at parse time, pinned by
    // `create_pg_db_rejects_a_non_postgres_bootstrap_url` in `cli.rs`); the bootstrap
    // URL simply points at a closed port, and the command fails fast at the admin
    // connection. That exercises the dispatch without provisioning anything.
    #[tokio::test]
    async fn run_create_pg_db_dispatches() {
        let cli = test_cli(Commands::CreatePgDb {
            pg: PgBootstrapArgs {
                bootstrap_db: "postgres://postgres@localhost:1/postgres".parse().unwrap(),
                app_db: "postgres://jaunder@localhost/jaunder".parse().unwrap(),
                app_role_password: "hunter2".parse().unwrap(),
            },
        });

        let err = run(cli).await.expect_err("a closed port must fail");

        // Assert *which* failure: a bare `is_err()` would also pass if the arguments were
        // rejected, which would mean the dispatch this test exists to cover never ran.
        let msg = err.to_string();
        for argument_rejection in [
            "must be a PostgreSQL URL",
            "must include a PostgreSQL database name",
        ] {
            assert!(
                !msg.contains(argument_rejection),
                "arguments are valid; the failure must come from the connection: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn run_init_fails_on_invalid_path() {
        let base = TempDir::new().unwrap();
        // Create a file where the storage directory should be
        let storage_path = base.path().join("file");
        std::fs::write(&storage_path, "not a dir").unwrap();

        let cli = test_cli(Commands::Init {
            storage: StorageArgs {
                storage_path: storage_path.clone(),
                db: format!("sqlite:{}", base.path().join("test.db").display())
                    .parse()
                    .unwrap(),
            },
            skip_if_exists: false,
        });
        let err = run(cli).await.unwrap_err();
        assert!(
            err.to_string().contains("Not a directory") || err.to_string().contains("File exists")
        );
    }

    #[tokio::test]
    async fn run_serve_prod_fails_when_uninitialized() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        let cli = test_cli(Commands::Serve {
            storage,
            bind: "127.0.0.1:0".parse().unwrap(),
            environment: jaunder::cli::DeploymentEnv::Prod,
        });
        let err = run(cli).await.unwrap_err();
        assert!(err.to_string().contains("run `jaunder init` first"));
    }

    #[tokio::test]
    async fn run_serve_dev_auto_inits() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let cli = test_cli(Commands::Serve {
            storage,
            bind,
            environment: jaunder::cli::DeploymentEnv::Dev,
        });

        let task = tokio::spawn(async move {
            let _ = run(cli).await;
        }); // cov:ignore

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        task.abort();

        // Verify that the database was created by auto-init.
        assert!(base.path().join("test.db").exists());
    }

    #[tokio::test]
    async fn run_backup_creates_artifact() {
        let base = TempDir::new().unwrap();
        let storage = initialized_storage(&base).await;

        // `cmd_backup` creates the artifact itself, so no prior backup is needed.
        let backup_path = base.path().join("backup");
        run(test_cli(Commands::Backup {
            storage,
            mode: CliBackupMode::Directory,
            path: Some(backup_path.clone()),
        }))
        .await
        .expect("backup dispatch should succeed");
        assert!(backup_path.exists());
    }

    #[tokio::test]
    async fn run_restore_from_backup() {
        // Produce a backup from an initialized source storage...
        let source_base = TempDir::new().unwrap();
        let source = initialized_storage(&source_base).await;
        let backup_path = source_base.path().join("backup");
        run(test_cli(Commands::Backup {
            storage: source,
            mode: CliBackupMode::Directory,
            path: Some(backup_path.clone()),
        }))
        .await
        .unwrap();

        // ...then restore it into a fresh (empty) target storage.
        let target_base = TempDir::new().unwrap();
        let target = initialized_storage(&target_base).await;
        run(test_cli(Commands::Restore {
            storage: target,
            path: backup_path,
        }))
        .await
        .expect("restore dispatch should succeed");
    }
}
