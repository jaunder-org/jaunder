use clap::Parser;
use server::cli::{Cli, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init {
            storage,
            skip_if_exists,
        } => {
            server::commands::cmd_init(&storage, skip_if_exists).await?;
        }
        Commands::Serve { storage, bind } => {
            server::commands::cmd_serve(&storage, bind).await?;
        }
        Commands::UserCreate {
            storage,
            username,
            password,
            display_name,
        } => {
            server::commands::cmd_user_create(
                &storage,
                &username,
                password.as_deref(),
                display_name.as_deref(),
            )
            .await?;
        }
        Commands::UserInvite {
            storage,
            expires_in,
        } => {
            server::commands::cmd_user_invite(&storage, expires_in).await?;
        }
    }
    Ok(())
}
