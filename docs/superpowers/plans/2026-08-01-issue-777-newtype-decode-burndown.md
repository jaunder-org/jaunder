# Plan — #777 slice 1: #693 secret newtype + the `UserRecordParts` over-bite

**Spec:**
[`docs/superpowers/specs/2026-08-01-issue-777-newtype-decode-burndown.md`](../specs/2026-08-01-issue-777-newtype-decode-burndown.md)
**For agentic workers:** drive with **`jaunder-iterate`**; delegate a single
task via **`jaunder-dispatch`** where useful. Tick checkboxes in real time.

---

## Review header

**Goal.** Close #693: type the Postgres bootstrap credential and its same-typed
neighbours, fix the credential-to-stderr leak the audit found, and discharge the
`deferred-newtype` allowlist entry that names #693. Close #777's class-2 residue
by de-tupling `UserRecordParts`.

**Scope — in:** `common/` (4 new newtypes), `storage/src/db.rs`,
`.../postgres/bootstrap.rs`, `.../postgres/teardown.rs`, `.../test_support.rs`,
`.../helpers.rs`, `.../users.rs`, `server/src/{cli,commands,main}.rs`,
`server/tests/{misc,storage}/`, 3 `xtask` allowlist deletions + 1 doc fix.

**Scope — out:** #687, #787, #750, #716, #776, #751 — all already filed.

**Tasks:**

| #   | Task                                                          | Commit |
| --- | ------------------------------------------------------------- | ------ |
| 1   | `PgRolePassword` secret newtype                               | 1      |
| 2   | `PgRoleName` + `PgDatabaseName`                               | 1      |
| 3   | `BootstrapDb` + `AppTarget` — two differently-shaped types    | 1      |
| 4   | Thread through `bootstrap.rs`                                 | 1      |
| 5   | Convert at the `commands.rs` derivation boundary              | 1      |
| 6   | Type the clap args; fix all 6 caller/test sites               | 1      |
| 7   | `expose_url()` + switch **3** consumers **(must precede 10)** | 2      |
| 8   | Redacting `Debug` (both arms)                                 | 2      |
| 9   | `FromStr` error names the scheme                              | 2      |
| 10  | Redacting `Display`                                           | 2      |
| 11  | `PasswordHash` stored-secret newtype                          | 3      |
| 12  | Type `authenticate`'s row; delete its allowlist entry         | 3      |
| 13  | `UserRecordParts` → struct; fix `users.rs:385` + row mapping  | 4      |
| 14  | Delete the 2 remaining entries + fix the gate doc             | 4      |

**Key risks / decisions:**

- **Task 7 is a hard prerequisite for task 10.** Three `Display` consumers feed
  a connect or a persist; redacting first breaks password-bearing
  `JAUNDER_PG_TEST_URL` runs opaquely.
- **Commit 1 must not reference `expose_url()`** (it lands in commit 2).
  Resolved better than planned: task 6's assertions check _parsed values_
  (`options().get_username()`, `role()`, `database()`) rather than a rendered
  URL, so commit 1 has no coupling to commit 2 at all.
- **Task 3's two types are differently shaped, not just distinct.** Only
  `bootstrap_db` is connected with, so only it keeps `PgConnectOptions`;
  `AppTarget` keeps the two identifiers `--app-db` is actually read for. Making
  both wrap `DbConnectOptions` put three fat values in `Commands` and tripped
  `clippy::large_enum_variant`.
- **The scheme check must be explicit.** `PgConnectOptions::from_str` accepts
  `sqlite:` URLs, defaulting the role to the OS user — so dropping
  `DbConnectOptions`'s check silently changes what gets provisioned. Both new
  types call `require_postgres_scheme`, each with a test.
- **Task 13 must not touch `UserRow`** — it is the real `query_as` target and
  keeps its `FlagOrCounter` entries. Touching it would be the
  burn-down-by-construction #777 forbids.
- Allowlist goes 59 → 56. No surviving entry's `category`/`count`/`reason` may
  change.

---

## Global constraints

- **Language:** Rust. Crates: `common`, `storage`, `server`, `xtask`.
- **Gate before each commit:** `cargo xtask check` clean (the pre-commit hook
  runs it in full — **`jaunder-commit`**). Stage, then commit; never
  `git commit -- <paths>`.
- **No `Co-Authored-By` trailer.** **No placeholders** — every task lands
  compiling, tested code.
- **Tests:** `common` and `storage/src/db.rs` use in-file `#[cfg(test)]`. The
  new tests are parse/format/redaction unit tests, not backend-parity tests, so
  the dual-backend template does not apply and `test-backend-pattern` is not
  tripped.
- **Run:**
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-777-newtype-decode-burndown -- <cmd>`.

---

## Commit 1 — #693: type the credential and its neighbours

### Task 1 — `PgRolePassword`

- [x] **Files:** create `common/src/pg_role_password.rs`; register in
      `common/src/lib.rs` (alphabetical — sorts between `password` at `:26` and
      `post_body` at `:27`).

`secret` **without** `sqlx`: this credential is a clap argument consumed once at
bootstrap, never stored in or decoded from a column. (`parse_opts` allows this —
it rejects only bare `sqlx` off a non-secret and `no_sqlx` on a secret.)

```rust
use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A validated, non-empty PostgreSQL role password.
///
/// Adopts the [`StrNewtype`] `secret` surface (ADR-0063 §2): a redacting `Debug` and
/// borrowed `AsRef<str>` access for the `CREATE ROLE … PASSWORD` statement, with no
/// `Display`, serde, `Deref`, owned-`String`, or `PartialEq` — so the credential cannot
/// be rendered, serialised, logged, or value-compared (ADR-0011).
///
/// Unlike [`SmtpPassword`](crate::smtp_password::SmtpPassword) this is **not** a stored
/// secret: it arrives as a clap argument, so it takes no `sqlx` bridge.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret)]
pub struct PgRolePassword(String);

/// Error returned when a Postgres role password fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("PostgreSQL role password must not be empty")]
pub struct InvalidPgRolePassword;

impl FromStr for PgRolePassword {
    type Err = InvalidPgRolePassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidPgRolePassword);
        }
        Ok(PgRolePassword(s.to_owned()))
    }
}
```

- [x] **Test** (in-file, mirroring `smtp_password.rs`'s four): accepts
      non-empty; rejects empty;
      `format!("{p:?}") == "PgRolePassword([redacted])"` and excludes the raw
      value; `p.as_ref()` returns the original.
- [x] **Run:** `cargo nextest run -p common pg_role_password` — FAIL, then PASS.
      **4/4 pass.**

### Task 2 — `PgRoleName` and `PgDatabaseName`

- [x] **Files:** create `common/src/pg_identifier.rs` with both; register in
      `common/src/lib.rs`.

Two **distinct** types — the distinctness is the point. Non-secret, so the full
trailer applies; `PartialEq`/`Eq` are required because the trailer emits `Ord`
and `Ord: Eq` (#761). `Hash` is **not** required (cf. `slug.rs:29`) — do not
derive it without a use.

```rust
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct PgRoleName(String);

#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct PgDatabaseName(String);
```

Each gets a hand-written `FromStr` rejecting empty, with its own error type
(`InvalidPgRoleName` / `InvalidPgDatabaseName`) so the message names which
failed.

- [x] **Test:** per type — accepts non-empty, rejects empty, `Display`
      round-trips.
- [x] **Run:** `cargo nextest run -p common pg_identifier` — FAIL, then PASS.
      **6/6 pass.**

### Task 3 — `BootstrapDb` and `AppTarget`

- [x] **Files:** `server/src/cli.rs` (they are CLI argument types, and both
      parse via `FromStr` so clap validates at argument parsing).

**Two different shapes, because the two arguments need different things.** An
earlier attempt made both wrap `DbConnectOptions`; that put three fat
connection-option values into `Commands` and tripped
`clippy::large_enum_variant`. Keeping only what each argument is _for_ removes
two of the three copies, so the enum genuinely shrinks — no `Box`, no
`#[allow]`, no stringly-typed retreat.

```rust
/// The superuser connection — the only argument we actually connect with.
/// No `Debug`: `PgConnectOptions` carries the bootstrap password.
#[derive(Clone)]
pub struct BootstrapDb(PgConnectOptions);

/// The application role + database. `cmd_create_pg_db` reads exactly these two
/// things from `--app-db` and never connects to it, so nothing else is kept —
/// which also means this cannot carry a password, so `Debug` is safe.
#[derive(Clone, Debug)]
pub struct AppTarget { role: PgRoleName, database: PgDatabaseName }
```

**Both must check the scheme explicitly.** `PgConnectOptions::from_str` accepts
`"sqlite:/tmp/jaunder.db"` — defaulting the role to the OS user and taking
`tmp/jaunder.db` as the database. A shared `require_postgres_scheme` restores
the check `DbConnectOptions::from_str` was doing.

- [x] **Test:** `AppTarget` extracts role + database; each type rejects a
      non-`PostgreSQL` scheme; the `PgBootstrapArgs` parse test asserts on
      parsed values, not a rendered URL.
- [x] **Run:** `cargo nextest run -p jaunder cli`. **All pass.**

### Task 4 — `bootstrap.rs` signature

- [x] **Files:** `storage/src/postgres/bootstrap.rs`.

`create_postgres_database_and_role` (`:40-45`) becomes — note the real return
type is `PgBootstrapError`, **not** `sqlx::Error`:

```rust
pub async fn create_postgres_database_and_role(
    bootstrap: &PgConnectOptions,
    app_role: &PgRoleName,
    app_role_password: &PgRolePassword,
    database_name: &PgDatabaseName,
) -> Result<(), PgBootstrapError>
```

- [x] `PgBootstrapError::RoleExists` (`:57`) and `DatabaseExists` (`:68`) hold
      `String`, so `app_role.to_owned()` now yields a `PgRoleName` — convert
      explicitly at both sites.
- [x] The password is unwrapped via `AsRef<str>` at **exactly one** site: the
      `CREATE ROLE … PASSWORD` statement. Any other use is a compile error (the
      `secret` surface has no `Display`/serde/`Deref`/owned-`String`), so this
      is compiler-enforced.
- [x] Update the two bare `"secret"` literals at `:128` and `:153` (the only two
      in the file).
- [x] **Test:** `compile_fail` doctest — a `PgDatabaseName` passed where a
      `PgRoleName` belongs does not compile.
- [x] **Run:** `cargo nextest run -p storage bootstrap`.

### Task 5 — `commands.rs` collapses

- [x] **Files:** `server/src/commands.rs`.

With the CLI as the parse boundary, `cmd_create_pg_db` has no preamble left:

```rust
pub async fn cmd_create_pg_db(
    bootstrap_db: &BootstrapDb,
    app_db: &AppTarget,
    app_role_password: &PgRolePassword,
) -> anyhow::Result<()>
```

- [x] Delete `require_postgres_options` — with the conversion gone it has no
      callers, and clippy's `dead_code` says so. Its test goes with it.
- [x] Delete the `.parse()?` calls and the `get_username()`/`get_database()`
      derivation: `AppTarget` already holds both, validated.
- [x] **Do not** write a test asserting an empty username is rejected — sqlx
      defaults it, so it is not reachable.
- [x] **Run:** `cargo nextest run -p jaunder`. **All pass.**

### Task 6 — clap args and every broken caller

- [x] **Files:** `server/src/cli.rs`, `server/src/main.rs`,
      `server/src/commands.rs` (tests), `server/tests/misc/commands.rs`,
      `server/tests/misc/postgres/commands.rs`.

`PgBootstrapArgs`: `bootstrap_db` → `BootstrapDb`, `app_db` → `AppTarget`,
`app_role_password` → `PgRolePassword`. Three fields, three distinct types.

Fix every site (this list is the grep, not recall):

- [x] `cli.rs` — `assert_eq!(pg.app_role_password, "secret")` compares against
      the raw secret, which task 1 makes impossible. Use `.as_ref()`. **Do not**
      add `PartialEq` to the secret type. The URL assertions now check parsed
      values (`options().get_username()`, `role()`, `database()`) rather than a
      rendered URL — so commit 1 has **no** coupling to commit 2's redaction.
- [x] `main.rs` — the `PgBootstrapArgs` literal. Its
      `run_create_pg_db_rejects_non_postgres_urls` test is replaced by
      `run_create_pg_db_dispatches`, which passes _valid_ arguments pointed at a
      closed port. **This is required, not optional:** deleting the old test
      left the `Commands::CreatePgDb` dispatch arm uncovered, and the coverage
      gate caught it.
- [x] `commands.rs` — the two in-file rejection tests move to `cli.rs` as
      `app_target_rejects_*`; both states are now unrepresentable here.
- [x] `server/tests/misc/commands.rs` — its rejection test moves for the same
      reason.
- [x] `server/tests/misc/postgres/commands.rs` — two call sites parse into the
      new types.
- [x] **Run:** `cargo nextest run -p common -p storage -p jaunder`. **All pass**
      (6 `atompub_media` Postgres failures are environmental — a bare `nextest`
      run has no PostgreSQL; the Nix gate provisions one).
- [x] **Gate + commit:** `cargo xtask check` **green**, then
      `feat(bootstrap): type the Postgres role password and its neighbours (#693)`.

---

## Commit 2 — #693: close the credential leak

### Task 7 — `expose_url()` + switch all three consumers **(prerequisite for task 10)**

- [x] **Files:** `storage/src/db.rs`, `storage/src/test_support.rs`,
      `storage/src/postgres/teardown.rs`, `server/tests/storage/mod.rs`.

```rust
impl DbConnectOptions {
    /// The full connection URL **including any password**. The single deliberate door past
    /// the redacting `Display`/`Debug`; call it only where the secret is genuinely required
    /// (recording or reopening a connection), never for logging.
    #[must_use]
    pub fn expose_url(&self) -> String { … }
}
```

Switch every consumer that needs a connectable/persistable URL — the spec's A3.4
table:

- [x] `storage/src/test_support.rs:270` —
      `fs::write(PG_URL_FILE, url.to_string())`; read back by `backup.rs:617`
      via `from_str` → connect.
- [x] `server/tests/storage/mod.rs:152` — `PgPool::connect(&url.to_string())`, a
      second direct `Display`→connect path.
- [x] `storage/src/postgres/teardown.rs:68` —
      `db_name_from_url(&options.to_string())`; switch for consistency (the db
      name sits after the credential, so it would survive either way).
- [x] **Test:** round-trip a **password-bearing** `DbConnectOptions` through
      `expose_url()` → `from_str`, asserting the password survives. This is the
      regression guard for the round-trip two drafts of the spec missed.
- [x] **Verify the enumeration:**
      `rg '\.to_string\(\)' storage/src server/tests/storage/` shows no
      remaining `DbConnectOptions` receiver used to connect or persist.
- [x] **Run:** `cargo nextest run -p storage db test_support`.

### Task 8 — redacting `Debug`

- [x] **Files:** `storage/src/db.rs` — remove `Debug` from the derive at `:25`;
      hand-write it.

Both arms carry the password when the URL did (`db.rs:54` parses `options` from
the same string). You cannot redact _inside_ sqlx's `PgConnectOptions: Debug`,
so **do not print it** — emit a summary built from its non-secret accessors:

```rust
impl fmt::Debug for DbConnectOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(o) => f.debug_tuple("Sqlite").field(&o.get_filename()).finish(),
            Self::Postgres { options, .. } => f
                .debug_struct("Postgres")
                .field("url", &"[redacted]")
                .field("host", &options.get_host())
                .field("port", &options.get_port())
                .field("database", &options.get_database())
                .finish(),
        }
    }
}
```

- [x] **Test:** `{:?}` of a password-bearing value contains neither the password
      nor `:hunter2@`, and still names the host and database (so the redaction
      stays useful for diagnosis).

### Task 9 — `FromStr` error names the scheme

- [x] **Files:** `storage/src/db.rs:57-61`.

Replace `format!("unsupported database URL '{s}'; …")` with a message naming
only the offending **scheme**. This is the live leak:
`JAUNDER_DB=postgre://user:secret@host/db` prints the credential to stderr today
via clap's `FromStr` (`cli.rs:35-36`).

- [x] **Test:** parsing `postgre://u:hunter2@h/db` yields an error containing
      neither `hunter2` nor `u:hunter2`, and containing `postgre`.

### Task 10 — redacting `Display` **(after task 7)**

- [x] **Files:** `storage/src/db.rs:40`. Redact the **password component only**.
- [x] **Test:** `{}` of a password-bearing value contains neither the password
      nor `:hunter2@`.
- [x] **Verify unmodified:** the 5 `cli.rs` `to_string()` tests
      (`:478,488,499,513,525`) pass **without edits**, as do task 6's rewritten
      URL assertions. If any needs editing, the redaction is too broad.
- [x] **Gate + commit:** `cargo xtask check`, then
      `fix(db): stop rendering the database password in errors, Debug and Display (#693)`.

---

## Commit 3 — #693: discharge the deferred allowlist entry

### Task 11 — `PasswordHash`

- [x] **Files:** create `common/src/stored_password_hash.rs`; register in
      `common/src/lib.rs`.

Unlike `PgRolePassword` this **is** a stored secret, so it takes the sqlx bridge
— the `SmtpPassword` shape exactly (`common/src/smtp_password.rs:23-25`):

```rust
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, sqlx)]
pub struct PasswordHash(String);
```

with a hand-written `FromStr` rejecting empty and its own error type.

- [x] **Test:** accepts non-empty; rejects empty; `Debug` is redacted; `as_ref`
      round-trips.
- [x] **Run:** `cargo nextest run -p common stored_password_hash`.

### Task 12 — type `authenticate`'s row, delete the entry

- [x] **Files:** `storage/src/users.rs`,
      `xtask/src/steps/sqlx_newtype_decode_check.rs`.

`users.rs::authenticate`'s `query_as` tuple element 6 (`password_hash`) becomes
`PasswordHash`, decoding through the bridge. Delete the `DeferredNewtype` entry
at `:735-745` whose reason names #693.

- [x] **Run:** `cargo nextest run -p storage users`, then `cargo xtask check`.
- [x] **Gate + commit:**
      `feat(users): decode password_hash into a secret newtype (#693)`.

---

## Commit 4 — #777: the `UserRecordParts` over-bite

### Task 13 — named struct + explicit mappings

- [ ] **Files:** `storage/src/helpers.rs`, `storage/src/users.rs`.

Replace the 9-element tuple alias at `helpers.rs:32` with a named struct (**no
`FromRow`** — it is not a decode target). `build_user_record` takes it by name.

- [ ] `helpers.rs:417-428` — the test's positional tuple literal → named-field
      construction.
- [ ] `helpers.rs:200-201` — `user_record_from_row` gains an explicit `UserRow`
      → struct mapping. This is the point of the task: the adjacent-`bool`
      hazard is **concentrated** into one named, reviewable site, not
      eliminated.
- [ ] `users.rs:385` — `build_user_record((…))` with a 9-element positional
      tuple literal. **The site that matters:** it hand-assembles
      `email_verified, is_operator` from an `authenticate` row. Part B is not
      complete without it.
- [ ] **Do not touch `UserRow`** (`helpers.rs:188-198`). It _is_ a genuine
      `query_as` target, stays a policed tuple alias, and keeps its
      `FlagOrCounter` entries (`:706-714`).
- [ ] **Run:** `cargo nextest run -p storage helpers users`.

### Task 14 — delete the two entries, fix the doc

- [ ] **Files:** `xtask/src/steps/sqlx_newtype_decode_check.rs`.

Delete the entries keyed `UserRecordParts.7` (`:719`) and `.8` (`:730`). The
unmatched-entry check in `problems()` (`:1515`, test at `:2203`) fails until
they are gone.

The module doc at `:125` cites `UserRecordParts` as its canonical tuple-alias
over-bite example; task 13 destroys it. Replace with a surviving instance, or
state that the tuple-alias over-bite currently has none. **This is the only
permitted prose change under `xtask/`.**

- [ ] **Verify the non-goals:** `git diff wt-base-issue-777..HEAD -- xtask/`
      shows only the three entry deletions (tasks 12 and 14) plus the `:125` doc
      fix — no surviving entry's `category`/`count`/`reason` edited, no `count`
      increased, and no change to `target_index`, the rule 1/2/3 precedence,
      `POLICED_ROOT`, `BRIDGE_DERIVES`, `BRIDGE_ATTRIBUTES`, or
      `APPROVED_FOREIGN`.
- [ ] **Run:** `cargo xtask validate --no-e2e` — green, allowlist at **57**.
- [ ] **Gate + commit:**
      `refactor(storage): name the UserRecord parts and drop the tuple-alias over-bite (#777)`.

---

## Self-review

- Spec → task mapping: A1→1, A2.1→3, A2.2→2, A2.3→5, A3.1→9, A3.2→8, A3.3→10,
  A3.4→7, A4(Part A)→4/6, A4(Part B)→13, A5.1→11, A5.2→12, B1/B2→13, B3→13
  (explicit guard), B4→14, Non-goals→14.
- **Ordering guards, each stated at its task:** 7 before 10; commit 1 never
  names `expose_url()`.
- **Standalone-compile check per commit:** 1 (no commit-2 symbols), 2
  (`db.rs`/test helpers only), 3 (`common` + `users.rs` + one deletion), 4
  (`helpers.rs`/`users.rs` + two deletions).
- No task depends on unmerged work: #773 merged; branch rebased onto `main`
  (`3d8d9337`).
