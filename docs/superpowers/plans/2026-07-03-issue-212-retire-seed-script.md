# Plan — issue #212: replace `seed-e2e-fixtures.sh` with `test-support` subcommands

- Issue: [#212](https://github.com/jaunder-org/jaunder/issues/212)
- Spec: `docs/superpowers/specs/2026-07-03-issue-212-retire-seed-script.md`
- **For agentic workers:** execute with **`jaunder-iterate`** (per-task
  implement → check → commit → review), delegating a task to a subagent via
  **`jaunder-dispatch`** when useful. Tick checkboxes in real time.

## Overview

Retire `scripts/seed-e2e-fixtures.sh` and the inline raw-SQL `site_config`
INSERTs entirely, moving all e2e fixture setup behind `test-support`
subcommands. Along the way, dedup the "timeline-visible seeded post" recipe into
one `storage::seed_rendered_post` helper behind a no-dep `storage/seed-posts`
feature. Order: land the shared helper (T1) and re-point the existing binary
seeder at it (T2); add the three new subcommands (T3-T5); migrate the two flake
e2e sites (T6); migrate the local runner and delete the script (T7).

## Global constraints

- Crates: `storage`, `test-support`, `common`, `server` (exact names). Rust
  edition 2021.
- **No `Co-Authored-By` trailer** on commits.
- Each commit: run `cargo xtask check` first (the pre-commit hook runs the full
  gate — fmt + clippy + Nix coverage/tests). See `jaunder-commit`.
- **`seed-posts` feature pulls NO deps** — `seed-posts = []`. The whole point is
  the binary reaches the recipe without linking `tempfile`/`rstest_reuse`.
- **Backend parity:** `seed_rendered_post` has no per-backend branching (it
  dispatches through `create_rendered_post`, which storage implements per
  backend), so the `test-support` smoke tests are **SQLite-only by design** —
  the same precedent as the existing `seed_tests` in `test-support/src/lib.rs`.
  The e2e matrix (`{sqlite,postgres}×{chromium,firefox}`) proves the
  dual-backend behaviour end-to-end. Do NOT add a `#[template]` dual-backend
  harness to the `test-support` crate.
- Coverage: the new `pub` fns (`create_user`, `set_site_config`, `reset_mail`,
  `seed_rendered_post`) are covered by the smoke tests / transitively; the
  classifier is line-based (see repo coverage policy).

---

## Task 1 — `storage/seed-posts` feature + `seed_rendered_post` helper

**Files**

- `storage/Cargo.toml` — add the feature and fold it into `test-support`:
  ```toml
  [features]
  test-utils = []
  seed-posts = []
  test-support = ["seed-posts", "dep:tempfile", "dep:rstest_reuse"]
  ```
- `storage/src/post_service.rs` — add the gated recipe helper next to
  `create_rendered_post` (all imports — `Slug`, `PostFormat`, `CreatePostError`,
  `AudienceTarget`, `chrono::Utc`, `PostStorage` — are already in this module):
  ```rust
  /// The single definition of "a timeline-visible seeded post": a public,
  /// Markdown-rendered post, published now iff `published`. Shared by
  /// `storage::test_support::seed_posts` (in-process) and the `test-support`
  /// binary's `seed_posts_for_user` (out-of-process) so the recipe — not just
  /// the row, but the Public audience + rendered HTML that make it visible —
  /// lives in one place. Gated so a normal `storage` build never compiles it,
  /// yet the `test-support` binary can reach it without the heavy
  /// `test-support` scaffolding (it enables only `seed-posts`).
  ///
  /// # Errors
  ///
  /// Returns `Err(CreatePostError)` if the storage write fails.
  #[cfg(any(test, feature = "seed-posts"))]
  pub async fn seed_rendered_post(
      posts: &dyn PostStorage,
      user_id: i64,
      slug: Slug,
      body: String,
      published: bool,
  ) -> Result<i64, CreatePostError> {
      create_rendered_post(
          posts,
          user_id,
          None,
          slug,
          body,
          PostFormat::Markdown,
          published.then(chrono::Utc::now),
          None,
          vec![AudienceTarget::Public],
      )
      .await
  }
  ```
  (`post_service::*` is re-exported at the crate root — `storage/src/lib.rs:50`
  — so this is reachable as `storage::seed_rendered_post` when the gate is on.)
- `storage/src/test_support.rs` — rewrite `seed_posts` (lines ~554-583) so its
  loop body calls the shared helper, preserving its `seed-{i}` / `# Post {i}`
  scheme, its `-> Vec<i64>` signature, and its panic-on-error contract:
  ```rust
  pub async fn seed_posts(
      state: &Arc<AppState>,
      user_id: i64,
      count: usize,
      published: bool,
  ) -> Vec<i64> {
      let mut ids = Vec::with_capacity(count);
      for i in 0..count {
          let id = crate::seed_rendered_post(
              &*state.posts,
              user_id,
              format!("seed-{i}").parse().expect("valid slug"),
              format!("# Post {i}\n\nbody"),
              published,
          )
          .await
          .expect("seed post should be created");
          ids.push(id);
      }
      ids
  }
  ```
  (`test_support` is gated `#[cfg(any(test, feature = "test-support"))]` and
  `test-support` now implies `seed-posts`, so `seed_rendered_post` is always
  present wherever `seed_posts` compiles.)

**Test** — behaviour-preserving refactor; the existing storage tests that use
`seed_posts` (dual-backend, e.g. `posts.rs`, plus `test-support`'s own
`seed_tests`) are the regression check. No new test.

**Run**

- `cargo build -p storage --features seed-posts` — **PASS** (proves the feature
  compiles standalone, pulls no new deps, `seed_rendered_post` is reachable
  without `test-support`).
- `cargo nextest run -p storage` — **PASS** (existing seed-using tests green).

**Commit** (after `cargo xtask check`):
`refactor(storage): extract seed_rendered_post behind seed-posts feature (#212)`

---

## Task 2 — point the `test-support` binary's seeder at the shared helper

**Files**

- `test-support/Cargo.toml` — enable `seed-posts` on the **normal** storage dep
  and update the comment (the dev-dep keeps `test-support` for the smoke tests'
  `storage::test_support` harness):
  ```toml
  [dependencies]
  storage = { workspace = true, features = ["seed-posts"] }
  # ...
  # The seed core shares storage's `seed_rendered_post` (the lightweight
  # `seed-posts` feature, no extra deps). The heavy `test-support` scaffolding
  # (`storage::test_support`, tempfile, rstest_reuse) is a dev-dependency ONLY,
  # for the smoke tests below — the shipped binary never links it.
  [dev-dependencies]
  storage = { workspace = true, features = ["test-support"] }
  tempfile = { workspace = true }
  ```
- `test-support/src/lib.rs` — in `seed_posts_for_user` (lines ~78-90) replace
  the inline `create_rendered_post(...)` call with the shared helper, keeping
  the username lookup, the per-`prefix` `seed_slug`/`seed_body` scheme, and the
  slug-parse error:
  ```rust
  let slug = seed_slug(prefix, i).parse().map_err(|_| {
      anyhow::anyhow!("generated slug invalid for prefix {prefix:?} index {i}")
  })?;
  let id = storage::seed_rendered_post(&*state.posts, user.user_id, slug, seed_body(prefix, i), published)
      .await
      .map_err(|e| anyhow::anyhow!("seed post {i} failed: {e:?}"))?;
  ids.push(id);
  ```
  (Drop the now-unused `create_rendered_post` / `PostFormat` imports if the
  compiler flags them; keep `AudienceTarget`-free.)

**Test** — behaviour-preserving; the existing `seed_tests` smoke tests
(`seeds_public_published_posts_visible_to_a_non_author`,
`rejects_a_prefix_that_cannot_form_a_valid_slug`) are the regression check.

**Run**

- `cargo build -p test-support` — **PASS** (the normal binary build links
  `seed-posts`, NOT `test-support`; confirm no `tempfile`/`rstest_reuse` in
  `cargo tree -p test-support -e no-dev`).
- `cargo nextest run -p test-support` — **PASS**.

**Commit**:
`refactor(test-support): seed via storage::seed_rendered_post (#212)`

---

## Task 3 — `create-user` subcommand

**Files**

- `test-support/src/lib.rs` — add the helper + a SQLite-only smoke test:
  ```rust
  /// Create a user through the real `UserStorage::create_user` path — the same
  /// call `jaunder user-create` makes (server::commands::cmd_user_create), minus
  /// that command's `CliBypass` registration metric (test seeding must not emit
  /// observability noise). Assumes a freshly-`init`'d DB; no upsert.
  ///
  /// # Errors
  /// Returns `Err` if the username/password are invalid or the user cannot be created.
  pub async fn create_user(
      state: &Arc<AppState>,
      username: &str,
      password: &str,
      display_name: Option<&str>,
      operator: bool,
  ) -> anyhow::Result<i64> {
      let uname = username.parse::<common::username::Username>()
          .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
      let pw = password.parse::<common::password::Password>()
          .map_err(|e| anyhow::anyhow!("invalid password: {e}"))?;
      let id = state.users.create_user(&uname, &pw, display_name, operator).await?;
      Ok(id)
  }
  ```
  Smoke test (mirrors `seed_tests` setup — `test_sqlite_state_with_pool`):
  ```rust
  #[tokio::test]
  async fn create_user_creates_a_lookupable_operator() {
      let base = tempfile::TempDir::new().unwrap();
      let (state, _pool) = test_support::test_sqlite_state_with_pool(&base).await;
      let id = create_user(&state, "testoperator", "testpassword123", None, true).await.unwrap();
      let u = state.users.get_user_by_username(&"testoperator".parse().unwrap()).await.unwrap().unwrap();
      assert_eq!(u.user_id, id);
      // duplicate username errors
      assert!(create_user(&state, "testoperator", "testpassword123", None, false).await.is_err());
  }
  ```
  (Confirm the operator-flag field name on the user record while implementing;
  if the record doesn't expose it, assert operator behaviour via a capability
  check instead of the raw field.)
- `test-support/src/main.rs` — add the `Commands::CreateUser` variant (mirror
  `SeedPosts`' `--db`/`JAUNDER_DB` `DbConnectOptions`) and dispatch:
  ```rust
  /// Create a fixture user through the real storage path.
  CreateUser {
      #[arg(long, env = "JAUNDER_DB")]
      db: DbConnectOptions,
      #[arg(long)]
      username: String,
      #[arg(long)]
      password: String,
      #[arg(long)]
      display_name: Option<String>,
      #[arg(long)]
      operator: bool,
  },
  ```
  ```rust
  Commands::CreateUser { db, username, password, display_name, operator } => {
      let state = storage::open_existing_database(&db).await?;
      let id = create_user(&state, &username, &password, display_name.as_deref(), operator).await?;
      eprintln!("created user {username} with id {id}");
  }
  ```

**Run**: `cargo nextest run -p test-support create_user` — **FAIL** (no test) →
**PASS** after implementing.

**Commit**: `feat(test-support): add create-user subcommand (#212)`

---

## Task 4 — `set-site-config` subcommand

**Files**

- `test-support/src/lib.rs`:
  ```rust
  /// Set a `site_config` key through `SiteConfigStorage::set` (an upsert) —
  /// replaces the raw-SQL INSERTs the e2e sites use for
  /// `site.registration_policy` and `feeds.websub_hub_url`.
  ///
  /// # Errors
  /// Returns `Err` if the storage write fails.
  pub async fn set_site_config(state: &Arc<AppState>, key: &str, value: &str) -> anyhow::Result<()> {
      state.site_config.set(key, value).await?;
      Ok(())
  }
  ```
  Smoke test: `set` then `get` round-trips; a second `set` overwrites (upsert).
- `test-support/src/main.rs` —
  `Commands::SetSiteConfig { db (--db/JAUNDER_DB), key, value }` + dispatch
  calling `set_site_config`.

**Run**: `cargo nextest run -p test-support set_site_config` — FAIL → PASS.

**Commit**: `feat(test-support): add set-site-config subcommand (#212)`

---

## Task 5 — `reset-mail` subcommand

**Files**

- `test-support/src/lib.rs` — `rm -f` semantics (missing file is success):
  ```rust
  /// Reset the mail-capture file: delete `path` if present. Missing is success
  /// (`rm -f` semantics); any other error propagates. Not storage-linked.
  ///
  /// # Errors
  /// Returns `Err` if the file exists but cannot be removed.
  pub fn reset_mail(path: &std::path::Path) -> anyhow::Result<()> {
      match std::fs::remove_file(path) {
          Ok(()) => Ok(()),
          Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
          Err(e) => Err(anyhow::anyhow!("reset-mail: {}: {e}", path.display())),
      }
  }
  ```
  Smoke test: create a temp file → `reset_mail` deletes it (`!exists`); a second
  `reset_mail` on the now-missing path returns `Ok`.
- `test-support/src/main.rs` —
  `Commands::ResetMail { path (--path, env = "JAUNDER_MAIL_CAPTURE_FILE", PathBuf) }`
  — **no `--db`** — dispatch calls `reset_mail(&path)`.

**Run**: `cargo nextest run -p test-support reset_mail` — FAIL → PASS.

**Commit**: `feat(test-support): add reset-mail subcommand (#212)`

---

## Task 6 — migrate the two `flake.nix` e2e sites

**Files** — `flake.nix`. In **`mkE2eSqliteCheck`** replace the two
`sqlite3 … INSERT … site_config …` lines (~830-831) and the
`cd /var/lib/jaunder && JAUNDER_BIN=… && JAUNDER_MAIL_CAPTURE_FILE=… && ${./scripts/seed-e2e-fixtures.sh}`
block (~834-837) with:

```
test-support create-user     --db sqlite:/var/lib/jaunder/data/jaunder.db --username testlogin    --password testpassword123
test-support create-user     --db sqlite:/var/lib/jaunder/data/jaunder.db --username testnoemail  --password testpassword123
test-support create-user     --db sqlite:/var/lib/jaunder/data/jaunder.db --username testoperator --password testpassword123 --operator
test-support set-site-config --db sqlite:/var/lib/jaunder/data/jaunder.db --key site.registration_policy --value open
test-support set-site-config --db sqlite:/var/lib/jaunder/data/jaunder.db --key feeds.websub_hub_url      --value https://hub.test.local/
test-support reset-mail      --path /var/lib/jaunder/mail.jsonl
```

In **`mkE2ePostgresCheck`** replace the two `psql … INSERT … site_config …`
lines (~989-990) and the script block (~991-997) with the same six commands but
`--db postgres://jaunder:testpassword@127.0.0.1/jaunder` (the URL already set as
`JAUNDER_DB` there — passing `--db` explicitly keeps both sites uniform).

- `testSupportBin` is already on both VMs' `environment.systemPackages` (~793 /
  ~912) — no wiring change.
- The **SQLite** site change is load-bearing: the deleted script relied on cwd +
  default path with no `--db`; `test-support` needs the explicit
  `sqlite:/var/lib/jaunder/data/jaunder.db` URL (the DB the raw INSERTs
  targeted).

**Test** — no unit test; the e2e Nix checks are the proof.

**Run** (heavy — Bash background mode):
`devtool run -- cargo xtask e2e sqlite chromium` and `… e2e postgres chromium` —
**PASS** (fixtures seed; auth/timeline/feeds tests green). Full
`cargo xtask validate` is the final gate (Task 7 / ship).

**Commit**:
`test-infra(e2e): seed flake e2e via test-support subcommands (#212)`

---

## Task 7 — migrate the local runner, delete the script

**Files**

- `end2end/run-e2e.sh` — replace the two `sqlite3 … INSERT … site_config …`
  lines (40-43) and the
  `"$(git rev-parse --show-toplevel)/scripts/seed-e2e-fixtures.sh"` invocation
  (45) with `test-support` calls. The local path is always SQLite:
  ```bash
  TS="../target/debug/test-support"; DB="sqlite:${JAUNDER_DB_PATH}"
  "$TS" create-user     --db "$DB" --username testlogin    --password testpassword123
  "$TS" create-user     --db "$DB" --username testnoemail  --password testpassword123
  "$TS" create-user     --db "$DB" --username testoperator --password testpassword123 --operator
  "$TS" set-site-config --db "$DB" --key site.registration_policy --value open
  "$TS" set-site-config --db "$DB" --key feeds.websub_hub_url      --value https://hub.test.local/
  "$TS" reset-mail      --path "$JAUNDER_MAIL_CAPTURE_FILE"
  ```
  **The `test-support` binary must exist for the local run.** `cargo-leptos`
  builds only the server, so add `cargo build -p test-support` to the local
  wrapper — Read `scripts/e2e-local.sh` and put the build where it builds the
  server binary (or guard it in `run-e2e.sh`: build if `$TS` is not executable).
  Drop the now-unused `JAUNDER_BIN` export if nothing else uses it.
- Delete `scripts/seed-e2e-fixtures.sh` (both consumers — flake T6, runner here
  — are migrated). Confirm no remaining references:
  `rg -n 'seed-e2e-fixtures' -- flake.nix end2end scripts` returns nothing.

**Test** — local e2e (`scripts/e2e-local.sh`) — **PASS** end-to-end.

**Run**: `devtool run -- cargo xtask validate` — the full local gate (static +
coverage + the four e2e combos). **PASS** = done.

**Commit**:
`test-infra(e2e): migrate local runner to test-support, delete seed script (#212)`

---

## Self-review checklist

- [ ] `seed-posts = []` pulls no deps; `test-support` implies it.
- [ ] `test-support` binary's **non-dev** tree has no `tempfile`/`rstest_reuse`
      (`cargo tree -p test-support -e no-dev`).
- [ ] `storage::test_support::seed_posts` signature/panic contract unchanged
      (all its dual-backend callers stay green).
- [ ] New `test-support` smoke tests are SQLite-only (no dual-backend template);
      they don't trip the `test-backend-pattern` guard.
- [ ] Both flake sites + the local runner pass `--db` explicitly; SQLite sites
      use `sqlite:<path>`.
- [ ] `scripts/seed-e2e-fixtures.sh` deleted; zero remaining references.
- [ ] No `Co-Authored-By`. Each commit references `(#212)`.
