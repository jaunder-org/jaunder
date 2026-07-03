//! `test-support` — out-of-process test/e2e helpers that link jaunder's real
//! crates (see `lib.rs`). Never shipped in the `jaunder` production binary.

use clap::{Parser, Subcommand};
use storage::DbConnectOptions;

use test_support::{create_user, seed_posts_for_user};

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
        display_name: Option<String>,
        /// Grant operator (admin) privileges.
        #[arg(long)]
        operator: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::SeedPosts {
            db,
            username,
            count,
            body_prefix,
            published,
        } => {
            let state = storage::open_existing_database(&db).await?;
            let ids =
                seed_posts_for_user(&state, &username, count, published, &body_prefix).await?;
            eprintln!("seeded {} posts for {username}", ids.len());
        }
        Commands::CreateUser {
            db,
            username,
            password,
            display_name,
            operator,
        } => {
            let state = storage::open_existing_database(&db).await?;
            let id = create_user(
                &state,
                &username,
                &password,
                display_name.as_deref(),
                operator,
            )
            .await?;
            eprintln!("created user {username} with id {id}");
        }
    }
    Ok(())
}
