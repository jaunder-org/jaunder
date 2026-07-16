# Plan — `jaunder site-config` CLI (set/get/list) + retire test-support site-config seeding

Issue #8. Spec: `docs/superpowers/specs/2026-07-15-issue-8-site-config-cli.md` (the
"what/why" — decisions D1–D7, acceptance criteria AC1–AC9). This plan is the "how".

## Review header

**Goal.** Ship `jaunder site-config set/get/list` in the production binary, add the one
missing storage primitive (`list`), and convert both e2e seed consumers off
`test-support set-site-config` onto the new command, retiring the dead subcommand.

**Scope.** In: the storage `list` method; the CLI subcommand group + handlers + tests; the
`devtool seed-e2e` and `end2end/seed.ts` migrations + their callers (xtask, flake); the
`test-support` retirement. Out: key validation, `unset`, an ADR (spec non-goals). No
separable follow-up issues surfaced — the whole change is this one issue.

**Tasks.**

1. Storage: add `SiteConfigStorage::list()` primitive (+ generic impl, +2 test doubles, dual-backend test).
2. CLI: `jaunder site-config {set,get,list}` — nested subcommand, handlers, `format_entries`, parse + handler tests.
3. Seed core: `seed_e2e.rs` binary-tagged invocations (site_config first, `Jaunder`) + `run(jaunder_bin)` + `devtool` `--jaunder-bin`.
4. Seed callers: `xtask e2e_local` passes `--jaunder-bin`; `flake.nix` both VMs pass it + add `jaunderBin` to `systemPackages`.
5. `end2end/tests/seed.ts`: `seedConfigViaTool` → `jaunder site-config set`.
6. Retire `test-support set-site-config` (subcommand + lib fn + tests), after 3 & 5.
7. Final gate + PR.

**Key risks / decisions.**

- **Two-binary seed + cheap-kdf fail-close** (spec D6). Mitigated by ordering the `Jaunder`
  steps first (fail fast on an empty DB) and a bail message naming the binary. The seed's
  `jaunder` builds are all cheap-kdf-OFF (verified).
- **Ordering dependency:** Task 6 (retire) must land after 3 and 5, or the `devtool`/`seed.ts`
  callers reference a removed subcommand and the e2e (AC9) breaks.
- **Adding a required trait method** breaks only the two hand-written doubles; `automock`
  auto-handles the mock (spec D5).
- **Coverage:** new handlers/`format_entries`/`list` impl must be covered by the task's own
  unit tests; `cargo xtask check` gate per commit.

**For agentic workers.** Execute with `jaunder-iterate` (delegating a task to a subagent via
`jaunder-dispatch` where useful), ticking checkboxes live. Runs after the plan HALT.

## Global constraints

- Rust 2021, backend parity (ADR-0019): storage `list` is implemented once on
  `SiteConfigStore<DB>`; the dual-backend test uses the `backends`/`Backend` template
  (a bare `#[tokio::test]` on storage trips the `test-backend-pattern` guard).
- The server crate's **package name is `jaunder`** (directory `server/`, binary target
  `jaunder`) — all `nextest`/`-p` commands below use `-p jaunder`. `test-support`, `storage`,
  `server`→`jaunder` are **root** workspace members (`-p <name>`, no `--manifest-path`);
  `devtool` is in the `tools/` workspace (`--manifest-path tools/Cargo.toml -p devtool`);
  `xtask` is workspace-excluded (`--manifest-path xtask/Cargo.toml`).
- Per-commit: run `cargo xtask check` clean first (fmt + clippy + Nix coverage). No
  `Co-Authored-By` trailer. Serialize edit→gate→commit (no edits mid-gate).
- Free-form keys/values; no validation (spec). `list` = human view, `get` = scriptable.

---

## Task 1 — Storage `SiteConfigStorage::list()`

**Files.** `storage/src/site_config.rs` (trait + generic impl + test), `storage/src/smtp.rs`
(`MapConfigStore` double), `server/src/mailer/mod.rs` (`MapConfigStore` double).

**1.1 (RED) Dual-backend test.** In `storage/src/site_config.rs`'s `#[cfg(test)] mod tests`,
add a test using the module's `rstest_reuse` template (`#[apply(backends)]` + `#[case]
backend: Backend`, reading through `env.state.site_config` — **not** a bare `#[tokio::test]`
+ manual loop, which trips the `test-backend-pattern` guard):

```rust
#[apply(backends)]
#[tokio::test]
async fn list_returns_all_entries_ordered_by_key(#[case] backend: Backend) {
    let env = backend.setup().await;
    let storage = &*env.state.site_config;
    storage.set("site.title", "T").await.unwrap();
    storage.set("feeds.websub_hub_url", "https://h/").await.unwrap();
    storage.set("backup.mode", "archive").await.unwrap();

    assert_eq!(
        storage.list().await.unwrap(),
        vec![
            ("backup.mode".to_string(), "archive".to_string()),
            ("feeds.websub_hub_url".to_string(), "https://h/".to_string()),
            ("site.title".to_string(), "T".to_string()),
        ],
        "list() is ordered by key, both backends",
    );
}
```

Cross-check the exact setup shape against the sibling tests at `site_config.rs:304+` before
writing (field/accessor names). Run: `cargo nextest run -p storage list_returns_all_entries`
→ **FAIL** (no `list` method).

**1.2 (GREEN) Trait method.** Add to the `SiteConfigStorage` trait (near `get`/`set`):

```rust
/// Enumerate all `site_config` entries, ordered by key. A third primitive
/// alongside `get`/`set` (no default: a `vec![]` default would silently
/// under-report for any implementor).
async fn list(&self) -> sqlx::Result<Vec<(String, String)>>;
```

Generic impl in `SiteConfigStore<DB>` (mirror `get`'s `query_as` style). **Required
`where`-clause edit:** the impl at `site_config.rs:266-273` currently bounds only
`(String,): for<'r> sqlx::FromRow<'r, DB::Row>`; add
`(String, String): for<'r> sqlx::FromRow<'r, DB::Row>` (the `list` query decodes a
2-tuple). This is a certainty, not a maybe:

```rust
async fn list(&self) -> sqlx::Result<Vec<(String, String)>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT key, value FROM site_config ORDER BY key",
    )
    .fetch_all(&self.pool)
    .await?;
    Ok(rows)
}
```

**1.3 (GREEN) Test doubles.** Add a `list` to each `MapConfigStore`:

```rust
async fn list(&self) -> sqlx::Result<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> =
        self.0.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
    out.sort();
    Ok(out)
}
```

Both doubles are `#[cfg(test)]` `MapConfigStore` with a `.0` map of `&'static str` →
`&'static str` (`storage/src/smtp.rs`, `server/src/mailer/mod.rs`) — hence `(*k).to_string()`
(a `k.clone()` would yield `&str`, not `String`). Read both to confirm the exact key/value
types before editing; not gate-measured.

**Verify.** `cargo nextest run -p storage site_config` → PASS. `cargo xtask check` clean.
Commit: `types(storage): add SiteConfigStorage::list enumeration primitive (#8)`.

**Covers:** AC3 (ordering), AC5 (dual-backend).

---

## Task 2 — CLI `jaunder site-config {set,get,list}`

**Files.** `server/src/cli.rs` (variant + `SiteConfigAction` + parse tests),
`server/src/commands.rs` (dispatch + `SiteConfigAction::execute` + 3 handlers +
`format_entries` + handler tests).

**2.1 (RED) Parse tests.** In `cli.rs` `mod tests`, add (using the existing `parse` helper):

```rust
#[test]
fn site_config_set_parses_positional_key_value() {
    let cli = parse(&["site-config", "set", "feeds.websub_hub_url", "https://h/"]);
    match cli.command {
        Some(Commands::SiteConfig { action: SiteConfigAction::Set { key, value, .. } }) => {
            assert_eq!(key, "feeds.websub_hub_url");
            assert_eq!(value, "https://h/");
        }
        _ => panic!("expected site-config set"),
    }
}

#[test]
fn site_config_set_allows_hyphen_leading_value() {
    let cli = parse(&["site-config", "set", "some.key", "-dashy"]);
    match cli.command {
        Some(Commands::SiteConfig { action: SiteConfigAction::Set { value, .. } }) =>
            assert_eq!(value, "-dashy"),
        _ => panic!("expected site-config set"),
    }
}

#[test]
fn site_config_get_parses_key() { /* -> SiteConfigAction::Get { key, .. } */ }

#[test]
fn site_config_list_parses() { /* -> SiteConfigAction::List { .. } */ }

#[test]
fn site_config_set_missing_value_is_clap_error() {
    assert!(Cli::try_parse_from(["jaunder", "site-config", "set", "only.key"]).is_err());
}

// AC4: --db is accepted on each leaf and does not swallow the positional value
// (the allow_hyphen_values × flattened-StorageArgs interaction).
#[test]
fn site_config_set_accepts_db_flag_after_positionals() {
    let cli = parse(&["site-config", "set", "some.key", "val", "--db", "sqlite:./x.db"]);
    match cli.command {
        Some(Commands::SiteConfig { action: SiteConfigAction::Set { key, value, storage } }) => {
            assert_eq!((key.as_str(), value.as_str()), ("some.key", "val"));
            // storage.db parsed the URL, not "val"
            let _ = storage;
        }
        _ => panic!("expected site-config set"),
    }
}
```

Run: `cargo nextest run -p jaunder site_config` → **FAIL** (no variant).

**2.2 (GREEN) clap types** in `cli.rs`. Add the nested action enum and the `Commands` arm:

```rust
/// Read or write `site_config` key/value entries.
///
/// site_config is a free-form key/value store. Known keys include
/// `site.registration_policy`, `site.title`, `site.base_url`,
/// `feeds.websub_hub_url`, `feeds.min_items`, `feeds.min_days`,
/// `posts.default_audience`, and the `backup.*` keys.
#[derive(Subcommand, Clone)]
pub enum SiteConfigAction {
    /// Set (upsert) a key to a value.
    Set {
        #[command(flatten)]
        storage: StorageArgs,
        /// The site_config key (e.g. `feeds.websub_hub_url`).
        key: String,
        /// The value to store (free-form; leading `-` allowed).
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Print the value for a key (nothing + non-zero exit if unset).
    Get {
        #[command(flatten)]
        storage: StorageArgs,
        /// The site_config key to read.
        key: String,
    },
    /// Print all entries as `key=value`, one per line, ordered by key.
    List {
        #[command(flatten)]
        storage: StorageArgs,
    },
}
```

And in `Commands`:

```rust
/// Read or write `site_config` entries (set/get/list).
///
/// The storage directory must already be initialized via `jaunder init`.
SiteConfig {
    #[command(subcommand)]
    action: SiteConfigAction,
},
```

**2.3 (GREEN) Dispatch + handlers** in `commands.rs`. Add the arm to `Commands::execute`
(keep it a thin delegation — this is jaunder's first nested group; the doc comment is the
copy-paste breadcrumb):

```rust
// First nested subcommand group: the arm stays thin and delegates to
// SiteConfigAction::execute, preserving the low-CRAP one-arm-per-command shape.
Commands::SiteConfig { action } => action.execute().await,
```

```rust
impl SiteConfigAction {
    /// Dispatch a `site-config` leaf to its handler (mirrors `Commands::execute`).
    ///
    /// # Errors
    /// Propagates the selected leaf's failure.
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            SiteConfigAction::Set { storage, key, value } =>
                cmd_site_config_set(&storage, &key, &value).await,
            SiteConfigAction::Get { storage, key } => cmd_site_config_get(&storage, &key).await,
            SiteConfigAction::List { storage } => cmd_site_config_list(&storage).await,
        }
    }
}

/// Upsert a `site_config` key/value through the real storage path.
async fn cmd_site_config_set(storage: &StorageArgs, key: &str, value: &str) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db).await?;
    state.site_config.set(key, value).await?;
    eprintln!("set site_config {key} = {value}");
    Ok(())
}

/// Print the value for `key`; error (→ non-zero exit) if the key is unset.
async fn cmd_site_config_get(storage: &StorageArgs, key: &str) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db).await?;
    match state.site_config.get(key).await? {
        Some(value) => {
            println!("{value}");
            Ok(())
        }
        None => Err(anyhow::anyhow!("no site_config value for key {key:?}")),
    }
}

/// Print all `site_config` entries as `key=value`, ordered by key.
async fn cmd_site_config_list(storage: &StorageArgs) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db).await?;
    let entries = state.site_config.list().await?;
    print!("{}", format_entries(&entries));
    Ok(())
}

/// Render `site_config` entries as `key=value\n` lines (a human/discovery view;
/// `get` is the lossless scriptable accessor). Pure, so it is unit-tested directly.
fn format_entries(entries: &[(String, String)]) -> String {
    entries.iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}
```

Import the trait/`open_existing_database` as already present in the file (both are).

**2.4 (GREEN) Handler + pure tests** in `commands.rs` `mod tests` (model on the
`cmd_user_invite` TempDir-SQLite tests already in this module):

```rust
#[test]
fn format_entries_renders_sorted_key_value_lines() {
    let e = vec![("a.b".to_string(), "1".to_string()), ("c.d".to_string(), "2".to_string())];
    assert_eq!(format_entries(&e), "a.b=1\nc.d=2\n");
    assert_eq!(format_entries(&[]), "");
}

#[tokio::test]
async fn cmd_site_config_set_upserts_and_get_reads_back() {
    let (_dir, storage) = temp_storage_args().await; // mirror the module's TempDir helper
    cmd_site_config_set(&storage, "feeds.websub_hub_url", "https://x/").await.unwrap();
    cmd_site_config_set(&storage, "feeds.websub_hub_url", "https://y/").await.unwrap(); // upsert
    let state = open_existing_database(&storage.db).await.unwrap();
    assert_eq!(state.site_config.get("feeds.websub_hub_url").await.unwrap(), Some("https://y/".into()));
    cmd_site_config_get(&storage, "feeds.websub_hub_url").await.expect("present key ok");
    cmd_site_config_get(&storage, "does.not.exist").await.expect_err("absent key errors");
    cmd_site_config_list(&storage).await.expect("list ok");
}
```

Reuse (or add, matching the module's existing convention) a `temp_storage_args()` helper
that builds a TempDir SQLite `StorageArgs`. It **must create+migrate the DB first** — call
`storage::open_database(&opts).await` before returning (the handlers use
`open_existing_database`, which errors on a missing DB), exactly as the existing
`cmd_user_invite` test (`commands.rs:815+`) does. Return the `TempDir` so it outlives the
test (dropping it unlinks the SQLite file).

**Verify.** `cargo nextest run -p jaunder site_config` and the `cli` parse tests → PASS.
`cargo xtask check` clean. Commit:
`feat(cli): add jaunder site-config set/get/list (#8)`.

**Covers:** AC1, AC2, AC3 (format), AC4.

---

## Task 3 — Seed core migration (`devtool`)

**Files.** `tools/devtool/src/seed_e2e.rs`, `tools/devtool/src/main.rs`.

**3.1 (RED) Update `canonical_fixture_invocations`.** Rewrite the test to expect: the two
`site_config` steps **first**, tagged `Jaunder`, shaped `["site-config","set",<key>,<value>]`;
then the three `create-user` and `reset-mail` steps, tagged `TestSupport`. Encode the tag in
the asserted structure. Run `cargo nextest run --manifest-path tools/Cargo.toml -p devtool
canonical_fixture` → **FAIL**.

**3.2 (GREEN) Binary-tagged invocations.** In `seed_e2e.rs`:

```rust
/// Which fixture binary a seed step runs against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SeedBin { TestSupport, Jaunder }

fn seed_invocations() -> Vec<(SeedBin, Vec<String>, bool)> {
    let ts = |xs: &[&str]| (SeedBin::TestSupport, xs.iter().map(|x| (*x).to_owned()).collect(), true);
    let jaunder = |xs: &[&str]| (SeedBin::Jaunder, xs.iter().map(|x| (*x).to_owned()).collect(), true);
    vec![
        // site_config first: a wrong --jaunder-bin (cheap-kdf) then fails fast on an empty DB.
        jaunder(&["site-config", "set", "site.registration_policy", "open"]),
        jaunder(&["site-config", "set", "feeds.websub_hub_url", "https://hub.test.local/"]),
        ts(&["create-user", "--username", "testlogin", "--password", "testpassword123"]),
        ts(&["create-user", "--username", "testnoemail", "--password", "testpassword123"]),
        ts(&["create-user", "--username", "testoperator", "--password", "testpassword123", "--operator"]),
        ts(&["reset-mail"]),
    ]
}

pub fn run(db: &str, test_support_bin: &Path, jaunder_bin: &Path) -> anyhow::Result<()> {
    for (bin, args, _fatal) in seed_invocations() {
        let path = match bin { SeedBin::TestSupport => test_support_bin, SeedBin::Jaunder => jaunder_bin };
        let status = Command::new(path)
            .args(&args)
            .env("JAUNDER_DB", db)
            .status()
            .with_context(|| format!("spawning {} {}", path.display(), args[0]))?;
        if !status.success() {
            bail!("{} {} failed ({status})", path.display(), args[0]);
        }
    }
    Ok(())
}
```

Update the module doc comment (it currently says "shells out to the `test-support`
binary") to note site_config now goes to `jaunder`.

**3.3 (GREEN) `devtool` arg + dispatch.** In `main.rs`, add to `SeedE2eArgs`:

```rust
/// Path to the real `jaunder` binary (site_config steps run through it). Bare
/// `jaunder` on the VM (systemPackages), absolute `target/debug/jaunder` on host.
#[arg(long)]
jaunder_bin: std::path::PathBuf,
```

Dispatch: `Command::SeedE2e(args) => seed_e2e::run(&args.db, &args.test_support_bin, &args.jaunder_bin),`.

**Verify.** `cargo nextest run --manifest-path tools/Cargo.toml -p devtool` → PASS.
`cargo xtask check` clean. **Do not commit Task 3 alone** — `--jaunder-bin` is a *required*
arg and xtask source-runs `devtool` in lockstep, so the tree is not e2e-runnable until Task
4 wires the callers. **Land Tasks 3 + 4 as a single commit** (`cargo xtask check` stays green
at each — it skips e2e — but no lone commit 3 is e2e-valid).

**Covers:** AC6.

---

## Task 4 — Seed callers (xtask + flake)

**Files.** `xtask/src/steps/e2e_local.rs`, `flake.nix`.

**4.1 xtask.** In the `seed-e2e` invocation (~line 151), add
`--jaunder-bin {root}/target/debug/jaunder` (a `let jaunder = format!("{root}/target/debug/jaunder");`
plus the arg). It is the same path already spawned for `serve` (line 102). Verify:
`cargo build --manifest-path xtask/Cargo.toml` and `cargo nextest run --manifest-path
xtask/Cargo.toml` (xtask is workspace-excluded — use its manifest).

**4.2 flake.nix.** In **both** `seed_db()` blocks, append `--jaunder-bin jaunder` to the
`devtool seed-e2e` command string. Add `jaunderBin` to **both** VMs'
`environment.systemPackages` (alongside `testSupportBin`/`devtoolBin`) so bare `jaunder`
resolves for both the seed and the `seed.ts` helper (Task 5). Verify:
`nix flake check --no-build` / `nix eval` parses (full VM proof is the e2e matrix, AC9;
dirty the tree per the flake-eval-in-worktree gotcha before eval).

**Verify.** `cargo xtask check` clean (Rust side). **Single commit for Tasks 3 + 4:**
`refactor(devtool,e2e): route seed-e2e site_config through the jaunder binary (#8)`.

**Covers:** the wiring half of AC6/AC9.

---

## Task 5 — `end2end/tests/seed.ts`

**File.** `end2end/tests/seed.ts`.

Change `seedConfigViaTool` to the new command (bare `jaunder`, PATH-resolved in VM via Task
4's `systemPackages` and on host via the `target/debug`-prefixed PATH):

```ts
export function seedConfigViaTool(key: string, value: string): void {
  execFileSync("jaunder", ["site-config", "set", key, value], {
    stdio: "pipe",
    env: process.env,
  });
}
```

Update its doc comment (name `jaunder site-config set` instead of `test-support
set-site-config`). The 5 `invite.spec.ts` call-sites are unchanged. `seedPostsViaTool`
stays on `test-support`.

**Verify.** `npx tsc --noEmit` in `end2end/` (`package.json` has no lint script; a
`tsconfig.json` is present). Functional proof is the e2e matrix (AC9). Commit:
`test(e2e): seedConfigViaTool via jaunder site-config set (#8)`.

**Covers:** AC7.

---

## Task 6 — Retire `test-support set-site-config` (after 3 & 5)

**Files.** `test-support/src/main.rs`, `test-support/src/lib.rs`.

- `main.rs`: remove the `SetSiteConfig` variant, its `Commands::SetSiteConfig` dispatch arm,
  `cmd_set_site_config`, the unused `set_site_config` import, and the `SetSiteConfig` block of
  `run_dispatches_db_commands_against_a_temp_db` (keep the create-user/seed-posts legs).
- `lib.rs`: remove `pub async fn set_site_config` and the `set_site_config_tests` module.

**Verify.** `rg 'set-site-config'` and `rg 'set_site_config'` → no hits outside
`docs/archive/`, this spec, and this plan. `cargo nextest run -p test-support` (a **root**
workspace member — no `--manifest-path`) → PASS. `cargo xtask check` clean (proves nothing
else referenced the removed items). Commit:
`chore(test-support): remove now-unused set-site-config subcommand (#8)`.

**Covers:** AC8.

---

## Task 7 — Gate + ship

- `cargo xtask validate --no-e2e` green (AC9 static/clippy/coverage half). Run foreground
  with `timeout: 600000` (coverage rebuild).
- Full e2e matrix is CI's job (ADR-0034); locally optional `cargo xtask validate` or
  `cargo xtask e2e sqlite chromium` smoke to prove the migrated seed runs.
- Ship via `jaunder-ship`: final conformance review + cold blind review, archive spec/plan,
  push, open PR referencing #8. **HALT before merge — no merge without explicit approval.**
