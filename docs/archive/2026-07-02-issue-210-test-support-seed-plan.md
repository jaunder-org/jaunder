# `test-support` Seed Binary — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the sequential per-post `POST /api/create_post` seed loops in
three heavy e2e timeline tests with a single shell-out to a new `test-support`
binary that seeds posts in-process through the real `storage` code path.

**Architecture:** A new workspace binary crate `test-support` links `storage`
(via its `test-support` feature), opens the live e2e DB with
`storage::open_existing_database`, resolves a username → `user_id`, and loops
`storage::create_rendered_post` to seed timeline-visible published posts. It is
a _separate binary_ from `jaunder` — never in the release artifact or the NixOS
module — built by crane and placed on the e2e VM PATH. The three Playwright
tests shell out to it mid-test, after their runtime registration.

**Tech Stack:** Rust (clap, tokio), `storage` crate, crane/Nix (flake.nix),
Playwright (TypeScript, `node:child_process`).

## Global Constraints

- **No production surface.** The `jaunder` binary and the `services.jaunder`
  NixOS module must not reference `test-support`. `resolver = "2"` keeps
  `storage/test-support` out of the `-p jaunder` build — do not enable that
  feature anywhere in the `jaunder` graph.
- **One storage code path.** Seed through `storage::create_rendered_post` — no
  raw SQL, no per-backend branching in the caller. Backend selection comes from
  `--db` (`sqlite:` vs `postgres://`) exactly as the server does.
- **Slug uniqueness is `(user_id, date(published_at|created_at), slug)`**
  (per-user, per-day). Each test registers a _fresh_ user, so per-user
  deterministic slugs are safe.
- **Seeded content must satisfy the tests' assertions**, which match the
  rendered body H1: a post seeded with body prefix `P` and index `i` renders
  text `P i` (e.g. `Timeline Post 50`). Newest post = highest index (ordering by
  `published_at DESC`, ties broken by `post_id DESC`, as
  `storage::test_support::seed_posts` already relies on).
- **Keep the current post counts** (`:305`→51, `:349`→2×26, `:410`→51 self + 2
  other).
- **Gate per task:** the pre-commit hook runs full `cargo xtask check` (fmt +
  clippy + Nix coverage/tests). Run `cargo xtask check` first so it's clean
  (`jaunder-commit`). One clean commit per task. **No `Co-Authored-By`
  trailer.** For e2e verification use `cargo xtask e2e sqlite chromium` while
  iterating; full `cargo xtask validate` (all four
  `{sqlite,postgres}×{chromium,firefox}` combos) is the final gate.
- In a worktree, run the gate via the Bash tool (or `cd <worktree> &&`) —
  `ctx_execute` targets the main repo.

---

### Task 1: File the follow-up issue (separable concern) — filed #212

Per `jaunder-start` step 5 / `jaunder-plan` scope rule, the separable concern is
filed as the first task — not folded in silently.

**Files:** none (GitHub issue only).

- [x] **Step 1: File the issue** via `jaunder-issues` conventions (GitHub MCP
      `issue_write` or `gh issue create`), repo `jaunder-org/jaunder`, milestone
      "E2E test suite", type Task, label `test-infra`:
  - **Title:**
    `test-infra(e2e): replace seed-e2e-fixtures.sh entirely with test-support subcommands`
  - **Body (verbatim intent):**
    > Follow-on to #210. #210 introduces the `test-support` binary (ADR
    > `test-support-seed-binary`) with a `seed-posts` subcommand. Extend
    > `test-support` to **replace `scripts/seed-e2e-fixtures.sh` in its
    > entirety** — fixture-user creation (currently `jaunder user-create`), the
    > `site.registration_policy` config step (today the _"no CLI for that yet"_
    > raw-SQL hack noted in the script header), and the mail-capture reset — so
    > all e2e fixture setup goes through one storage-linked tool instead of a
    > shell script + raw SQL. Retire the script and its two flake.nix invocation
    > sites (`mkE2eSqliteCheck`, `mkE2ePostgresCheck`). Possible future: migrate
    > `storage::test_support::seed_posts` out of `storage` into `test-support`
    > (noted in #210's ADR; not committed there).

- [x] **Step 2: Record the issue number** in this plan's Task 1 heading (append
      `— filed #<N>`) so `jaunder-ship` can reference it. No commit needed (docs
      updated with the plan checkboxes at ship).

---

### Task 2: Create the `test-support` crate with a tested seed core

**Files:**

- Create: `test-support/Cargo.toml`
- Create: `test-support/src/main.rs`
- Create: `test-support/src/lib.rs`
- Modify: `Cargo.toml` (root workspace `members`)
- Modify: `coverage-baseline.json` (residual `main.rs` wiring lines, via
  `coverage reanchor`)

**Interfaces:**

- Consumes:
  `storage::open_existing_database(&DbConnectOptions) -> sqlx::Result<Arc<AppState>>`;
  `storage::DbConnectOptions` (the `--db` value type, `sqlite:`/`postgres://`);
  `storage::create_rendered_post(&dyn PostStorage, user_id, title, slug, body, format, published_at, summary, audiences)`;
  `UserStorage::get_user_by_username(&Username) -> sqlx::Result<Option<UserRecord>>`
  (via `AppState.users`); the type imports used at the top of
  `storage/src/test_support.rs` (`PostFormat`, `Slug`/`Username` parse,
  `AudienceTarget`).
- Produces: a `test-support` binary exposing
  `test-support seed-posts --db <url> --username <name> --count <N> --body-prefix <P> [--published]`
  and library fns `seed_body`, `seed_slug`, `seed_posts_for_user` (unit-tested).

- [x] **Step 1: Add the crate to the workspace**

  In root `Cargo.toml`, add `"test-support"` to `[workspace] members` (keep
  alphabetical):

  ```toml
  members = [
    "common",
    "csr",
    "server",
    "storage",
    "test-support",
    "web"
  ]
  ```

- [x] **Step 2: Write `test-support/Cargo.toml`**

  ```toml
  [package]
  name = "test-support"
  version = "0.1.0"
  edition = "2021"
  publish = false

  [[bin]]
  name = "test-support"
  path = "src/main.rs"

  [dependencies]
  storage = { workspace = true, features = ["test-support"] }
  common = { workspace = true }
  clap = { workspace = true, features = ["derive"] }
  tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
  anyhow = { workspace = true }

  [dev-dependencies]
  storage = { workspace = true, features = ["test-support"] }
  tempfile = { workspace = true }   # for test_sqlite_state_with_pool(&TempDir)
  ```

  (`tempfile` is used only by the unit test in Step 5. If it isn't in root
  `[workspace.dependencies]`, copy the version spec from `storage/Cargo.toml`'s
  optional `tempfile` dep.)

  If `anyhow`/`clap`/`tokio` are not in root `[workspace.dependencies]`, mirror
  the version specs `server/Cargo.toml` uses (copy its `clap`/`tokio`/`anyhow`
  lines).

- [x] **Step 3: Write the failing unit tests for the pure content helpers** in
      `test-support/src/lib.rs`:

  ```rust
  //! Test-only tooling that reaches jaunder's storage layer from OUTSIDE the server
  //! process (e.g. a live-server Playwright e2e). Sibling of the in-process
  //! `storage::test_support` module; here we cross a process boundary. Never linked
  //! into the `jaunder` production binary (see ADR test-support-seed-binary).

  use std::sync::Arc;

  use storage::test_support as _; // ensures the feature is wired; real imports below.

  /// The rendered-body text for seeded post `i` under `prefix`. Matches the shape the
  /// e2e tests assert on: the article's H1 renders `"{prefix} {i}"`.
  pub fn seed_body(prefix: &str, i: usize) -> String {
      format!("# {prefix} {i}\n\nBody for {prefix} {i}")
  }

  /// A per-user-unique slug string for seeded post `i` under `prefix`
  /// (lowercased, spaces → hyphens, index suffix). Valid `Slug` input.
  pub fn seed_slug(prefix: &str, i: usize) -> String {
      let base: String = prefix
          .to_lowercase()
          .chars()
          .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
          .collect();
      let base = base.trim_matches('-');
      format!("{base}-{i}")
  }

  #[cfg(test)]
  mod content_tests {
      use super::*;

      #[test]
      fn seed_body_renders_prefix_and_index() {
          assert_eq!(seed_body("Timeline Post", 50), "# Timeline Post 50\n\nBody for Timeline Post 50");
      }

      #[test]
      fn seed_slug_is_slug_safe() {
          assert_eq!(seed_slug("Timeline Post", 0), "timeline-post-0");
          assert_eq!(seed_slug("Home Feed Mine", 12), "home-feed-mine-12");
      }
  }
  ```

- [x] **Step 4: Run the helper tests, verify they fail (not yet compiled)**

  Run:
  `cargo nextest run -p test-support seed_body_renders_prefix_and_index seed_slug_is_slug_safe`
  Expected: FAIL to compile / not found — crate doesn't build yet (no
  `main.rs`).

- [x] **Step 5: Add the DB-touching seed core + its test** to
      `test-support/src/lib.rs`. Mirror the type imports from the top of
      `storage/src/test_support.rs` (adjust paths to the crate-public re-exports
      — `storage::create_rendered_post`, `storage::PostFormat`, the
      `Slug`/`Username` types, `common::visibility::AudienceTarget`):

  ```rust
  use storage::{create_rendered_post, AppState, PostFormat};
  // NOTE: resolve Slug / Username / AudienceTarget to the exact paths used at the top of
  // storage/src/test_support.rs (they are re-exported from `common`/`storage`).

  /// Seed `count` published-or-draft posts for `username`, through the real
  /// `create_rendered_post` path. Bodies/slugs derive from `prefix` (see `seed_body`
  /// / `seed_slug`). Returns created ids in creation order (oldest → newest).
  pub async fn seed_posts_for_user(
      state: &Arc<AppState>,
      username: &str,
      count: usize,
      published: bool,
      prefix: &str,
  ) -> anyhow::Result<Vec<i64>> {
      let uname = username.parse().map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
      let user = state
          .users
          .get_user_by_username(&uname)
          .await?
          .ok_or_else(|| anyhow::anyhow!("no such user: {username}"))?;

      let mut ids = Vec::with_capacity(count);
      for i in 0..count {
          let published_at = if published { Some(chrono::Utc::now()) } else { None };
          let id = create_rendered_post(
              &*state.posts,
              user.user_id,
              None,
              seed_slug(prefix, i).parse().map_err(|_| anyhow::anyhow!("bad slug"))?,
              seed_body(prefix, i),
              PostFormat::Markdown,
              published_at,
              None,
              vec![common::visibility::AudienceTarget::Public],
          )
          .await
          .map_err(|e| anyhow::anyhow!("seed post {i} failed: {e:?}"))?;
          ids.push(id);
      }
      Ok(ids)
  }

  #[cfg(test)]
  mod seed_tests {
      use super::*;
      use storage::test_support;

      #[tokio::test]
      async fn seeds_public_published_posts_visible_to_a_non_author() {
          // Verified names/signatures:
          //   test_support::test_sqlite_state_with_pool(&TempDir) -> (Arc<AppState>, SqlitePool)
          //   test_support::seed_user(&state) -> i64  (creates user "testuser")
          //   ViewerIdentity::Anonymous  (unit variant — no channel id needed)
          let base = tempfile::TempDir::new().unwrap();
          let (state, _pool) = test_support::test_sqlite_state_with_pool(&base).await;
          let _uid = test_support::seed_user(&state).await; // "testuser"

          let ids = seed_posts_for_user(&state, "testuser", 3, true, "Timeline Post")
              .await
              .expect("seed ok");
          assert_eq!(ids.len(), 3);

          // The point of the tool: seeded posts are PUBLIC + published, so a
          // non-author (Anonymous) viewer sees them. A bare `posts` insert with no
          // `post_audiences` row would be private and this would return 0.
          // `list_published_by_user` arg shape mirrors web/src/posts/listing.rs:199
          // (&Username, cursor: Option<&_>, limit, &ViewerIdentity, now):
          let page = state
              .posts
              .list_published_by_user(
                  &"testuser".parse().unwrap(),
                  None,
                  10,
                  &storage::ViewerIdentity::Anonymous,
                  chrono::Utc::now(),
              )
              .await
              .expect("list ok");
          assert_eq!(page.len(), 3);
      }
  }
  ```

  The helper names above are verified against
  `storage/src/test_support.rs:510,591` and `common/src/visibility.rs:47`. If
  `list_published_by_user`'s exact return type needs a different length
  accessor, mirror the call in `server/tests/web/web_posts.rs` — do not invent a
  signature.

- [x] **Step 6: Run the seed-core test, verify it fails**

  Run:
  `cargo nextest run -p test-support seeds_published_posts_visible_to_the_author`
  Expected: FAIL — `seed_posts_for_user` not yet compiling / assertions unmet.

- [x] **Step 7: Write `test-support/src/main.rs` (thin clap shell)**

  ```rust
  use clap::{Parser, Subcommand};
  use storage::DbConnectOptions;

  #[derive(Parser)]
  #[command(name = "test-support", about = "Out-of-process test/e2e helpers (never shipped in jaunder)")]
  struct Cli {
      #[command(subcommand)]
      command: Commands,
  }

  #[derive(Subcommand)]
  enum Commands {
      /// Seed N posts for a user through the real storage path.
      SeedPosts {
          /// Database URL (`sqlite:...` or `postgres://...`). Same as the server's.
          #[arg(long, env = "JAUNDER_DB")]
          db: DbConnectOptions,
          #[arg(long)]
          username: String,
          #[arg(long)]
          count: usize,
          /// Body/slug prefix; post `i` renders "<prefix> i".
          #[arg(long)]
          body_prefix: String,
          /// Publish immediately (else drafts).
          #[arg(long)]
          published: bool,
      },
  }

  #[tokio::main]
  async fn main() -> anyhow::Result<()> {
      let cli = Cli::parse();
      match cli.command {
          Commands::SeedPosts { db, username, count, body_prefix, published } => {
              let state = storage::open_existing_database(&db).await?;
              let ids = test_support::seed_posts_for_user(&state, &username, count, published, &body_prefix).await?;
              eprintln!("seeded {} posts for {username}", ids.len());
          }
      }
      Ok(())
  }
  ```

  If `DbConnectOptions` does not implement `clap`'s value parsing directly, take
  `db: String` and parse it via the same `FromStr`/constructor
  `server/src/cli.rs` uses for its `StorageArgs.db` field (copy that mechanism).

- [x] **Step 8: Run the full crate test suite, verify pass**

  Run: `cargo nextest run -p test-support` Expected: PASS (all helper +
  seed-core tests).

- [x] **Step 9: Run the gate; absorb residual `main.rs` coverage**

  Run: `cargo xtask check` If coverage flags new uncovered lines in
  `test-support/src/main.rs` (the `#[tokio::main]` wiring, which unit tests
  can't reach), record them into the baseline: Run:
  `cargo xtask coverage reanchor` (records the new crate's accepted gaps), then
  re-run `cargo xtask check` to confirm green. The seed _logic_ is covered by
  the lib tests; only the thin `main` shell is baselined. Note this in the
  eventual PR per the coverage-baseline policy (tooling glue, no
  server-fn/storage logic baselined).

- [x] **Step 10: Commit**

  ```bash
  git add Cargo.toml test-support/ coverage-baseline.json
  git commit -m "feat(test-support): add seed-posts binary linking storage"
  ```

---

### Task 3: Build `test-support` in the flake and expose it to the e2e VMs

**Files:**

- Modify: `flake.nix` (add `testSupportBin`; add to both VMs'
  `environment.systemPackages`; inject `JAUNDER_DB` into the Playwright exec env
  in `e2eRunAndCapture`)

**Interfaces:**

- Consumes: `commonArgs`, `cargoArtifacts` (flake.nix:331–359).
- Produces: `test-support` on the e2e VM PATH; `JAUNDER_DB` set in the
  Playwright process env for both backends.

- [x] **Step 1: Add the crane package** next to `jaunderBin` (flake.nix ~359):

  ```nix
  testSupportBin = craneLib.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      cargoExtraArgs = "-p test-support";
      doCheck = false;
    }
  );
  ```

- [x] **Step 2: Put it on both VMs' PATH.** In `mkE2eSqliteCheck` (env ~770) and
      `mkE2ePostgresCheck` (env ~887), add `testSupportBin` to
      `environment.systemPackages`:

  ```nix
  environment.systemPackages = [
    pkgs.sqlite            # (postgres VM: pkgs.postgresql_16)
    pkgs.opentelemetry-collector-contrib
    testSupportBin
  ];
  ```

- [x] **Step 3: Inject `JAUNDER_DB` into the Playwright exec env.** In the
      `e2eRunAndCapture` Playwright invocation (flake.nix ~705, the
      `machine.execute("cd /tmp/e2e" + " && …")` string), add a `JAUNDER_DB=`
      clause so the test process (and the `test-support` child it spawns, which
      reads `env = "JAUNDER_DB"`) points at the _same_ DB the server uses.
      Thread the value in per combo:
  - SQLite VM: `+ " JAUNDER_DB=sqlite:/var/lib/jaunder/data/jaunder.db"`
  - Postgres VM:
    `+ " JAUNDER_DB=postgres://jaunder:testpassword@127.0.0.1/jaunder"`

  If `e2eRunAndCapture` is shared across combos, add a `jaunderDb` parameter to
  it and pass the backend-appropriate value from `mkE2eSqliteCheck` /
  `mkE2ePostgresCheck` (mirror how `browser`/`traceId` are already threaded).

- [x] **Step 4: Verify the flake evaluates and the package builds**

  Run: `cargo xtask check` (evaluates flake for the coverage/check derivations)
  and `nix build .#packages.x86_64-linux.jaunder` sanity, then build the tool:
  `nix build .#packages.x86_64-linux.test-support` **if** exposed as a package —
  otherwise confirm it builds transitively by evaluating one e2e check:
  `nix build --no-link .#checks.x86_64-linux.e2e-sqlite-chromium --dry-run`
  Expected: evaluation succeeds, `testSupportBin` resolves.

  If `test-support` should be a named package output, add it under `packages`
  alongside `jaunder` (mirror that attr) so it's directly buildable.

- [x] **Step 5: Commit**

  ```bash
  git add flake.nix
  git commit -m "build(e2e): build test-support and expose it to the e2e VMs"
  ```

---

### Task 4: Convert `:305` (per-user timeline) to tool seeding

**Files:**

- Create: `end2end/tests/seed.ts` (the `seedPostsViaTool` helper)
- Modify: `end2end/tests/posts.spec.ts` (the `:305` test)

**Interfaces:**

- Consumes: `test-support` on PATH; `JAUNDER_DB` in `process.env` (Task 3).
- Produces: `seedPostsViaTool(username, count, bodyPrefix, opts?)` for Tasks
  5–6.

- [x] **Step 1: Write the helper** `end2end/tests/seed.ts`:

  ```typescript
  import { execFileSync } from "node:child_process";

  /**
   * Seed `count` published posts for `username` via the `test-support` binary
   * (one in-process storage write per post — no HTTP round-trip). Post `i` renders
   * an H1 of `"${bodyPrefix} ${i}"`. Runs synchronously; the tool reads JAUNDER_DB
   * from the environment (set by the nix e2e harness).
   */
  export function seedPostsViaTool(
    username: string,
    count: number,
    bodyPrefix: string,
    opts: { published?: boolean } = {},
  ): void {
    const args = [
      "seed-posts",
      "--username",
      username,
      "--count",
      String(count),
      "--body-prefix",
      bodyPrefix,
    ];
    if (opts.published ?? true) args.push("--published");
    execFileSync("test-support", args, { stdio: "pipe", env: process.env });
  }
  ```

- [x] **Step 2: Rewrite the `:305` seed block.** In `posts.spec.ts`, replace the
      `perf.timed("seed_posts", …)` loop (the `for` over
      `createPublishedPostViaApi(page, \`Timeline Post ${i}\`)`) with:

  ```typescript
  import { seedPostsViaTool } from "./seed";
  // ...
  const username = await register(page, firstNavigationTimeoutMs);

  await perf.timed("seed_posts", async () => {
    seedPostsViaTool(
      username,
      TIMELINE_PAGE_SIZE + TIMELINE_OVERFLOW_COUNT,
      "Timeline Post",
    );
  });
  ```

  Leave every assertion unchanged — `article.j-post` count `TIMELINE_PAGE_SIZE`,
  first = `Timeline Post ${TIMELINE_PAGE_SIZE}`, last after Load more =
  `Timeline Post 0`. These hold because `seed_body("Timeline Post", i)` renders
  `# Timeline Post i` and ordering is newest-first.

- [x] **Step 3: Run the tsc gate + the e2e combo**

  Run: `cargo xtask check --no-test` (includes the `end2end` `tsc --noEmit`
  gate, #169). Then: `cargo xtask e2e sqlite chromium` Expected: the `:305` test
  passes; no `SQLITE_BUSY` in the log. (Grep the parked log for
  `SQLITE_BUSY`/`database is locked` — expect none.)

- [x] **Step 4: Commit**

  ```bash
  git add end2end/tests/seed.ts end2end/tests/posts.spec.ts
  git commit -m "test(e2e): seed :305 per-user timeline via test-support"
  ```

---

### Task 5: Convert `:349` (unauthenticated local timeline, two authors)

**Files:**

- Modify: `end2end/tests/posts.spec.ts` (the `:349` test)

**Interfaces:**

- Consumes: `seedPostsViaTool` (Task 4).

- [x] **Step 1: Rewrite both author seed blocks.** Replace the two
      `createPublishedPostViaApi` loops with tool calls, keeping the
      two-browser-context / two-user structure and the
      `LOCAL_TIMELINE_AUTHOR_COUNT` count:

  ```typescript
  await perf.timed("seed_author_one", async () => {
    const u1 = await register(page, firstNavigationTimeoutMs);
    seedPostsViaTool(u1, LOCAL_TIMELINE_AUTHOR_COUNT, "Local Author One");
  });

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  await perf.timed("seed_author_two", async () => {
    const u2 = await register(secondPage, firstNavigationTimeoutMs);
    seedPostsViaTool(u2, LOCAL_TIMELINE_AUTHOR_COUNT, "Local Author Two");
  });
  ```

  (The current test discards `register`'s return; capture it as `u1`/`u2` for
  the tool.) Leave the guest-context assertions (`>= TIMELINE_PAGE_SIZE`,
  Load-more growth, `jaunder.local` title) unchanged — this test asserts
  counts/growth, not titles.

- [x] **Step 2: Run tsc + e2e combo**

  Run: `cargo xtask check --no-test` then `cargo xtask e2e sqlite chromium`
  Expected: `:349` passes (allowing the documented environmental flake — re-run
  once if a lone `:349` timeout appears, per
  [[project_csr_e2e_local_heavy_test_flake]]); no `SQLITE_BUSY`.

- [x] **Step 3: Commit**

  ```bash
  git add end2end/tests/posts.spec.ts
  git commit -m "test(e2e): seed :349 local timeline authors via test-support"
  ```

---

### Task 6: Convert `:410` (authenticated home feed, self + other)

**Files:**

- Modify: `end2end/tests/posts.spec.ts` (the `:410` test)

**Interfaces:**

- Consumes: `seedPostsViaTool` (Task 4).

- [x] **Step 1: Rewrite the self + other seed blocks.** Confirmed safe: the
      `/app` feed reads live via `list_published_by_user`
      (web/src/posts/listing.rs:199), which the seeded posts populate directly —
      no feed-event emission needed.

  ```typescript
  await perf.timed("seed_self", async () => {
    const me = await register(page, firstNavigationTimeoutMs);
    seedPostsViaTool(me, HOME_FEED_SELF_COUNT, "Home Feed Mine");
  });

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  await perf.timed("seed_other", async () => {
    const other = await register(secondPage, firstNavigationTimeoutMs);
    seedPostsViaTool(other, HOME_FEED_OTHER_COUNT, "Home Feed Other");
  });
  ```

  Leave assertions unchanged: first page `TIMELINE_PAGE_SIZE`, first =
  `Home Feed Mine ${HOME_FEED_SELF_COUNT - 1}`, body **not** containing
  `Home Feed Other`, Load-more → `HOME_FEED_SELF_COUNT` total. The "not Home
  Feed Other" assertion is the loud-failure guard the spec required: if seeding
  silently produced an empty/leaky feed the count or exclusion assertion fails
  rather than passing vacuously.

- [x] **Step 2: Run tsc + e2e combo**

  Run: `cargo xtask check --no-test` then `cargo xtask e2e sqlite chromium`
  Expected: `:410` passes; no `SQLITE_BUSY`.

- [x] **Step 3: Confirm `createPublishedPostViaApi` is now unused** and remove
      it (and any now-unused imports) if no test references it. Search: Run:
      `rg -n 'createPublishedPostViaApi' end2end/tests` If zero non-definition
      hits, delete the function (posts.spec.ts:18–33). Otherwise leave it.

- [x] **Step 4: Commit**

  ```bash
  git add end2end/tests/posts.spec.ts
  git commit -m "test(e2e): seed :410 home feed via test-support"
  ```

---

### Task 7: Full-matrix validation and documentation

**Files:**

- Modify: `docs/observability.md` (pointer note only)

**Interfaces:** none.

- [x] **Step 1: Run the full local gate**

  Run: `cargo xtask validate` Expected: static + coverage + all four e2e combos
  green (allowing a lone `:349` environmental flake — re-run once before
  trusting a `:349`-only failure). Confirm the three tests pass under
  **postgres** combos too (the tool's only automated dual-backend proof — its
  unit test is sqlite-only).

- [x] **Step 2: Grep all four parked e2e logs for lock errors**

  Use the Grep tool / `rg -i 'sqlite_busy|database is locked' .xtask/run/` on
  the parked logs. Expected: no matches (acceptance criterion — the SQLite
  concurrent-write risk did not materialize).

- [x] **Step 3: Add the observability pointer.** Append to
      `docs/observability.md` a short note that the heavy timeline tests
      (`:305/:349/:410`) now seed via the `test-support` binary rather than
      sequential `create_post` loops, and that the #155 worker-contention
      timeout headroom is a candidate for reduction once workers>1 is unblocked
      (#173) — the before/after measurement is driven by the #152
      `run-e2e-trace-analysis` harness separately (out of scope here). Keep it
      to a few lines; do not re-tune timeouts in this cycle.

- [x] **Step 4: Commit**

  ```bash
  git add docs/observability.md
  git commit -m "docs(observability): note test-support seeding for heavy timeline tests"
  ```

---

## Self-Review

**Spec coverage** (against
`docs/superpowers/specs/2026-07-02-issue-210-test-support-seed.md`):

- New `test-support` crate, not in `jaunder`'s graph → Task 2 (+ Global
  Constraints).
- `seed-posts` through `create_rendered_post`, both backends, collision-free
  slugs → Task 2 (per-user slugs) + Task 7 (postgres combos, lock check).
- Three tests drop the sequential loop, seed mid-test; `:349` two authors →
  Tasks 4–6.
- Built + on e2e VM PATH; prod artifact/module untouched → Task 3 (+
  Constraint).
- Follow-up issue filed → Task 1.
- `:410` materialization gate → resolved (live read confirmed) and encoded as
  Task 6's loud-failure assertion.
- Content-shape per test → `seed_body`/`--body-prefix`, enumerated per test in
  Tasks 4–6.
- #155 headroom / measurement out of scope → Task 7 Step 3 (pointer only).

**Placeholder scan:** every code step carries real code; the two soft spots are
flagged inline with a concrete resolution path, not left as TODO — (a) exact
`Slug`/`Username`/ `ViewerIdentity`/`list_published_by_user` paths: "copy the
forms from `storage/src/test_support.rs` / `posts.rs`"; (b) `DbConnectOptions`
clap parsing: "mirror `server/src/cli.rs`'s `StorageArgs.db` mechanism"; (c)
`coverage reanchor` for the `main` shell.

**Type consistency:** `seed_body`/`seed_slug`/`seed_posts_for_user` and
`seedPostsViaTool(username, count, bodyPrefix, opts)` are named identically
across Tasks 2/4/5/6.
