use clap::Parser;
use jaunder::cli::{Cli, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    run(cli).await
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Init {
            storage,
            skip_if_exists,
        } => {
            jaunder::commands::cmd_init(&storage, skip_if_exists).await?;
        }
        Commands::Serve {
            storage,
            bind,
            environment,
        } => {
            jaunder::commands::cmd_serve(&storage, bind, environment.is_prod()).await?;
        }
        Commands::UserCreate {
            storage,
            username,
            password,
            display_name,
        } => {
            let username = username
                .parse::<jaunder::username::Username>()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let password = password
                .map(|p| p.parse::<jaunder::password::Password>())
                .transpose()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            jaunder::commands::cmd_user_create(
                &storage,
                &username,
                password,
                display_name.as_deref(),
            )
            .await?;
        }
        Commands::UserInvite {
            storage,
            expires_in,
        } => {
            jaunder::commands::cmd_user_invite(&storage, expires_in).await?;
        }
        Commands::SmtpTest { storage, to } => {
            jaunder::commands::cmd_smtp_test(&storage, &to).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jaunder::cli::{Cli, Commands, StorageArgs};
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
    async fn run_init() {
        let base = TempDir::new().unwrap();
        let cli = Cli {
            command: Commands::Init {
                storage: test_storage_args(&base),
                skip_if_exists: false,
            },
        };
        run(cli).await.unwrap();
    }

    #[tokio::test]
    async fn run_user_create() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        run(Cli {
            command: Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            },
        })
        .await
        .unwrap();

        let cli = Cli {
            command: Commands::UserCreate {
                storage,
                username: "alice".to_string(),
                password: Some("password123".to_string()),
                display_name: None,
            },
        };
        run(cli).await.unwrap();
    }

    #[tokio::test]
    async fn run_user_invite() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        run(Cli {
            command: Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            },
        })
        .await
        .unwrap();

        let cli = Cli {
            command: Commands::UserInvite {
                storage,
                expires_in: None,
            },
        };
        run(cli).await.unwrap();
    }

    #[tokio::test]
    async fn run_smtp_test_fails_when_smtp_not_configured() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        run(Cli {
            command: Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            },
        })
        .await
        .unwrap();

        let cli = Cli {
            command: Commands::SmtpTest {
                storage,
                to: "alice@example.com".to_string(),
            },
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
    async fn run_serve() {
        let base = TempDir::new().unwrap();
        let storage = test_storage_args(&base);
        run(Cli {
            command: Commands::Init {
                storage: storage.clone(),
                skip_if_exists: false,
            },
        })
        .await
        .unwrap();

        let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let cli = Cli {
            command: Commands::Serve {
                storage,
                bind,
                environment: jaunder::cli::DeploymentEnv::Dev,
            },
        };

        // We can't easily run the server indefinitely in a test that expects
        // completion, but we can spawn it and abort it, similar to theSuccess
        // test in commands.rs.
        let task = tokio::spawn(async move {
            let _ = run(cli).await;
        });

        // Wait a bit for it to start.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        task.abort();
    }
}
