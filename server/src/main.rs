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
        Commands::Serve { .. } => {
            todo!("M1.5: implement cmd_serve")
        }
    }
    Ok(())
}
