//! `test-support` — out-of-process test/e2e helpers that link jaunder's real
//! crates (see `lib.rs`). Never shipped in the `jaunder` production binary.

use clap::{Parser, Subcommand};
use common::display_name::DisplayName;
use host::capture;
use storage::DbConnectOptions;

use test_support::{create_user, reset_mail, seed_posts_for_user};

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
    /// Reset the mail-capture file (delete it; missing is fine). Derives
    /// `<JAUNDER_CAPTURE_DIR>/mail.jsonl`; errors if the capture dir is unset.
    ResetMail,
    /// Print the resolved capture-file path for a stream (`mail`/`websub`/`diag`),
    /// derived from `JAUNDER_CAPTURE_DIR`. Errors on an unset dir or unknown stream.
    CapturePath {
        /// The capture stream key.
        stream: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Cli::parse()).await
}

/// Dispatch the parsed subcommand to its handler. A flat match: each arm
/// evaluates to the handler's `Result<()>`, so `main` stays a thin shell and each
/// command is a small, individually-covered unit (#232).
async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::SeedPosts {
            db,
            username,
            count,
            body_prefix,
            published,
        } => cmd_seed_posts(&db, &username, count, &body_prefix, published).await,
        Commands::CreateUser {
            db,
            username,
            password,
            display_name,
            operator,
        } => cmd_create_user(&db, &username, &password, display_name.as_ref(), operator).await,
        Commands::ResetMail => cmd_reset_mail(),
        Commands::CapturePath { stream } => cmd_capture_path(&stream),
    }
}

/// Seed `count` posts for `username` through the real storage path.
async fn cmd_seed_posts(
    db: &DbConnectOptions,
    username: &str,
    count: usize,
    body_prefix: &str,
    published: bool,
) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db).await?;
    let ids = seed_posts_for_user(&state, username, count, published, body_prefix).await?;
    eprintln!("seeded {} posts for {username}", ids.len());
    Ok(())
}

/// Create a fixture user through the real storage path.
async fn cmd_create_user(
    db: &DbConnectOptions,
    username: &str,
    password: &str,
    display_name: Option<&DisplayName>,
    operator: bool,
) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db).await?;
    let id = create_user(&state, username, password, display_name, operator).await?;
    eprintln!("created user {username} with id {}", i64::from(id));
    Ok(())
}

/// Reset the mail-capture file (delete it; missing is fine).
fn cmd_reset_mail() -> anyhow::Result<()> {
    let path = capture::file(capture::Stream::Mail)
        .ok_or_else(|| anyhow::anyhow!("JAUNDER_CAPTURE_DIR is not set"))?;
    reset_mail(&path)?;
    eprintln!("reset mail-capture file {}", path.display());
    Ok(())
}

/// Print the resolved capture-file path for a stream (`mail`/`websub`/`diag`).
fn cmd_capture_path(stream: &str) -> anyhow::Result<()> {
    let stream = capture::Stream::parse(stream)
        .ok_or_else(|| anyhow::anyhow!("unknown capture stream {stream:?}"))?;
    let path =
        capture::file(stream).ok_or_else(|| anyhow::anyhow!("JAUNDER_CAPTURE_DIR is not set"))?;
    println!("{}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        storage::open_database(&db).await.unwrap();
        (dir, db)
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

        // Read back through a fresh connection to prove the dispatch wired each
        // command's arguments through to storage (not merely returned Ok): the
        // seeded post is published and attributed to alice.
        let state = storage::open_existing_database(&db).await.unwrap();
        let published = state
            .posts
            .list_published_by_user(
                &"alice".parse().unwrap(),
                None,
                10,
                &common::visibility::ViewerIdentity::Anonymous,
                chrono::Utc::now(),
            )
            .await
            .expect("list ok");
        assert_eq!(
            published.len(),
            1,
            "seed-posts should publish 1 post for alice"
        );
    }
}
