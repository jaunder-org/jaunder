use std::{net::SocketAddr, path::PathBuf};

use clap::{Args, Parser, Subcommand};

use server::storage::DbConnectOptions;

#[derive(Parser)]
#[command(name = "jaunder", about = "A self-hosted social reader")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Arguments shared by subcommands that need access to the storage directory.
#[derive(Args)]
pub struct StorageArgs {
    /// Path to the storage directory (media, backups).
    #[arg(long, env = "JAUNDER_STORAGE_PATH", default_value = "./data")]
    pub storage_path: PathBuf,

    /// Database URL.
    ///
    /// Only `sqlite:` URLs are supported until M20 adds PostgreSQL.
    #[arg(long, env = "JAUNDER_DB", default_value = "sqlite:./data/jaunder.db")]
    pub db: DbConnectOptions,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize the storage directory and database.
    ///
    /// Creates the storage directory, required subdirectories (media/, backups/),
    /// and the SQLite database with the initial schema. Run this once before
    /// starting the server for the first time.
    Init {
        #[command(flatten)]
        storage: StorageArgs,

        /// Succeed silently if the instance is already initialized.
        /// Useful in scripts and container entrypoints.
        #[arg(long)]
        skip_if_exists: bool,
    },
    /// Start the HTTP server.
    ///
    /// The storage directory must already be initialized via `jaunder init`.
    Serve {
        #[command(flatten)]
        storage: StorageArgs,

        /// Address and port to bind to.
        #[arg(long, env = "JAUNDER_BIND", default_value = "127.0.0.1:3000")]
        bind: SocketAddr,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("jaunder").chain(args.iter().copied()))
            .expect("parse failed")
    }

    // --- storage_path precedence ---

    #[test]
    fn storage_path_default() {
        let cli = parse(&["init"]);
        let Commands::Init { storage, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(storage.storage_path, PathBuf::from("./data"));
    }

    #[test]
    fn storage_path_from_flag() {
        let cli = parse(&["init", "--storage-path", "/tmp/mydata"]);
        let Commands::Init { storage, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(storage.storage_path, PathBuf::from("/tmp/mydata"));
    }

    #[test]
    fn storage_path_flag_beats_env() {
        std::env::set_var("JAUNDER_STORAGE_PATH", "/tmp/from_env");
        let cli = parse(&["init", "--storage-path", "/tmp/from_flag"]);
        std::env::remove_var("JAUNDER_STORAGE_PATH");
        let Commands::Init { storage, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(storage.storage_path, PathBuf::from("/tmp/from_flag"));
    }

    #[test]
    fn storage_path_env_beats_default() {
        std::env::set_var("JAUNDER_STORAGE_PATH", "/tmp/from_env");
        let cli = parse(&["init"]);
        std::env::remove_var("JAUNDER_STORAGE_PATH");
        let Commands::Init { storage, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(storage.storage_path, PathBuf::from("/tmp/from_env"));
    }

    // --- bind precedence ---

    #[test]
    fn bind_default() {
        let cli = parse(&["serve"]);
        let Commands::Serve { bind, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(bind, "127.0.0.1:3000".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn bind_from_flag() {
        let cli = parse(&["serve", "--bind", "0.0.0.0:8080"]);
        let Commands::Serve { bind, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(bind, "0.0.0.0:8080".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn bind_flag_beats_env() {
        std::env::set_var("JAUNDER_BIND", "0.0.0.0:9000");
        let cli = parse(&["serve", "--bind", "0.0.0.0:8080"]);
        std::env::remove_var("JAUNDER_BIND");
        let Commands::Serve { bind, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(bind, "0.0.0.0:8080".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn bind_env_beats_default() {
        std::env::set_var("JAUNDER_BIND", "0.0.0.0:9000");
        let cli = parse(&["serve"]);
        std::env::remove_var("JAUNDER_BIND");
        let Commands::Serve { bind, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(bind, "0.0.0.0:9000".parse::<SocketAddr>().unwrap());
    }

    // --- skip_if_exists flag ---

    #[test]
    fn skip_if_exists_defaults_false() {
        let cli = parse(&["init"]);
        let Commands::Init { skip_if_exists, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert!(!skip_if_exists);
    }

    #[test]
    fn skip_if_exists_flag_sets_true() {
        let cli = parse(&["init", "--skip-if-exists"]);
        let Commands::Init { skip_if_exists, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert!(skip_if_exists);
    }

    // --- db precedence ---

    #[test]
    fn db_default() {
        let cli = parse(&["init"]);
        let Commands::Init { storage, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(storage.db.to_string(), "sqlite:./data/jaunder.db");
    }

    #[test]
    fn db_from_flag() {
        let cli = parse(&["init", "--db", "sqlite:/tmp/test.db"]);
        let Commands::Init { storage, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(storage.db.to_string(), "sqlite:/tmp/test.db");
    }

    #[test]
    fn db_flag_beats_env() {
        std::env::set_var("JAUNDER_DB", "sqlite:/tmp/from_env.db");
        let cli = parse(&["init", "--db", "sqlite:/tmp/from_flag.db"]);
        std::env::remove_var("JAUNDER_DB");
        let Commands::Init { storage, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(storage.db.to_string(), "sqlite:/tmp/from_flag.db");
    }

    #[test]
    fn db_env_beats_default() {
        std::env::set_var("JAUNDER_DB", "sqlite:/tmp/from_env.db");
        let cli = parse(&["init"]);
        std::env::remove_var("JAUNDER_DB");
        let Commands::Init { storage, .. } = cli.command else {
            panic!("wrong variant");
        };
        assert_eq!(storage.db.to_string(), "sqlite:/tmp/from_env.db");
    }
}
