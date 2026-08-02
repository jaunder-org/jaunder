use std::fmt;
use std::str::FromStr;
use std::{net::SocketAddr, path::PathBuf};

use clap::{Args, Parser, Subcommand};
use sqlx::postgres::PgConnectOptions;

use common::backup::BackupMode;
use common::display_name::DisplayName;
use common::invite::InviteTtlHours;
use common::pg_identifier::{InvalidPgDatabaseName, InvalidPgRoleName, PgDatabaseName, PgRoleName};
use common::pg_role_password::PgRolePassword;
use common::username::Username;
use storage::DbConnectOptions;

#[derive(Parser, Clone)]
#[command(name = "jaunder", about = "A self-hosted social reader")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Enable verbose logging (DEBUG level for application crates).
    #[arg(long, global = true, env = "JAUNDER_VERBOSE")]
    pub verbose: bool,
}

/// Arguments shared by subcommands that need access to the storage directory.
#[derive(Args, Clone)]
pub struct StorageArgs {
    /// Path to the storage directory (media, backups).
    #[arg(long, env = "JAUNDER_STORAGE_PATH", default_value = "./data")]
    pub storage_path: PathBuf,

    /// Database URL.
    ///
    /// Supports `sqlite:` and `postgres://` URLs.
    /// `PostgreSQL` passwords may also be supplied via `JAUNDER_DB_PASSWORD` or
    /// `JAUNDER_DB_PASSWORD_FILE`.
    #[arg(long, env = "JAUNDER_DB", default_value = "sqlite:./data/jaunder.db")]
    pub db: DbConnectOptions,
}

/// A URL whose scheme is not `PostgreSQL`.
#[derive(Debug, thiserror::Error)]
#[error("{flag} must be a PostgreSQL URL")]
pub struct InvalidPgUrl {
    flag: &'static str,
}

/// Rejects a URL whose scheme is not `PostgreSQL`.
///
/// **sqlx does not do this.** `PgConnectOptions::from_str("sqlite:/tmp/jaunder.db")`
/// *succeeds*: with no authority the username falls back to the OS user and the path
/// becomes the database, yielding `role="mdorman", database="tmp/jaunder.db"`. So a
/// mistyped scheme would silently provision a role named after whoever ran the command.
/// The scheme check in [`DbConnectOptions::from_str`] is load-bearing, and anything that
/// parses straight to `PgConnectOptions` has to repeat it.
fn require_postgres_scheme(s: &str, flag: &'static str) -> Result<(), InvalidPgUrl> {
    if s.starts_with("postgres://") || s.starts_with("postgresql://") {
        Ok(())
    } else {
        Err(InvalidPgUrl { flag })
    }
}

/// The superuser connection `create-pg-db` provisions with — the only argument whose
/// full connection options the command actually needs, because it is the one we connect
/// with. See [`require_postgres_scheme`] for why this is a newtype and not a bare
/// `PgConnectOptions`.
///
/// No `Debug`: `PgConnectOptions` carries the bootstrap password.
#[derive(Clone)]
pub struct BootstrapDb(PgConnectOptions);

impl BootstrapDb {
    /// The superuser connection options.
    #[must_use]
    pub fn options(&self) -> &PgConnectOptions {
        &self.0
    }
}

/// Why a `--bootstrap-db` value is unusable.
#[derive(Debug, thiserror::Error)]
pub enum InvalidBootstrapDb {
    /// Not a `PostgreSQL` scheme.
    #[error(transparent)]
    Scheme(#[from] InvalidPgUrl),
    /// Right scheme, but unparseable.
    #[error("--bootstrap-db must be a PostgreSQL URL: {0}")]
    Url(#[from] sqlx::Error),
}

impl FromStr for BootstrapDb {
    type Err = InvalidBootstrapDb;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        require_postgres_scheme(s, "--bootstrap-db")?;
        Ok(BootstrapDb(s.parse()?))
    }
}

/// The application role and database named by `--app-db`.
///
/// The URL is parsed here, at the CLI boundary, and **only these two identifiers are
/// kept**: `create-pg-db` never connects to the application database — it creates a
/// database and a role to own it, and reads nothing else from that URL. Holding a whole
/// `PgConnectOptions` would carry several hundred bytes of connection state the command
/// never touches, and would let a non-`PostgreSQL` URL, or one with no database name,
/// survive until the command rejected it a layer later.
///
/// A useful consequence: because the URL is discarded, `AppTarget` **cannot** carry the
/// password an `--app-db` URL may contain, so `Debug` here is safe to derive.
#[derive(Clone, Debug)]
pub struct AppTarget {
    role: PgRoleName,
    database: PgDatabaseName,
}

impl AppTarget {
    /// The role that will own the application database.
    #[must_use]
    pub fn role(&self) -> &PgRoleName {
        &self.role
    }

    /// The application database to create.
    #[must_use]
    pub fn database(&self) -> &PgDatabaseName {
        &self.database
    }
}

/// Why an `--app-db` value is not a usable [`AppTarget`].
#[derive(Debug, thiserror::Error)]
pub enum InvalidAppTarget {
    /// Not a `PostgreSQL` scheme.
    #[error(transparent)]
    Scheme(#[from] InvalidPgUrl),
    /// Right scheme, but unparseable.
    #[error("--app-db must be a PostgreSQL URL: {0}")]
    Url(#[from] sqlx::Error),
    /// Parseable, but names no database.
    #[error("--app-db must include a PostgreSQL database name")]
    MissingDatabase,
    /// The role name failed its invariant.
    #[error(transparent)]
    Role(#[from] InvalidPgRoleName),
    /// The database name failed its invariant.
    #[error(transparent)]
    Database(#[from] InvalidPgDatabaseName),
}

impl FromStr for AppTarget {
    type Err = InvalidAppTarget;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        require_postgres_scheme(s, "--app-db")?;
        let options: PgConnectOptions = s.parse()?;
        Ok(AppTarget {
            role: options.get_username().parse()?,
            database: options
                .get_database()
                .ok_or(InvalidAppTarget::MissingDatabase)?
                .parse()?,
        })
    }
}

/// The three arguments were once three adjacent `String`s — any permutation parsed, and
/// the middle one is a credential (#693). Three distinct types now, so a transposition is
/// a compile error and each value validates at the parse boundary.
///
/// Only `bootstrap_db` carries connection options, because it is the only one we connect
/// with. See [`AppTarget`] for why `--app-db` keeps two identifiers instead.
#[derive(Args, Clone)]
pub struct PgBootstrapArgs {
    /// `PostgreSQL` URL for a bootstrap/superuser role.
    ///
    /// This command only supports `PostgreSQL` URLs.
    #[arg(long)]
    pub bootstrap_db: BootstrapDb,

    /// `PostgreSQL` URL for the long-term application role and target database.
    #[arg(long = "app-db")]
    pub app_db: AppTarget,

    /// Password to set on the application role being created.
    #[arg(long)]
    pub app_role_password: PgRolePassword,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum CliBackupMode {
    Directory,
    Archive,
}

impl From<CliBackupMode> for BackupMode {
    fn from(m: CliBackupMode) -> Self {
        match m {
            CliBackupMode::Directory => BackupMode::Directory,
            CliBackupMode::Archive => BackupMode::Archive,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum DeploymentEnv {
    Dev,
    Prod,
}

impl DeploymentEnv {
    #[must_use]
    pub fn is_prod(self) -> bool {
        matches!(self, DeploymentEnv::Prod)
    }
}

impl fmt::Display for DeploymentEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeploymentEnv::Dev => write!(f, "dev"),
            DeploymentEnv::Prod => write!(f, "prod"),
        }
    }
}

#[derive(Subcommand, Clone)]
pub enum Commands {
    /// Initialize the storage directory and database.
    ///
    /// Creates the storage directory, required subdirectories (media/, backups/),
    /// creates the configured database where supported by the selected backend,
    /// and applies the initial schema. Run this once before starting the
    /// server for the first time.
    Init {
        #[command(flatten)]
        storage: StorageArgs,

        /// Succeed silently if the instance is already initialized.
        /// Useful in scripts and container entrypoints.
        #[arg(long)]
        skip_if_exists: bool,
    },
    /// Create a `PostgreSQL` application role and database using bootstrap credentials.
    ///
    /// Intended for one-time administrative provisioning. This is separate from
    /// `jaunder init`, which assumes the target database already exists and only
    /// initializes storage plus schema state. Fails if the requested role or
    /// database already exists rather than trying to repair or modify them.
    CreatePgDb {
        #[command(flatten)]
        pg: PgBootstrapArgs,
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

        /// Deployment environment.
        #[arg(long, env = "JAUNDER_ENV", default_value_t = DeploymentEnv::Dev)]
        environment: DeploymentEnv,

        /// Path to write the runtime-info JSON file (default
        /// `<storage-path>/runtime.json`). Records the bound `ip`/`port`.
        #[arg(long, env = "JAUNDER_RUNTIME_FILE")]
        runtime_file: Option<PathBuf>,
    },

    /// Create a user account directly, bypassing the registration policy.
    ///
    /// Intended for bootstrapping an initial operator account. The storage
    /// directory must already be initialized via `jaunder init`.
    UserCreate {
        #[command(flatten)]
        storage: StorageArgs,

        /// Username for the new account (must match [a-z0-9_-]+).
        #[arg(long)]
        username: Username,

        /// Password for the new account. If omitted, you will be prompted
        /// interactively (input is hidden).
        #[arg(long)]
        password: Option<String>,

        /// Optional display name.
        #[arg(long)]
        display_name: Option<DisplayName>,

        /// Mark the user as an operator with administrative privileges.
        #[arg(long)]
        operator: bool,
    },

    /// Mint an app password (session token) for a user and print it to stdout.
    ///
    /// The storage directory must already be initialized via `jaunder init`.
    AppPasswordCreate {
        #[command(flatten)]
        storage: StorageArgs,

        /// Username to mint the app password for.
        #[arg(long)]
        username: Username,

        /// Label recorded with the session (shown in the sessions UI).
        #[arg(long, default_value = "app-password")]
        label: String,
    },

    /// Generate an invite code.
    ///
    /// The storage directory must already be initialized via `jaunder init`.
    UserInvite {
        #[command(flatten)]
        storage: StorageArgs,

        /// Hours until the invite code expires (1..=336). Defaults to 168 (7 days).
        #[arg(long)]
        expires_in: Option<InviteTtlHours>,
    },

    /// Send a test email via the configured SMTP relay.
    ///
    /// Loads SMTP configuration from the database and sends a test message to
    /// the given address. Returns an error if SMTP is not configured.
    /// The storage directory must already be initialized via `jaunder init`.
    SmtpTest {
        #[command(flatten)]
        storage: StorageArgs,

        /// Email address to send the test message to.
        #[arg(long)]
        to: String,
    },

    /// Immediately run a backup.
    Backup {
        #[command(flatten)]
        storage: StorageArgs,

        /// Backup format to write.
        #[arg(long, value_enum, default_value = "directory")]
        mode: CliBackupMode,

        /// Destination directory or .tar.gz archive path for this backup.
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Restore from a backup archive or directory.
    Restore {
        #[command(flatten)]
        storage: StorageArgs,

        /// Backup archive or directory to restore from.
        #[arg(required = true)]
        path: PathBuf,
    },

    /// Read or write `site_config` key/value entries (set/get/list).
    ///
    /// The storage directory must already be initialized via `jaunder init`.
    SiteConfig {
        #[command(subcommand)]
        action: SiteConfigAction,
    },
}

/// `site-config` leaf actions. `site_config` is a free-form key/value store;
/// values are not validated. Known keys include `site.registration_policy`,
/// `site.title`, `site.base_url`, `feeds.websub_hub_url`, `feeds.min_items`,
/// `feeds.min_days`, `posts.default_audience`, and the `backup.*` keys.
#[derive(Subcommand, Clone)]
pub enum SiteConfigAction {
    /// Set (upsert) a key to a value.
    Set {
        #[command(flatten)]
        storage: StorageArgs,

        /// The `site_config` key (e.g. `feeds.websub_hub_url`).
        key: String,

        /// The value to store (free-form; a leading `-` is allowed).
        #[arg(allow_hyphen_values = true)]
        value: String,
    },

    /// Print the value for a key (nothing, and a non-zero exit, if unset).
    Get {
        #[command(flatten)]
        storage: StorageArgs,

        /// The `site_config` key to read.
        key: String,
    },

    /// Print every entry as `key=value`, one per line, ordered by key.
    List {
        #[command(flatten)]
        storage: StorageArgs,
    },

    /// Delete a key. Idempotent: unsetting an absent key is a no-op (exit 0).
    Unset {
        #[command(flatten)]
        storage: StorageArgs,

        /// The `site_config` key to delete.
        key: String,
    },
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use common::test_support::{parse_display_name, parse_invite_ttl_hours};

    /// Serializes all tests that read or write the env vars clap resolves at parse time.
    /// `cargo test` runs tests in parallel threads within the same process, so concurrent
    /// `set_var/remove_var` calls race against each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("jaunder").chain(args.iter().copied()))
            .expect("parse failed")
    }

    // --- storage_path precedence ---

    #[test]
    fn create_pg_db_parses_bootstrap_and_target_urls() {
        let cli = parse(&[
            "create-pg-db",
            "--bootstrap-db",
            "postgres://postgres@localhost/postgres",
            "--app-db",
            "postgres://jaunder@localhost/jaunder",
            "--app-role-password",
            "secret",
        ]);
        let Commands::CreatePgDb { pg } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::CreatePgDb")
        };
        // Asserted on the parsed values, not a rendered URL — so this test has no
        // dependence on how a connection string is formatted or redacted.
        assert_eq!(pg.bootstrap_db.options().get_username(), "postgres");
        assert_eq!(pg.bootstrap_db.options().get_database(), Some("postgres"));
        assert_eq!(pg.app_db.role().to_string(), "jaunder");
        assert_eq!(pg.app_db.database().to_string(), "jaunder");
        // `as_ref()`, not `==`: the secret surface has no `PartialEq` by design.
        assert_eq!(pg.app_role_password.as_ref(), "secret");
    }

    #[test]
    fn app_target_extracts_role_and_database() {
        let target: AppTarget = "postgres://jaunder@localhost/jaunder_db".parse().unwrap();
        assert_eq!(target.role().to_string(), "jaunder");
        assert_eq!(target.database().to_string(), "jaunder_db");
    }

    // Replaces `cmd_create_pg_db_rejects_non_postgres_app_db`: the rejection moved to the
    // parse boundary, so a non-PostgreSQL `--app-db` can no longer reach the command.
    //
    // The scheme check this drives is **not** redundant with sqlx. `PgConnectOptions`
    // parses "sqlite:/tmp/jaunder.db" *successfully* — with no authority it defaults the
    // role to the OS user and takes "tmp/jaunder.db" as the database. Without the explicit
    // check, this input would provision a role named after whoever ran the command.
    #[test]
    fn app_target_rejects_a_non_postgres_url() {
        let err = "sqlite:/tmp/jaunder.db".parse::<AppTarget>().unwrap_err();
        assert!(err
            .to_string()
            .contains("--app-db must be a PostgreSQL URL"));
    }

    #[test]
    fn bootstrap_db_rejects_a_non_postgres_url() {
        // `.err()` rather than `.unwrap_err()`: the latter needs `BootstrapDb: Debug`,
        // which it deliberately does not have — it holds the bootstrap password.
        let err = "sqlite:/tmp/bootstrap.db"
            .parse::<BootstrapDb>()
            .err()
            .expect("a sqlite URL is not a bootstrap URL");
        assert!(err
            .to_string()
            .contains("--bootstrap-db must be a PostgreSQL URL"));
    }

    // Replaces `cmd_create_pg_db_requires_database_name`, for the same reason.
    #[test]
    fn app_target_rejects_a_url_with_no_database() {
        let err = "postgres://app@localhost".parse::<AppTarget>().unwrap_err();
        assert!(err
            .to_string()
            .contains("--app-db must include a PostgreSQL database name"));
    }

    // Replaces `run_create_pg_db_rejects_non_postgres_urls` in main.rs: `bootstrap_db` is
    // a `PgConnectOptions`, so a non-PostgreSQL bootstrap URL cannot be constructed —
    // clap rejects it during argument parsing rather than the command rejecting it later.
    #[test]
    fn create_pg_db_rejects_a_non_postgres_bootstrap_url() {
        let result = Cli::try_parse_from([
            "jaunder",
            "create-pg-db",
            "--bootstrap-db",
            "sqlite:/tmp/bootstrap.db",
            "--app-db",
            "postgres://jaunder@localhost/jaunder",
            "--app-role-password",
            "secret",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn storage_path_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("JAUNDER_STORAGE_PATH");
        let cli = parse(&["init"]);
        let Commands::Init { storage, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert_eq!(storage.storage_path, PathBuf::from("./data"));
    }

    #[test]
    fn storage_path_from_flag() {
        let _guard = ENV_LOCK.lock().unwrap();
        let cli = parse(&["init", "--storage-path", "/tmp/mydata"]);
        let Commands::Init { storage, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert_eq!(storage.storage_path, PathBuf::from("/tmp/mydata"));
    }

    #[test]
    fn storage_path_flag_beats_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_STORAGE_PATH", "/tmp/from_env");
        let cli = parse(&["init", "--storage-path", "/tmp/from_flag"]);
        std::env::remove_var("JAUNDER_STORAGE_PATH");
        let Commands::Init { storage, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert_eq!(storage.storage_path, PathBuf::from("/tmp/from_flag"));
    }

    #[test]
    fn storage_path_env_beats_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_STORAGE_PATH", "/tmp/from_env");
        let cli = parse(&["init"]);
        std::env::remove_var("JAUNDER_STORAGE_PATH");
        let Commands::Init { storage, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert_eq!(storage.storage_path, PathBuf::from("/tmp/from_env"));
    }

    // --- bind precedence ---

    #[test]
    fn bind_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("JAUNDER_BIND");
        let cli = parse(&["serve"]);
        let Commands::Serve { bind, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Serve")
        };
        assert_eq!(bind, "127.0.0.1:3000".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn bind_from_flag() {
        let _guard = ENV_LOCK.lock().unwrap();
        let cli = parse(&["serve", "--bind", "0.0.0.0:8080"]);
        let Commands::Serve { bind, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Serve")
        };
        assert_eq!(bind, "0.0.0.0:8080".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn bind_flag_beats_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_BIND", "0.0.0.0:9000");
        let cli = parse(&["serve", "--bind", "0.0.0.0:8080"]);
        std::env::remove_var("JAUNDER_BIND");
        let Commands::Serve { bind, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Serve")
        };
        assert_eq!(bind, "0.0.0.0:8080".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn bind_env_beats_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_BIND", "0.0.0.0:9000");
        let cli = parse(&["serve"]);
        std::env::remove_var("JAUNDER_BIND");
        let Commands::Serve { bind, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Serve")
        };
        assert_eq!(bind, "0.0.0.0:9000".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn environment_defaults_dev() {
        let _guard = ENV_LOCK.lock().unwrap();
        let cli = parse(&["serve"]);
        let Commands::Serve { environment, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Serve")
        };
        assert_eq!(environment, DeploymentEnv::Dev);
    }

    #[test]
    fn environment_from_flag() {
        let _guard = ENV_LOCK.lock().unwrap();
        let cli = parse(&["serve", "--environment", "prod"]);
        let Commands::Serve { environment, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Serve")
        };
        assert_eq!(environment, DeploymentEnv::Prod);
    }

    #[test]
    fn environment_env_beats_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_ENV", "prod");
        let cli = parse(&["serve"]);
        std::env::remove_var("JAUNDER_ENV");
        let Commands::Serve { environment, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Serve")
        };
        assert_eq!(environment, DeploymentEnv::Prod);
    }

    // --- skip_if_exists flag ---

    #[test]
    fn skip_if_exists_defaults_false() {
        let cli = parse(&["init"]);
        let Commands::Init { skip_if_exists, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert!(!skip_if_exists);
    }

    #[test]
    fn skip_if_exists_flag_sets_true() {
        let cli = parse(&["init", "--skip-if-exists"]);
        let Commands::Init { skip_if_exists, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert!(skip_if_exists);
    }

    // --- db precedence ---

    #[test]
    fn db_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("JAUNDER_DB");
        let cli = parse(&["init"]);
        let Commands::Init { storage, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert_eq!(storage.db.to_string(), "sqlite:./data/jaunder.db");
    }

    #[test]
    fn db_from_flag() {
        let _guard = ENV_LOCK.lock().unwrap();
        let cli = parse(&["init", "--db", "sqlite:/tmp/test.db"]);
        let Commands::Init { storage, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert_eq!(storage.db.to_string(), "sqlite:/tmp/test.db");
    }

    #[test]
    fn postgres_db_from_flag() {
        let _guard = ENV_LOCK.lock().unwrap();
        let cli = parse(&["init", "--db", "postgres://jaunder@localhost/testdb"]);
        let Commands::Init { storage, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert_eq!(
            storage.db.to_string(),
            "postgres://jaunder@localhost/testdb"
        );
    }

    #[test]
    fn db_flag_beats_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_DB", "sqlite:/tmp/from_env.db");
        let cli = parse(&["init", "--db", "sqlite:/tmp/from_flag.db"]);
        std::env::remove_var("JAUNDER_DB");
        let Commands::Init { storage, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert_eq!(storage.db.to_string(), "sqlite:/tmp/from_flag.db");
    }

    #[test]
    fn db_env_beats_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_DB", "sqlite:/tmp/from_env.db");
        let cli = parse(&["init"]);
        std::env::remove_var("JAUNDER_DB");
        let Commands::Init { storage, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Init")
        };
        assert_eq!(storage.db.to_string(), "sqlite:/tmp/from_env.db");
    }

    // --- user-create ---

    #[test]
    fn user_create_parses_username_and_password() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&[
            "user-create",
            "--username",
            "alice",
            "--password",
            "secret123",
        ]);
        let Commands::UserCreate {
            username,
            password,
            display_name,
            ..
        } = cli.command.expect("subcommand")
        else {
            unreachable!("parse yields Commands::UserCreate")
        };
        assert_eq!(username, "alice");
        assert_eq!(password, Some("secret123".to_owned()));
        assert_eq!(display_name, None);
    }

    #[test]
    fn user_create_parses_display_name() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&[
            "user-create",
            "--username",
            "alice",
            "--password",
            "secret123",
            "--display-name",
            "Alice Smith",
        ]);
        let Commands::UserCreate { display_name, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::UserCreate")
        };
        assert_eq!(display_name, Some(parse_display_name("Alice Smith")));
    }

    #[test]
    fn user_create_password_optional() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&["user-create", "--username", "alice"]);
        let Commands::UserCreate { password, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::UserCreate")
        };
        assert_eq!(password, None);
    }

    #[test]
    fn user_create_missing_username_is_clap_error() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let result = Cli::try_parse_from(["jaunder", "user-create", "--password", "secret123"]);
        assert!(result.is_err());
    }

    #[test]
    fn user_create_malformed_username_is_clap_error() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        // The `Username` value parser rejects a bad username at parse time, before
        // any handler runs, rather than surfacing it as a later runtime error.
        let result = Cli::try_parse_from(["jaunder", "user-create", "--username", "bad name"]);
        assert!(result.is_err());
    }

    // --- user-invite ---

    #[test]
    fn user_invite_parses_expires_in() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&["user-invite", "--expires-in", "48"]);
        let Commands::UserInvite { expires_in, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::UserInvite")
        };
        assert_eq!(expires_in, Some(parse_invite_ttl_hours("48")));
    }

    #[test]
    fn user_invite_expires_in_optional() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&["user-invite"]);
        let Commands::UserInvite { expires_in, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::UserInvite")
        };
        assert_eq!(expires_in, None);
    }

    // --- smtp-test ---

    #[test]
    fn smtp_test_parses_to() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&["smtp-test", "--to", "alice@example.com"]);
        let Commands::SmtpTest { to, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::SmtpTest")
        };
        assert_eq!(to, "alice@example.com");
    }

    #[test]
    fn smtp_test_missing_to_is_clap_error() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let result = Cli::try_parse_from(["jaunder", "smtp-test"]);
        assert!(result.is_err());
    }

    // --- backup / restore ---

    #[test]
    fn backup_path_optional() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&["backup"]);
        let Commands::Backup { mode, path, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Backup")
        };
        assert_eq!(mode, CliBackupMode::Directory);
        assert_eq!(path, None);
    }

    #[test]
    fn backup_parses_path() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&["backup", "--path", "/tmp/backup"]);
        let Commands::Backup { path, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Backup")
        };
        assert_eq!(path, Some(PathBuf::from("/tmp/backup")));
    }

    #[test]
    fn backup_parses_archive_mode() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&[
            "backup",
            "--mode",
            "archive",
            "--path",
            "/tmp/backup.tar.gz",
        ]);
        let Commands::Backup { mode, path, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Backup")
        };
        assert_eq!(mode, CliBackupMode::Archive);
        assert_eq!(path, Some(PathBuf::from("/tmp/backup.tar.gz")));
    }

    #[test]
    fn restore_parses_required_path() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&["restore", "/tmp/backup"]);
        let Commands::Restore { path, .. } = cli.command.expect("subcommand") else {
            unreachable!("parse yields Commands::Restore")
        };
        assert_eq!(path, PathBuf::from("/tmp/backup"));
    }

    #[test]
    fn restore_missing_path_is_clap_error() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let result = Cli::try_parse_from(["jaunder", "restore"]);
        assert!(result.is_err());
    }

    // --- site-config ---

    #[test]
    fn site_config_set_parses_positional_key_value() {
        let cli = parse(&["site-config", "set", "feeds.websub_hub_url", "https://h/"]);
        let Commands::SiteConfig { action } = cli.command.expect("subcommand") else {
            unreachable!("parse yields site-config")
        };
        let SiteConfigAction::Set { key, value, .. } = action else {
            unreachable!("parse yields set")
        };
        assert_eq!(key, "feeds.websub_hub_url");
        assert_eq!(value, "https://h/");
    }

    #[test]
    fn site_config_set_allows_hyphen_leading_value() {
        let cli = parse(&["site-config", "set", "some.key", "-dashy"]);
        let Commands::SiteConfig { action } = cli.command.expect("subcommand") else {
            unreachable!("parse yields site-config")
        };
        let SiteConfigAction::Set { value, .. } = action else {
            unreachable!("parse yields set")
        };
        assert_eq!(value, "-dashy");
    }

    #[test]
    fn site_config_set_accepts_db_flag_after_positionals() {
        // The allow_hyphen_values value must not swallow the flattened --db flag.
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cli = parse(&[
            "site-config",
            "set",
            "some.key",
            "val",
            "--db",
            "sqlite:./x.db",
        ]);
        let Commands::SiteConfig { action } = cli.command.expect("subcommand") else {
            unreachable!("parse yields site-config")
        };
        let SiteConfigAction::Set { key, value, .. } = action else {
            unreachable!("parse yields set")
        };
        assert_eq!((key.as_str(), value.as_str()), ("some.key", "val"));
    }

    #[test]
    fn site_config_get_parses_key() {
        let cli = parse(&["site-config", "get", "site.title"]);
        let Commands::SiteConfig { action } = cli.command.expect("subcommand") else {
            unreachable!("parse yields site-config")
        };
        let SiteConfigAction::Get { key, .. } = action else {
            unreachable!("parse yields get")
        };
        assert_eq!(key, "site.title");
    }

    #[test]
    fn site_config_list_parses() {
        let cli = parse(&["site-config", "list"]);
        assert!(matches!(
            cli.command,
            Some(Commands::SiteConfig {
                action: SiteConfigAction::List { .. },
            })
        ));
    }

    #[test]
    fn site_config_unset_parses_key() {
        let cli = parse(&["site-config", "unset", "site.title"]);
        let Commands::SiteConfig { action } = cli.command.expect("subcommand") else {
            unreachable!("parse yields site-config")
        };
        let SiteConfigAction::Unset { key, .. } = action else {
            unreachable!("parse yields unset")
        };
        assert_eq!(key, "site.title");
    }

    #[test]
    fn site_config_set_missing_value_is_clap_error() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        assert!(Cli::try_parse_from(["jaunder", "site-config", "set", "only.key"]).is_err());
    }

    #[test]
    fn test_deployment_env() {
        assert!(!DeploymentEnv::Dev.is_prod());
        assert!(DeploymentEnv::Prod.is_prod());
        assert_eq!(DeploymentEnv::Dev.to_string(), "dev");
        assert_eq!(DeploymentEnv::Prod.to_string(), "prod");
    }

    // --- backup_mode conversion ---

    #[test]
    fn cli_backup_mode_converts_directory() {
        assert_eq!(
            BackupMode::from(CliBackupMode::Directory),
            BackupMode::Directory
        );
    }

    #[test]
    fn cli_backup_mode_converts_archive() {
        assert_eq!(
            BackupMode::from(CliBackupMode::Archive),
            BackupMode::Archive
        );
    }
}
