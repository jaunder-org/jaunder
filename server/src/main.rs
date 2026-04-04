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
        Commands::UserCreate { .. } => {
            todo!("user create not yet implemented")
        }
        Commands::UserInvite { .. } => {
            todo!("user invite not yet implemented")
        }
    }
    Ok(())
}
