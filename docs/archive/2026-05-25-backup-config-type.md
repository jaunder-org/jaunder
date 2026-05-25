# BackupConfig Common Type Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce a single typed `BackupConfig` in `common` that replaces the string-bag `BackupSettings` DTO, the ad-hoc `BackupWorkerConfig` struct, and all duplicated validation/default logic spread across `web` and `server`.

**Architecture:** `common/src/backup.rs` defines `BackupConfig` (with `BackupSchedule` and `BackupMode` as infallible types) and derives `Serialize`/`Deserialize` so it crosses the WASM boundary directly. `storage` gains `get_backup_config`/`set_backup_config` default methods on `SiteConfigStorage`. The web server functions become thin boundary parsers (strings in → `BackupConfig` or error). `BackupWorkerConfig` and `BackupSettings` are deleted. `BackupMode` moves from `storage` to `common` with no `clap` knowledge; the CLI defines its own `clap::ValueEnum` wrapper and maps it to `common::backup::BackupMode` at the call site.

**Tech Stack:** Rust, `croner` (schedule validation), `serde`, `clap` (CLI only), `leptos` server functions, `sqlx`.

---

## Design decisions

### `BackupMode` and `clap`

`BackupMode` currently lives in `storage` and derives `clap::ValueEnum` so the CLI can use it directly as an `#[arg]` type. Moving it to `common` means keeping `common` free of any CLI dependency. The CLI defines its own `CliBackupMode` enum in `server/src/cli.rs` with `clap::ValueEnum`, and converts to `common::backup::BackupMode` via `From`. This is a trivial two-arm match and keeps `common` a pure domain library.

### `BackupSchedule` storage

Stored as a plain string in the DB (the `backup.schedule` site_config key). `get_backup_config` parses it; `set_backup_config` calls `.as_str()`. The newtype wraps a validated string and can only be constructed via `BackupSchedule::parse` (returns `Option<Self>`) or `BackupSchedule::default()`.

### `destination_path` type

`Option<String>` in `BackupConfig` — `None` means unconfigured. It's a user-supplied filesystem path with no further validation at this layer (the file system is the authority). The server converts to `PathBuf` at the point of use.

---

## File Map

| File | Change |
|---|---|
| `common/src/backup.rs` | **Create** — `BackupConfig`, `BackupSchedule`, `BackupMode` (no clap) |
| `common/src/lib.rs` | Add `pub mod backup;` |
| `common/Cargo.toml` | Add `croner` dependency |
| `storage/src/backup.rs` | Remove `BackupMode` definition; re-export from `common::backup` |
| `storage/src/site_config.rs` | Add `get_backup_config`/`set_backup_config` as default methods on `SiteConfigStorage` |
| `storage/src/lib.rs` | `BackupMode` re-export unchanged (still flows through `backup::BackupMode`) |
| `server/src/cli.rs` | Add `CliBackupMode` with `clap::ValueEnum`; `From<CliBackupMode> for BackupMode`; update `#[arg]` to use it |
| `server/src/lib.rs` | Delete `BackupWorkerConfig` + local helpers; use `BackupConfig` from `common` |
| `server/src/commands.rs` | Import `BackupMode` from `common::backup` instead of `storage` |
| `web/src/backup/mod.rs` | Delete `BackupSettings` + local helpers; server fns use `BackupConfig` |

---

### Task 1: Create `common/src/backup.rs` with `BackupMode`, `BackupSchedule`, `BackupConfig`

**Files:**
- Create: `common/src/backup.rs`
- Modify: `common/src/lib.rs`
- Modify: `common/Cargo.toml`

- [ ] **Step 1: Add `croner` to `common/Cargo.toml`**

  In `common/Cargo.toml`, add to `[dependencies]`:
  ```toml
  croner.workspace = true
  ```

  No `clap` dependency — `common` stays CLI-free.

- [ ] **Step 2: Create `common/src/backup.rs`**

  ```rust
  use croner::Cron;
  use serde::{Deserialize, Serialize};

  #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
  #[serde(rename_all = "snake_case")]
  pub enum BackupMode {
      Directory,
      Archive,
  }

  impl Default for BackupMode {
      fn default() -> Self {
          Self::Directory
      }
  }

  /// A validated six-field cron schedule expression.
  ///
  /// Can only be constructed via [`BackupSchedule::parse`] or
  /// [`BackupSchedule::default`], guaranteeing the inner string is always valid.
  #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
  pub struct BackupSchedule(String);

  impl BackupSchedule {
      pub fn parse(s: &str) -> Option<Self> {
          Cron::new(s.trim())
              .with_seconds_required()
              .parse()
              .ok()
              .map(|_| Self(s.trim().to_owned()))
      }

      pub fn as_str(&self) -> &str {
          &self.0
      }
  }

  impl Default for BackupSchedule {
      fn default() -> Self {
          Self("0 0 0 * * *".to_owned())
      }
  }

  pub const DEFAULT_BACKUP_RETENTION_COUNT: usize = 7;

  #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
  pub struct BackupConfig {
      /// `None` means no destination is configured and scheduled backups are disabled.
      pub destination_path: Option<String>,
      pub schedule: BackupSchedule,
      pub retention_count: usize,
      pub mode: BackupMode,
  }

  impl Default for BackupConfig {
      fn default() -> Self {
          Self {
              destination_path: None,
              schedule: BackupSchedule::default(),
              retention_count: DEFAULT_BACKUP_RETENTION_COUNT,
              mode: BackupMode::default(),
          }
      }
  }
  ```

- [ ] **Step 3: Register the module in `common/src/lib.rs`**

  Add:
  ```rust
  pub mod backup;
  ```

- [ ] **Step 4: Verify `common` builds**

  ```bash
  cargo build -p common 2>&1 | head -30
  ```

  Expected: no errors.

- [ ] **Step 5: Commit**

  ```bash
  git add common/src/backup.rs common/src/lib.rs common/Cargo.toml
  git commit -m "feat(common): add backup module with BackupMode, BackupSchedule, BackupConfig"
  ```

---

### Task 2: Move `BackupMode` out of `storage`; add CLI wrapper in `server/src/cli.rs`

`storage/src/backup.rs` currently defines `BackupMode` with `clap::ValueEnum`. We replace it with the `common` version (no clap), re-export it from `storage` for existing call sites, and give the CLI its own thin wrapper type.

**Files:**
- Modify: `storage/src/backup.rs`
- Modify: `server/src/cli.rs`
- Modify: `server/src/commands.rs`

- [ ] **Step 1: Remove `BackupMode` definition from `storage/src/backup.rs`**

  Delete this block:
  ```rust
  #[derive(Debug, Clone, Copy, Eq, PartialEq, clap::ValueEnum, Serialize, Deserialize)]
  #[serde(rename_all = "snake_case")]
  pub enum BackupMode {
      Directory,
      Archive,
  }
  ```

  Add at the top of `storage/src/backup.rs`:
  ```rust
  pub use common::backup::BackupMode;
  ```

  `storage/src/lib.rs` re-exports `BackupMode` via `pub use backup::{..., BackupMode, ...}` — that line is unchanged and still resolves through the new re-export chain.

- [ ] **Step 2: Add `CliBackupMode` to `server/src/cli.rs`**

  Add near the top of `server/src/cli.rs`, alongside other clap types:
  ```rust
  use common::backup::BackupMode;

  #[derive(Clone, Copy, Debug, clap::ValueEnum)]
  enum CliBackupMode {
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
  ```

  Find the `Backup` subcommand `#[arg]` that currently uses `BackupMode`:
  ```rust
  #[arg(long, value_enum, default_value = "directory")]
  mode: BackupMode,
  ```
  Change to:
  ```rust
  #[arg(long, value_enum, default_value = "directory")]
  mode: CliBackupMode,
  ```

  At the call site where `mode` is passed to `cmd_backup`, convert it:
  ```rust
  cmd_backup(&storage, mode.into(), path).await
  ```

- [ ] **Step 3: Update `server/src/commands.rs`**

  Replace `use storage::BackupMode` with `use common::backup::BackupMode` — or simply remove it if `BackupMode` is already in scope via another import. Verify `cmd_backup` signature still accepts `BackupMode`.

- [ ] **Step 4: Verify the full workspace builds**

  ```bash
  cargo build 2>&1 | head -40
  ```

  Expected: no errors.

- [ ] **Step 5: Run tests**

  ```bash
  cargo nextest run -p storage -p server 2>&1 | tail -20
  ```

  Expected: all pass.

- [ ] **Step 6: Commit**

  ```bash
  git add storage/src/backup.rs server/src/cli.rs server/src/commands.rs
  git commit -m "refactor: move BackupMode to common::backup; CLI uses local CliBackupMode wrapper"
  ```

---

### Task 3: Add `get_backup_config` / `set_backup_config` to `SiteConfigStorage`

These are default methods (like the existing `get_int`) so all backends get them for free without touching SQLite/Postgres implementations.

**Files:**
- Modify: `storage/src/site_config.rs`

- [ ] **Step 1: Add imports to `storage/src/site_config.rs`**

  Add at the top:
  ```rust
  use common::backup::{BackupConfig, BackupMode, BackupSchedule, DEFAULT_BACKUP_RETENTION_COUNT};
  ```

- [ ] **Step 2: Add default methods to `SiteConfigStorage`**

  Inside the `trait SiteConfigStorage` block, after `get_int`, add:

  ```rust
  async fn get_backup_config(&self) -> sqlx::Result<BackupConfig> {
      let destination_path = self
          .get(BACKUP_DESTINATION_PATH_KEY)
          .await?
          .and_then(|v| {
              let v = v.trim().to_owned();
              if v.is_empty() { None } else { Some(v) }
          });
      let schedule = self
          .get(BACKUP_SCHEDULE_KEY)
          .await?
          .as_deref()
          .and_then(BackupSchedule::parse)
          .unwrap_or_default();
      let retention_count = self
          .get(BACKUP_RETENTION_COUNT_KEY)
          .await?
          .as_deref()
          .and_then(|v| v.trim().parse::<usize>().ok())
          .unwrap_or(DEFAULT_BACKUP_RETENTION_COUNT);
      let mode = self
          .get(BACKUP_MODE_KEY)
          .await?
          .as_deref()
          .and_then(parse_backup_mode)
          .unwrap_or_default();
      Ok(BackupConfig { destination_path, schedule, retention_count, mode })
  }

  async fn set_backup_config(&self, config: &BackupConfig) -> sqlx::Result<()> {
      self.set(
          BACKUP_DESTINATION_PATH_KEY,
          config.destination_path.as_deref().unwrap_or(""),
      )
      .await?;
      self.set(BACKUP_SCHEDULE_KEY, config.schedule.as_str()).await?;
      self.set(
          BACKUP_RETENTION_COUNT_KEY,
          &config.retention_count.to_string(),
      )
      .await?;
      self.set(BACKUP_MODE_KEY, backup_mode_str(config.mode)).await?;
      Ok(())
  }
  ```

- [ ] **Step 3: Add the two private helper fns below the trait**

  ```rust
  fn parse_backup_mode(value: &str) -> Option<BackupMode> {
      match value.trim() {
          "directory" => Some(BackupMode::Directory),
          "archive" => Some(BackupMode::Archive),
          _ => None,
      }
  }

  fn backup_mode_str(mode: BackupMode) -> &'static str {
      match mode {
          BackupMode::Directory => "directory",
          BackupMode::Archive => "archive",
      }
  }
  ```

- [ ] **Step 4: Verify `storage` builds**

  ```bash
  cargo build -p storage 2>&1 | head -30
  ```

  Expected: no errors.

- [ ] **Step 5: Add integration tests in `storage/src/site_config.rs`**

  Add a `#[cfg(test)]` block at the bottom of `storage/src/site_config.rs`:

  ```rust
  #[cfg(test)]
  mod tests {
      use sqlx::SqlitePool;
      use crate::sqlite::SqliteSiteConfigStorage;
      use common::backup::{BackupConfig, BackupMode, BackupSchedule};
      use super::SiteConfigStorage;

      async fn test_pool() -> SqlitePool {
          let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
          sqlx::migrate!("./migrations/sqlite").run(&pool).await.unwrap();
          pool
      }

      #[tokio::test]
      async fn get_backup_config_returns_defaults_when_unconfigured() {
          let pool = test_pool().await;
          let storage = SqliteSiteConfigStorage::new(pool);
          let config = storage.get_backup_config().await.unwrap();
          assert_eq!(config, BackupConfig::default());
      }

      #[tokio::test]
      async fn set_and_get_backup_config_round_trips() {
          let pool = test_pool().await;
          let storage = SqliteSiteConfigStorage::new(pool);
          let config = BackupConfig {
              destination_path: Some("/srv/backups".to_owned()),
              schedule: BackupSchedule::parse("0 30 2 * * *").unwrap(),
              retention_count: 14,
              mode: BackupMode::Archive,
          };
          storage.set_backup_config(&config).await.unwrap();
          assert_eq!(storage.get_backup_config().await.unwrap(), config);
      }

      #[tokio::test]
      async fn get_backup_config_ignores_invalid_stored_values() {
          let pool = test_pool().await;
          let storage = SqliteSiteConfigStorage::new(pool);
          storage.set("backup.schedule", "not a cron").await.unwrap();
          storage.set("backup.retention_count", "daily").await.unwrap();
          storage.set("backup.mode", "floppy").await.unwrap();
          let config = storage.get_backup_config().await.unwrap();
          assert_eq!(config, BackupConfig::default());
      }
  }
  ```

- [ ] **Step 6: Run the tests**

  ```bash
  cargo nextest run -p storage 2>&1 | tail -20
  ```

  Expected: all pass including the three new tests.

- [ ] **Step 7: Commit**

  ```bash
  git add storage/src/site_config.rs
  git commit -m "feat(storage): add get_backup_config/set_backup_config to SiteConfigStorage"
  ```

---

### Task 4: Refactor `server/src/lib.rs` — delete `BackupWorkerConfig` and local helpers

**Files:**
- Modify: `server/src/lib.rs`

- [ ] **Step 1: Update imports in `server/src/lib.rs`**

  Replace:
  ```rust
  use storage::{export_backup, BackupExportOptions, BackupMode, DbConnectOptions};
  ```
  With:
  ```rust
  use common::backup::{BackupConfig, BackupMode};
  use storage::{export_backup, BackupExportOptions, DbConnectOptions, SiteConfigStorage};
  ```

- [ ] **Step 2: Delete `BackupWorkerConfig` and its `impl` block**

  Remove the entire `struct BackupWorkerConfig { ... }` and `impl BackupWorkerConfig { ... }` definitions.

- [ ] **Step 3: Rewrite `start_backup_worker`**

  Replace the existing function body with:

  ```rust
  pub async fn start_backup_worker(
      state: Arc<AppState>,
      database: DbConnectOptions,
      storage_path: PathBuf,
  ) -> anyhow::Result<Option<JobScheduler>> {
      let config = state.site_config.get_backup_config().await?;
      let Some(destination_root) = config.destination_path.as_deref().map(PathBuf::from) else {
          tracing::warn!("backup worker disabled: backup.destination_path is not configured");
          return Ok(None);
      };

      let scheduler = JobScheduler::new().await?;
      let schedule = config.schedule.as_str().to_owned();
      let job = Job::new_async(schedule.as_str(), move |_uuid, _lock| {
          let database = database.clone();
          let media_path = storage_path.join("media");
          let destination_root = destination_root.clone();
          let config = config.clone();
          Box::pin(async move {
              if let Err(error) =
                  run_scheduled_backup(&database, &media_path, &destination_root, &config).await
              {
                  tracing::error!(error = %error, "scheduled backup failed");
              }
          })
      })?;
      scheduler.add(job).await?;
      scheduler.start().await?;
      Ok(Some(scheduler))
  }
  ```

- [ ] **Step 4: Rewrite `run_scheduled_backup` to take `&BackupConfig`**

  ```rust
  async fn run_scheduled_backup(
      database: &DbConnectOptions,
      media_path: &Path,
      destination_root: &Path,
      config: &BackupConfig,
  ) -> anyhow::Result<PathBuf> {
      fs::create_dir_all(destination_root)?;
      let destination_path = backup_path_for_mode(destination_root, config.mode);
      export_backup(BackupExportOptions {
          database,
          media_path,
          destination_path: &destination_path,
          mode: config.mode,
      })
      .await?;
      prune_backups(destination_root, config.retention_count)?;
      tracing::info!(path = %destination_path.display(), "scheduled backup complete");
      Ok(destination_path)
  }
  ```

- [ ] **Step 5: Delete the now-unused local helper functions**

  Remove:
  - `fn default_backup_schedule() -> String`
  - `fn default_backup_retention_count() -> usize`
  - `fn default_backup_mode() -> BackupMode`
  - `fn backup_schedule_valid(schedule: &str) -> bool`
  - `fn parse_backup_mode(value: &str) -> Option<BackupMode>`
  - `fn non_empty_path(value: &str) -> Option<PathBuf>` (if only used by `BackupWorkerConfig::load`)

- [ ] **Step 6: Build and fix any remaining compile errors**

  ```bash
  cargo build -p server 2>&1 | head -40
  ```

  Fix any reference errors. The `croner` import in `server/src/lib.rs` (used by the deleted `backup_schedule_valid`) can be removed from the imports if no longer needed.

- [ ] **Step 7: Run server tests**

  ```bash
  cargo nextest run -p server 2>&1 | tail -20
  ```

  Expected: all pass. The tests in `server/src/lib.rs` that tested `BackupWorkerConfig::load` and `parse_backup_mode` will need to be deleted (they tested now-deleted code) or rewritten against `get_backup_config` (already covered by storage tests in Task 3).

- [ ] **Step 8: Commit**

  ```bash
  git add server/src/lib.rs
  git commit -m "refactor(server): replace BackupWorkerConfig with common::backup::BackupConfig"
  ```

---

### Task 5: Refactor `web/src/backup/mod.rs` — delete `BackupSettings` and local helpers

**Files:**
- Modify: `web/src/backup/mod.rs`

- [ ] **Step 1: Update imports**

  Replace the existing `use` block and `#[cfg(feature = "ssr")]` imports with:

  ```rust
  use crate::error::WebResult;
  use common::backup::{BackupConfig, BackupMode, BackupSchedule};
  use leptos::prelude::*;
  use serde::{Deserialize, Serialize};

  #[cfg(feature = "ssr")]
  mod server;
  #[cfg(feature = "ssr")]
  use server::require_operator;

  #[cfg(feature = "ssr")]
  use {
      crate::auth::require_auth,
      crate::error::{InternalError, WebError},
      std::sync::Arc,
      storage::{SiteConfigStorage, UserStorage},
  };
  ```

- [ ] **Step 2: Delete `BackupSettings` and all local helper functions**

  Remove:
  - `pub struct BackupSettings { ... }`
  - `fn backup_destination_configured`
  - `fn default_backup_schedule`
  - `fn default_backup_retention_count`
  - `fn default_backup_mode`
  - `fn backup_retention_count_valid`
  - `fn backup_schedule_valid`
  - `fn backup_mode_valid`
  - `fn backup_schedule_value`
  - `fn backup_retention_count_value`
  - `fn backup_mode_value`
  - `fn optional_backup_schedule_valid`
  - `fn optional_backup_retention_count_valid`
  - `fn optional_backup_mode_valid`
  - `fn backup_configuration_complete_and_valid`

- [ ] **Step 3: Rewrite `backup_warning_visible`**

  ```rust
  #[server(endpoint = "/backup_warning_visible")]
  #[cfg_attr(
      feature = "ssr",
      tracing::instrument(name = "web.backup.warning_visible")
  )]
  pub async fn backup_warning_visible() -> WebResult<bool> {
      boundary!("backup_warning_visible", {
          let auth = match require_auth().await {
              Ok(auth) => auth,
              Err(error) if matches!(error.public(), WebError::Unauthorized) => return Ok(false),
              Err(error) => return Err(error),
          };

          let users = expect_context::<Arc<dyn UserStorage>>();
          let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
          let Some(user) = users
              .get_user(auth.user_id)
              .await
              .map_err(InternalError::storage)?
          else {
              return Ok(false);
          };

          if !user.is_operator {
              return Ok(false);
          }

          let config = site_config
              .get_backup_config()
              .await
              .map_err(InternalError::storage)?;

          Ok(config.destination_path.is_none())
      })
  }
  ```

- [ ] **Step 4: Rewrite `get_backup_settings` to return `BackupConfig`**

  ```rust
  #[server(endpoint = "/get_backup_settings")]
  #[cfg_attr(feature = "ssr", tracing::instrument(name = "web.backup.get_settings"))]
  pub async fn get_backup_settings() -> WebResult<BackupConfig> {
      boundary!("get_backup_settings", {
          require_operator().await?;
          let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
          site_config
              .get_backup_config()
              .await
              .map_err(InternalError::storage)
      })
  }
  ```

- [ ] **Step 5: Rewrite `update_backup_settings` to parse at the boundary**

  ```rust
  #[server(endpoint = "/update_backup_settings")]
  #[cfg_attr(
      feature = "ssr",
      tracing::instrument(
          name = "web.backup.update_settings",
          skip(destination_path, schedule, retention_count, mode)
      )
  )]
  pub async fn update_backup_settings(
      destination_path: String,
      schedule: String,
      retention_count: String,
      mode: String,
  ) -> WebResult<()> {
      boundary!("update_backup_settings", {
          require_operator().await?;

          let schedule = BackupSchedule::parse(schedule.trim()).ok_or_else(|| {
              InternalError::validation(
                  "backup schedule must be a valid six-field cron expression",
              )
          })?;
          let retention_count = retention_count
              .trim()
              .parse::<usize>()
              .map_err(|_| InternalError::validation(
                  "backup retention count must be a non-negative integer",
              ))?;
          let mode = match mode.trim() {
              "directory" => BackupMode::Directory,
              "archive" => BackupMode::Archive,
              _ => return Err(InternalError::validation("backup mode must be directory or archive")),
          };
          let destination_path = {
              let s = destination_path.trim().to_owned();
              if s.is_empty() { None } else { Some(s) }
          };

          let config = BackupConfig { destination_path, schedule, retention_count, mode };
          let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
          site_config
              .set_backup_config(&config)
              .await
              .map_err(InternalError::storage)
      })
  }
  ```

- [ ] **Step 6: Fix any call sites in `web/src/pages/` that used `BackupSettings`**

  ```bash
  grep -rn "BackupSettings\|get_backup_settings\|backup_schedule\|backup_mode\|backup_retention" web/src/pages/ --include="*.rs"
  ```

  Update any page components that destructure `BackupSettings` fields to use `BackupConfig` fields instead. Field names are the same except types differ (`String` → typed). Display by calling `.as_str()` on `BackupSchedule`, `.to_string()` on `usize`, and matching on `BackupMode`.

- [ ] **Step 7: Build web**

  ```bash
  cargo build -p web 2>&1 | head -40
  ```

  Expected: no errors.

- [ ] **Step 8: Delete the now-empty `#[cfg(test)]` block** if it only tested the deleted helpers. If any tests remain, keep them.

- [ ] **Step 9: Commit**

  ```bash
  git add web/src/backup/mod.rs
  git commit -m "refactor(web): replace BackupSettings with common::backup::BackupConfig"
  ```

---

### Task 6: Full verification and close

- [ ] **Step 1: Run full verify**

  ```bash
  scripts/verify
  ```

  Expected: all checks pass.

- [ ] **Step 2: Update coverage manifest if needed**

  ```bash
  scripts/check-coverage 2>&1 | tail -10
  ```

  If coverage regressions are reported, investigate before updating the manifest. Do not update the manifest without confirming no real coverage was lost.

- [ ] **Step 3: Update the beads issue**

  ```bash
  bd update jaunder-czq --notes="Implemented: BackupConfig in common::backup replaces BackupSettings (web) and BackupWorkerConfig (server). SiteConfigStorage gains get_backup_config/set_backup_config default methods."
  bd close jaunder-czq
  ```

- [ ] **Step 4: Push**

  ```bash
  git push
  ```
