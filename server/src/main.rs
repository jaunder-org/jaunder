use clap::Parser;
use jaunder::cli::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Fail-closed: a production binary must never link a `common` compiled with
    // cheap test KDF params. Feature isolation (resolver 2, dev-deps only) keeps
    // this false in production; if it is ever true, refuse to start rather than
    // hash passwords weakly. main() is never run by the integration tests, so this
    // does not affect the test build.
    if common::CHEAP_KDF_ENABLED {
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
    // `run` owns telemetry for *every* command, `serve` included: one guard,
    // held across the whole dispatch, whose Drop flushes the OTLP exporters
    // before exit. For a one-shot command that means before the process returns
    // (on success, `?` error, or panic unwind); for `serve` the guard is simply
    // held for the process lifetime and flushes at shutdown. `cmd_serve` no
    // longer inits telemetry itself, so all commands share this one mechanism.
    // Bound after command resolution so the no-subcommand `--help` path exits via
    // clap without initializing telemetry it would never use; bound at function
    // scope so the guard outlives the dispatch below.
    let _telemetry = jaunder::observability::init_tracing(cli.verbose);
    command.execute().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use jaunder::cli::{Cli, CliBackupMode, Commands, PgBootstrapArgs, StorageArgs};
    use tempfile::TempDir;

    fn test_storage_args(base: &TempDir) -> StorageArgs {
        StorageArgs {
            storage_path: base.path().join("storage"),
            db: format!("sqlite:{}", base.path().join("test.db").display())
                .parse()
                .unwrap(),
        }
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
        let storage = test_storage_args(&base);
        run(Cli {
            command: Some(Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            }),
            verbose: false,
        })
        .await
        .unwrap();

        let cli = Cli {
            command: Some(Commands::UserCreate {
                storage,
                username: "alice".to_string(),
                password: Some("password123".to_string()),
                display_name: None,
                operator: false,
            }),
            verbose: false,
        };
        run(cli).await.unwrap();
    }

    #[tokio::test]
    async fn run_user_invite() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        run(Cli {
            command: Some(Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            }),
            verbose: false,
        })
        .await
        .unwrap();

        let cli = Cli {
            command: Some(Commands::UserInvite {
                storage,
                expires_in: None,
            }),
            verbose: false,
        };
        run(cli).await.unwrap();
    }

    #[tokio::test]
    async fn run_smtp_test_fails_when_smtp_not_configured() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        run(Cli {
            command: Some(Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            }),
            verbose: false,
        })
        .await
        .unwrap();

        let cli = Cli {
            command: Some(Commands::SmtpTest {
                storage,
                to: "alice@example.com".to_string(),
            }),
            verbose: false,
        };
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
        let storage = test_storage_args(&base);
        run(Cli {
            command: Some(Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            }),
            verbose: false,
        })
        .await
        .unwrap();
        run(Cli {
            command: Some(Commands::UserCreate {
                storage: storage.clone(),
                username: "alice".to_string(),
                password: Some("password123".to_string()),
                display_name: None,
                operator: false,
            }),
            verbose: false,
        })
        .await
        .unwrap();

        run(Cli {
            command: Some(Commands::AppPasswordCreate {
                storage,
                username: "alice".to_string(),
                label: "ert".to_string(),
            }),
            verbose: false,
        })
        .await
        .expect("app-password-create should succeed for an existing user");
    }

    #[tokio::test]
    async fn run_serve() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        run(Cli {
            command: Some(Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            }),
            verbose: false,
        })
        .await
        .unwrap();

        let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let cli = Cli {
            command: Some(Commands::Serve {
                storage,
                bind,
                environment: jaunder::cli::DeploymentEnv::Dev,
                runtime_file: None,
            }),
            verbose: false,
        };

        // We can't easily run the server indefinitely in a test that expects
        // completion, but we can spawn it and abort it, similar to theSuccess
        // test in commands.rs.
        let task = tokio::spawn(async move {
            let _ = run(cli).await;
        }); // cov:ignore

        // Wait a bit for it to start.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        task.abort();
    }

    #[tokio::test]
    async fn run_create_pg_db_rejects_non_postgres_urls() {
        let cli = Cli {
            command: Some(Commands::CreatePgDb {
                pg: PgBootstrapArgs {
                    bootstrap_db: "sqlite:/tmp/bootstrap.db".to_owned(),
                    app_db: "postgres://jaunder@localhost/jaunder".to_owned(),
                    app_role_password: std::iter::repeat_n('z', 20).collect(),
                },
            }),
            verbose: false,
        };

        let err = run(cli).await.unwrap_err();
        assert!(err.to_string().contains("PostgreSQL URL"));
    }

    #[tokio::test]
    async fn run_user_create_rejects_invalid_username() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        let cli = Cli {
            command: Some(Commands::UserCreate {
                storage,
                username: "invalid username".to_string(),
                password: Some("password123".to_string()),
                display_name: None,
                operator: false,
            }),
            verbose: false,
        };
        let err = run(cli).await.unwrap_err();
        assert!(err.to_string().contains("username must be non-empty"));
    }

    #[tokio::test]
    async fn run_user_create_rejects_invalid_password() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        let cli = Cli {
            command: Some(Commands::UserCreate {
                storage,
                username: "alice".to_string(),
                password: Some("short".to_string()),
                display_name: None,
                operator: false,
            }),
            verbose: false,
        };
        let err = run(cli).await.unwrap_err();
        assert!(err
            .to_string()
            .contains("password must be at least 8 characters"));
    }

    #[tokio::test]
    async fn run_init_fails_on_invalid_path() {
        let base = TempDir::new().unwrap();
        // Create a file where the storage directory should be
        let storage_path = base.path().join("file");
        std::fs::write(&storage_path, "not a dir").unwrap();

        let cli = Cli {
            command: Some(Commands::Init {
                storage: StorageArgs {
                    storage_path: storage_path.clone(),
                    db: format!("sqlite:{}", base.path().join("test.db").display())
                        .parse()
                        .unwrap(),
                },
                skip_if_exists: false,
            }),
            verbose: false,
        };
        let err = run(cli).await.unwrap_err();
        assert!(
            err.to_string().contains("Not a directory") || err.to_string().contains("File exists")
        );
    }

    #[tokio::test]
    async fn run_serve_prod_fails_when_uninitialized() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        let cli = Cli {
            command: Some(Commands::Serve {
                storage,
                bind: "127.0.0.1:0".parse().unwrap(),
                environment: jaunder::cli::DeploymentEnv::Prod,
                runtime_file: None,
            }),
            verbose: false,
        };
        let err = run(cli).await.unwrap_err();
        assert!(err.to_string().contains("run `jaunder init` first"));
    }

    #[tokio::test]
    async fn run_serve_dev_auto_inits() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let cli = Cli {
            command: Some(Commands::Serve {
                storage,
                bind,
                environment: jaunder::cli::DeploymentEnv::Dev,
                runtime_file: None,
            }),
            verbose: false,
        };

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
        let storage = test_storage_args(&base);
        run(Cli {
            command: Some(Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            }),
            verbose: false,
        })
        .await
        .unwrap();

        // `cmd_backup` creates the artifact itself, so no prior backup is needed.
        let backup_path = base.path().join("backup");
        run(Cli {
            command: Some(Commands::Backup {
                storage,
                mode: CliBackupMode::Directory,
                path: Some(backup_path.clone()),
            }),
            verbose: false,
        })
        .await
        .expect("backup dispatch should succeed");
        assert!(backup_path.exists());
    }

    #[tokio::test]
    async fn run_restore_from_backup() {
        // Produce a backup from an initialized source storage...
        let source_base = TempDir::new().unwrap();
        let source = test_storage_args(&source_base);
        run(Cli {
            command: Some(Commands::Init {
                storage: source.clone(),
                skip_if_exists: false,
            }),
            verbose: false,
        })
        .await
        .unwrap();
        let backup_path = source_base.path().join("backup");
        run(Cli {
            command: Some(Commands::Backup {
                storage: source,
                mode: CliBackupMode::Directory,
                path: Some(backup_path.clone()),
            }),
            verbose: false,
        })
        .await
        .unwrap();

        // ...then restore it into a fresh (empty) target storage.
        let target_base = TempDir::new().unwrap();
        let target = test_storage_args(&target_base);
        run(Cli {
            command: Some(Commands::Init {
                storage: target.clone(),
                skip_if_exists: false,
            }),
            verbose: false,
        })
        .await
        .unwrap();
        run(Cli {
            command: Some(Commands::Restore {
                storage: target,
                path: backup_path,
            }),
            verbose: false,
        })
        .await
        .expect("restore dispatch should succeed");
    }
}
