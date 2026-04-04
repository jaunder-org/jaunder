# M2 Step 8: `jaunder user create` and `jaunder user invite` Commands

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `user-create` and `user-invite` CLI subcommands to bootstrap users and invite codes on a running instance.

**Architecture:** Two new `Commands` variants (`UserCreate`, `UserInvite`) are added to `cli.rs`; their handlers live in `commands.rs` and are dispatched from `main.rs`. `cmd_user_create` bypasses the registration policy. `cmd_user_invite` generates an invite code with a configurable expiry. The `rpassword` crate handles interactive password prompting when `--password` is omitted.

**Tech Stack:** Rust, clap 4, rpassword 7, chrono 0.4, sqlx (sqlite), argon2, existing `Username`/`Password` newtypes.

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `Cargo.toml` | Add `rpassword = "7"` workspace dep |
| Modify | `server/Cargo.toml` | Wire `rpassword` into server binary deps |
| Modify | `server/src/cli.rs` | Add `UserCreate` and `UserInvite` variants + unit tests |
| Modify | `server/src/commands.rs` | Implement `cmd_user_create` and `cmd_user_invite` |
| Modify | `server/src/main.rs` | Dispatch `UserCreate` and `UserInvite` |
| Modify | `server/tests/commands.rs` | Integration tests for both commands |

---

### Task 1: Add `rpassword` dependency and CLI variants with unit tests

**Files:**
- Modify: `Cargo.toml`
- Modify: `server/Cargo.toml`
- Modify: `server/src/cli.rs`

- [ ] **Step 1: Write the failing CLI unit tests**

Add these tests to the `#[cfg(test)] mod tests` block in `server/src/cli.rs`, just before the closing `}`:

```rust
// --- user-create ---

#[test]
fn user_create_parses_username_and_password() {
    let cli = parse(&["user-create", "--username", "alice", "--password", "secret123"]);
    let Commands::UserCreate {
        username,
        password,
        display_name,
        ..
    } = cli.command
    else {
        panic!("wrong variant");
    };
    assert_eq!(username, "alice");
    assert_eq!(password, Some("secret123".to_owned()));
    assert_eq!(display_name, None);
}

#[test]
fn user_create_parses_display_name() {
    let cli = parse(&[
        "user-create",
        "--username",
        "alice",
        "--password",
        "secret123",
        "--display-name",
        "Alice Smith",
    ]);
    let Commands::UserCreate { display_name, .. } = cli.command else {
        panic!("wrong variant");
    };
    assert_eq!(display_name, Some("Alice Smith".to_owned()));
}

#[test]
fn user_create_password_optional() {
    let cli = parse(&["user-create", "--username", "alice"]);
    let Commands::UserCreate { password, .. } = cli.command else {
        panic!("wrong variant");
    };
    assert_eq!(password, None);
}

#[test]
fn user_create_missing_username_is_clap_error() {
    let result =
        Cli::try_parse_from(["jaunder", "user-create", "--password", "secret123"]);
    assert!(result.is_err());
}

// --- user-invite ---

#[test]
fn user_invite_parses_expires_in() {
    let cli = parse(&["user-invite", "--expires-in", "48"]);
    let Commands::UserInvite { expires_in, .. } = cli.command else {
        panic!("wrong variant");
    };
    assert_eq!(expires_in, Some(48));
}

#[test]
fn user_invite_expires_in_optional() {
    let cli = parse(&["user-invite"]);
    let Commands::UserInvite { expires_in, .. } = cli.command else {
        panic!("wrong variant");
    };
    assert_eq!(expires_in, None);
}
```

- [ ] **Step 2: Run the tests to verify they fail (compilation error)**

```bash
cargo nextest run -E 'test(user_create_parses_username_and_password)'
```

Expected: compilation error — `Commands::UserCreate` does not exist.

- [ ] **Step 3: Add `rpassword` to workspace dependencies**

In `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
rpassword = "7"
```

- [ ] **Step 4: Add `rpassword` to server dependencies**

In `server/Cargo.toml`, add to `[dependencies]`:

```toml
rpassword.workspace = true
```

- [ ] **Step 5: Add `UserCreate` and `UserInvite` variants to `Commands` in `server/src/cli.rs`**

Replace the closing `}` of the `Commands` enum (after the `Serve` variant) with:

```rust
    /// Create a user account directly, bypassing the registration policy.
    ///
    /// Intended for bootstrapping an initial operator account. The storage
    /// directory must already be initialized via `jaunder init`.
    UserCreate {
        #[command(flatten)]
        storage: StorageArgs,

        /// Username for the new account (must match [a-z0-9_-]+).
        #[arg(long)]
        username: String,

        /// Password for the new account. If omitted, you will be prompted
        /// interactively (input is hidden).
        #[arg(long)]
        password: Option<String>,

        /// Optional display name.
        #[arg(long)]
        display_name: Option<String>,
    },

    /// Generate an invite code.
    ///
    /// The storage directory must already be initialized via `jaunder init`.
    UserInvite {
        #[command(flatten)]
        storage: StorageArgs,

        /// Hours until the invite code expires. Defaults to 168 (7 days).
        #[arg(long)]
        expires_in: Option<u64>,
    },
}
```

- [ ] **Step 6: Run the unit tests to verify they pass**

```bash
cargo nextest run -E 'test(user_create) + test(user_invite)'
```

Expected: all 6 new tests PASS.

- [ ] **Step 7: Run full build and lint**

```bash
cargo build && cargo nextest run && cargo clippy -- -D warnings
```

Expected: all pass with no warnings.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml server/Cargo.toml server/src/cli.rs
git commit -m "M2.8.1-M2.8.2: Add UserCreate and UserInvite CLI variants; add rpassword dep"
```

---

### Task 2: Implement `cmd_user_create` with integration test

**Files:**
- Modify: `server/tests/commands.rs`
- Modify: `server/src/commands.rs`
- Modify: `server/src/main.rs`

- [ ] **Step 1: Write the failing integration test**

Add to `server/tests/commands.rs` (after the existing imports, add the new imports, and append the test at the bottom of the file):

New imports to add at the top:
```rust
use server::commands::{cmd_user_create, cmd_user_invite};
use server::username::Username;
```

New test:
```rust
#[tokio::test]
async fn cmd_user_create_creates_retrievable_user() {
    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);
    cmd_init(&args, false).await.expect("init");

    cmd_user_create(&args, "alice", Some("password123"), None)
        .await
        .expect("user create");

    let state = open_existing_database(&args.db).await.expect("open db");
    let username: Username = "alice".parse().expect("valid username");
    let user = state
        .users
        .get_user_by_username(&username)
        .await
        .expect("db query");
    assert!(user.is_some(), "user should exist after creation");
    assert_eq!(
        user.expect("user present").username.as_str(),
        "alice"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -E 'test(cmd_user_create_creates_retrievable_user)'
```

Expected: compilation error — `cmd_user_create` does not exist.

- [ ] **Step 3: Implement `cmd_user_create` in `server/src/commands.rs`**

Add to the top of `commands.rs`:
```rust
use crate::password::Password;
use crate::username::Username;
```

Append this function to `commands.rs`:
```rust
pub async fn cmd_user_create(
    storage: &StorageArgs,
    username: &str,
    password: Option<&str>,
    display_name: Option<&str>,
) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let username = username
        .parse::<Username>()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let password_str = match password {
        Some(p) => p.to_owned(),
        None => {
            let p1 = rpassword::prompt_password("Password: ")?;
            let p2 = rpassword::prompt_password("Confirm password: ")?;
            if p1 != p2 {
                return Err(anyhow::anyhow!("passwords do not match"));
            }
            p1
        }
    };

    let password = password_str
        .parse::<Password>()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let user_id = state
        .users
        .create_user(&username, &password, display_name)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Created user '{}' with id {user_id}", username);
    Ok(())
}
```

Also add to the imports at the top of `commands.rs`:
```rust
use crate::storage::open_existing_database;
```
(Check whether `open_existing_database` is already imported; it is — adjust if needed.)

- [ ] **Step 4: Add `rpassword` import to `commands.rs`**

At the top of `commands.rs`, the `rpassword` crate is used directly — it does not need an explicit `use` since `rpassword::prompt_password` is called with the full path. Confirm the code compiles.

- [ ] **Step 5: Add dispatch in `main.rs`**

In `main.rs`, add a match arm after `Commands::Serve`:

```rust
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
Commands::UserInvite { storage, expires_in } => {
    server::commands::cmd_user_invite(&storage, expires_in).await?;
}
```

(Add `cmd_user_invite` dispatch now even though the function doesn't exist yet — it will be added in Task 3. Alternatively, add only `UserCreate` dispatch here and add `UserInvite` in Task 3. Since `main.rs` must be exhaustive for the match, add a placeholder `unimplemented!()` for `UserInvite` here and replace it in Task 3.)

Actually, since adding `UserInvite` dispatch requires the function to exist to compile, add a `todo!()` stub in commands.rs for `cmd_user_invite` now:

```rust
pub async fn cmd_user_invite(
    _storage: &StorageArgs,
    _expires_in: Option<u64>,
) -> anyhow::Result<()> {
    todo!("cmd_user_invite not yet implemented")
}
```

And add dispatch for both in `main.rs`:

```rust
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
Commands::UserInvite { storage, expires_in } => {
    server::commands::cmd_user_invite(&storage, expires_in).await?;
}
```

- [ ] **Step 6: Run the integration test to verify it passes**

```bash
cargo nextest run -E 'test(cmd_user_create_creates_retrievable_user)'
```

Expected: PASS.

- [ ] **Step 7: Run full build and lint**

```bash
cargo build && cargo nextest run && cargo clippy -- -D warnings
```

Expected: all pass. (The `todo!()` stub will not be triggered by any test.)

- [ ] **Step 8: Commit**

```bash
git add server/src/commands.rs server/src/main.rs server/tests/commands.rs
git commit -m "M2.8.3-M2.8.4: Implement cmd_user_create; dispatch UserCreate and UserInvite in main"
```

---

### Task 3: Implement `cmd_user_invite` with integration test

**Files:**
- Modify: `server/tests/commands.rs`
- Modify: `server/src/commands.rs`

- [ ] **Step 1: Write the failing integration test**

Append to `server/tests/commands.rs`:

```rust
#[tokio::test]
async fn cmd_user_invite_creates_retrievable_invite() {
    let base = TempDir::new().expect("temp dir");
    let args = storage_args(&base);
    cmd_init(&args, false).await.expect("init");

    cmd_user_invite(&args, Some(48)).await.expect("user invite");

    let state = open_existing_database(&args.db).await.expect("open db");
    let invites = state
        .invites
        .list_invites()
        .await
        .expect("list invites");
    assert_eq!(invites.len(), 1, "exactly one invite should exist");
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -E 'test(cmd_user_invite_creates_retrievable_invite)'
```

Expected: FAIL — panics with `todo!("cmd_user_invite not yet implemented")`.

- [ ] **Step 3: Implement `cmd_user_invite` in `server/src/commands.rs`**

Replace the `todo!()` stub for `cmd_user_invite` with the real implementation:

```rust
pub async fn cmd_user_invite(
    storage: &StorageArgs,
    expires_in: Option<u64>,
) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let hours = expires_in.unwrap_or(168) as i64;
    let expires_at = chrono::Utc::now() + chrono::TimeDelta::hours(hours);

    let code = state.invites.create_invite(expires_at).await?;
    println!("{code}");
    Ok(())
}
```

Also add to the imports at the top of `commands.rs` if not already present:

```rust
use chrono;
```

(Since `chrono` is already used transitively but not directly imported, confirm the crate is in scope. The full path `chrono::Utc::now()` and `chrono::TimeDelta::hours()` will work without a `use` statement as long as `chrono` is in `[dependencies]`.)

Note: `chrono::Duration::hours` was renamed to `chrono::TimeDelta::hours` in chrono 0.4.35. Both `chrono::Duration::hours` and `chrono::TimeDelta::hours` work in chrono 0.4.x — use `chrono::Duration::hours` for compatibility if `TimeDelta` isn't available, or check which is available:

```rust
// Safe across chrono 0.4.x versions:
let expires_at = chrono::Utc::now() + chrono::Duration::hours(hours);
```

- [ ] **Step 4: Run the integration test to verify it passes**

```bash
cargo nextest run -E 'test(cmd_user_invite_creates_retrievable_invite)'
```

Expected: PASS.

- [ ] **Step 5: Run full build, tests, and lint**

```bash
cargo build && cargo nextest run && cargo clippy -- -D warnings
```

Expected: all pass with no warnings.

- [ ] **Step 6: Commit**

```bash
git add server/src/commands.rs server/tests/commands.rs
git commit -m "M2.8.5-M2.8.8: Implement cmd_user_invite; add integration tests for both user commands"
```

---

## Self-Review Checklist

**Spec coverage (M2.8.x):**
- [x] 8.1: `UserCreate` variant with `rpassword` prompt when `--password` omitted → Task 1 + Task 2
- [x] 8.2: `UserInvite` variant with optional `--expires-in` → Task 1
- [x] 8.3: Dispatch both in `main.rs` → Task 2 Step 5
- [x] 8.4: `cmd_user_create` opens DB, parses username, creates user, prints confirmation → Task 2 Step 3
- [x] 8.5: `cmd_user_invite` opens DB, computes expiry with 168h default, creates invite, prints code → Task 3 Step 3
- [x] 8.6: CLI unit tests for both commands → Task 1 Step 1
- [x] 8.7: Integration test: `cmd_user_create` → user retrievable via `get_user_by_username` → Task 2 Step 1
- [x] 8.8: Integration test: `cmd_user_invite` → invite retrievable via `list_invites` → Task 3 Step 1

**Placeholder scan:** No TBD/TODO/similar present in plan tasks.

**Type consistency:**
- `cmd_user_create(&StorageArgs, &str, Option<&str>, Option<&str>)` is used consistently in Task 2 and referenced correctly in `main.rs` dispatch (`.as_deref()` converts `Option<String>` to `Option<&str>`).
- `cmd_user_invite(&StorageArgs, Option<u64>)` consistent throughout.
- `Username::from_str` / `Password::from_str` used correctly.
- `state.users.create_user(&username, &password, display_name)` matches the `UserStorage` trait signature.
- `state.invites.create_invite(expires_at)` matches `InviteStorage` trait signature (`expires_at: DateTime<Utc>`).
