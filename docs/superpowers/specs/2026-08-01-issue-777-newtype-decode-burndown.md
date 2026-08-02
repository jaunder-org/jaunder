# Spec — #777 burn-down, slice 1: #693 secret newtype + the `UserRecordParts` over-bite

**Issue:** #777 (umbrella) — this slice closes **#693** and the class-2 residue
of #777. **Branch:** `worktree-issue-777-newtype-decode-burndown`, based on
`main`. **Base:** `3d8d9337` — tagged `wt-base-issue-777`.

## Why this slice is small

#777 asks for a burn-down of the `sqlx-newtype-decode` allowlist. Its premise —
`category: Category::` and ~45 entries — did not exist on `main` when this cycle
opened; the categorised allowlist lived in then-unmerged PR #773 (#728). **#773
has since merged**, so this branch forks from `main` directly and the allowlist
is live at **59** entries in 9 categories. Each commit here is independently
rebasable so it can be split into its own PR (see _Delivery_).

Scope was narrowed after measurement. #777's class-2 analysis proposed either
ascribing 7 call sites or narrowing the gate's `fn`-return rule; investigation
found a third, better answer for most of them (a domain fix), now **folded into
#687**. What remains in #777's own territory is the `UserRecordParts` pair.

## Out of scope, and where it went

| Concern                                                              | Disposition                                                                                                                                                                                           |
| -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Site-config **value** typing (7 of 9 class-2 entries)                | Folded into **#687**, which now owns both halves of that seam                                                                                                                                         |
| Scanner never visits trait default bodies                            | Filed as **#787** (Bug)                                                                                                                                                                               |
| `SubscriberRef` newtype                                              | **#750**, unchanged                                                                                                                                                                                   |
| `.bind(` direction / gate module split / adjacent same-typed columns | **#716** / **#776** / **#751**, unchanged                                                                                                                                                             |
| Narrowing the gate's `fn`-return rule                                | **Not done.** Once #687 lands, the 7 method-call over-bites disappear at the source; narrowing the rule would shrink the population — the silent direction — to fix what the domain fix already fixes |

---

## Part A — #693: the bootstrap credential

### A1. The secret newtype

`create_postgres_database_and_role(app_role: &str, app_role_password: &str, database_name: &str)`
(`storage/src/postgres/bootstrap.rs:40`) is three adjacent same-typed `&str`
with a credential in the middle. Any permutation compiles.

- **A1.1** `PgRolePassword` — a `#[str_newtype(secret)]` type in
  **`common/src/pg_role_password.rs`**, modelled on `common/src/password.rs` and
  `common/src/smtp_password.rs`: redacting `Debug`, `AsRef<str>` only, no
  `Display`, no serde, no `Deref`, no owned-`String` extraction.
- **A1.2** Construction is a **hand-written `FromStr`** with a non-empty
  invariant and a value-free error, exactly as both cited models do
  (`password.rs:32`, `smtp_password.rs:32`). `StrNewtype` deliberately leaves
  `FromStr` to the author (`macros/src/str_newtype.rs:2`). `FromStr` — not
  `TryFrom<String>` alone — because clap's derive value parser requires it.
- **A1.3** It is threaded `cli.rs` → `commands.rs` → `bootstrap.rs`, so the
  credential carries that type from the clap boundary to the point of use, and
  is unwrapped via `AsRef<str>` at exactly one site (the
  `CREATE ROLE … PASSWORD` statement).

**Acceptance (observable):**

- `storage::create_postgres_database_and_role`'s password parameter has type
  `&PgRolePassword`, and `server/src/cli.rs`'s `app_role_password` field has
  type `PgRolePassword`. (Stated as _the signature is this_, not as _a spelling
  is absent_ — a grep for a missing spelling passes on a rename alone.)
- `PgRolePassword` appears in `bootstrap.rs` only as the parameter and at the
  `CREATE ROLE … PASSWORD` statement. The `secret` surface is what makes this
  enforceable rather than a convention: with no `Display`, serde, `Deref`, or
  owned-`String`, _any_ other use is a compile error, so this criterion is
  checked by the compiler and confirmed by reading one function.
- A test asserts `format!("{:?}", pw)` contains neither the secret nor
  `hunter2`.

### A2. Type the neighbours

Typing only the password leaves two adjacent bare `&str` — the hazard moves
rather than closes, and swapping a role name with a database name is a silent
bootstrap corruption.

**The four neighbours are two different kinds, which the issue's flat list
obscures:**

- `bootstrap_db` and `app_db` (`cli.rs:45,49`) are `String` **PostgreSQL URLs**,
  parsed to `DbConnectOptions` at `commands.rs:177-178`.
- `app_role` and `database_name` are **not** clap arguments at all. They are
  _derived_ inside `cmd_create_pg_db` from `app_options.get_username()` and
  `app_options.get_database()` (`commands.rs:179-183`), both sqlx `&str`
  accessors.

So:

- **A2.1** `bootstrap_db` and `app_db` become **two differently-shaped types**,
  because they need different things:
  - `BootstrapDb(PgConnectOptions)` — the superuser connection. This is the only
    argument whose connection options the command actually uses, because it is
    the only one we connect with.
  - `AppTarget { role: PgRoleName, database: PgDatabaseName }` — the application
    role and database. `cmd_create_pg_db` reads exactly two things from the
    `--app-db` URL (`get_username()`, `get_database()`) and **never connects to
    it**, so nothing else is kept.

  Both live in `server/src/cli.rs` and parse via `FromStr`, so clap validates at
  argument parsing.

  **Why not two `DbConnectOptions`-shaped types.** A first draft made both
  `BootstrapDbUrl`/`AppDbUrl` over `DbConnectOptions`. That put three fat
  connection-option values into `Commands` (`StorageArgs` already contributes
  one), taking the `CreatePgDb` variant to 744 bytes against a 457-byte
  runner-up and tripping `clippy::large_enum_variant`. Neither `Box` nor storing
  the raw URL string is the right answer — the first hides the size, the second
  discards the parse. Keeping only what each argument is _for_ removes two of
  the three copies outright, so the enum genuinely shrinks.

- **A2.2** `PgRoleName` and `PgDatabaseName` — two distinct `StrNewtype`s in
  `common/`, each with a hand-written `FromStr` enforcing non-empty.
- **A2.3** **The scheme check is load-bearing and must be explicit.** sqlx does
  _not_ validate it: `PgConnectOptions::from_str("sqlite:/tmp/jaunder.db")`
  **succeeds**, defaulting the role to the OS user and taking `tmp/jaunder.db`
  as the database — so a mistyped scheme would provision a role named after
  whoever ran the command. `DbConnectOptions::from_str` performs this check, and
  anything parsing straight to `PgConnectOptions` has to repeat it. Both
  `BootstrapDb` and `AppTarget` do, via a shared `require_postgres_scheme`.

  Note also that `get_database()` genuinely returns `Option`, so its absence is
  a real error path, while `get_username()` is defaulted by sqlx and is never
  reachably empty — so do not write a test asserting an empty username is
  rejected.

**The bootstrap path, enumerated** (so "no adjacent same-typed parameters" is
checkable rather than a judgement call):

| Site                                                  | Today                                                    | After                                               |
| ----------------------------------------------------- | -------------------------------------------------------- | --------------------------------------------------- |
| `cli.rs` `PgBootstrapArgs`                            | 3 adjacent `String`                                      | `BootstrapDb`, `AppTarget`, `PgRolePassword`        |
| `commands.rs` `cmd_create_pg_db`                      | 3 adjacent `&str`                                        | `&BootstrapDb`, `&AppTarget`, `&PgRolePassword`     |
| `bootstrap.rs:40` `create_postgres_database_and_role` | 3 adjacent `&str` (after `bootstrap: &PgConnectOptions`) | `&PgRoleName`, `&PgRolePassword`, `&PgDatabaseName` |

**Acceptance:** each of those three signatures has no two adjacent parameters of
the same type. A `compile_fail` doctest shows
`create_postgres_database_and_role` rejects a `PgDatabaseName` passed where a
`PgRoleName` is expected. Tests assert `BootstrapDb` and `AppTarget` each reject
a non-`PostgreSQL` scheme — the behaviour sqlx does not provide.

**A2.4 — `require_postgres_options` is deleted.** Its only purpose was
converting a `DbConnectOptions` into a `PgConnectOptions` and reporting "must be
a PostgreSQL URL" one layer after parsing. With the CLI as the parse boundary it
has no callers, and its test plus the two `cmd_create_pg_db` rejection tests
move to `cli.rs` as parse-boundary tests.

### A3. Log hygiene — verified reachable, so fixed here

#693 says "confirm the value cannot reach a log". It can. Three paths in
`storage/src/db.rs`:

1. **`FromStr` echoes the whole URL** (`db.rs:57-61`). `StorageArgs.db` is
   `#[arg(long, env = "JAUNDER_DB")] pub db: DbConnectOptions` (`cli.rs:35-36`),
   so clap parses via `FromStr` and `JAUNDER_DB=postgre://user:secret@host/db` —
   a one-character scheme typo — prints the credential to stderr. **Live and
   reachable today, no code change needed.**
2. **`Display` writes the raw URL** (`db.rs:40`), credential included.
3. **`Debug` is derived** (`db.rs:25`). ADR-0011 forbids secrets in telemetry;
   today that is discipline alone. The derived `Debug` leaks by **two** routes —
   the `url: String` field and the `options: PgConnectOptions` field. Both carry
   a password only when the **URL** carried one (`db.rs:54` builds `options` by
   parsing the same string).

   `resolved_postgres_options` (`storage/src/postgres/mod.rs:263-270`) does
   **not** contribute: it takes `&PgConnectOptions` and returns a _clone_ with
   the `JAUNDER_DB_PASSWORD`/`_FILE` password applied, never writing back into
   `DbConnectOptions::Postgres { options }`. An earlier draft asserted it did;
   the redacting `Debug` is still required for both arms, but that was not the
   reason.

Requirements:

- **A3.1** The `FromStr` error names the offending **scheme**, never the URL.
- **A3.2** `DbConnectOptions` gets a hand-written redacting `Debug` covering
  both the `url` and `options` arms.
- **A3.3** `Display` redacts the password component only — **and this requires
  A3.4 first.**
- **A3.4** `DbConnectOptions::expose_url()` — an explicit, greppable accessor
  returning the unredacted URL — is added, and **every** `Display` consumer that
  needs a connectable URL is switched to it.

**A3.4 is not optional, and the enumeration below corrects two successive errors
in this spec's drafts.**

There _is_ a full round-trip through `Display`. The complete consumer list:

| Site                                    | What it does                                                                                                                | Action                                                               |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `storage/src/test_support.rs:270`       | `fs::write(PG_URL_FILE, url.to_string())`; `backup.rs:617` reads it back via `recorded_postgres_url` → `from_str` → connect | **switch to `expose_url()`**                                         |
| `server/tests/storage/mod.rs:152`       | `PgPool::connect(&url.to_string())` — a second, direct `Display`→connect path                                               | **switch to `expose_url()`**                                         |
| `storage/src/postgres/teardown.rs:68`   | `db_name_from_url(&options.to_string())` — extracts the database name, which sits _after_ the credential                    | switch for consistency; a password-only redaction would not break it |
| `server/src/cli.rs:478,488,499,513,525` | assertions                                                                                                                  | leave — see A3.3 acceptance                                          |

**Draft 1** claimed "nothing round-trips" — it asked who _read_ `pg_test_url`
and never asked who _wrote_ it. **Draft 2** found the writer but audited only
`test_support.rs` and stopped, missing the `server/tests/` path — the same
failure one level out. Both are recorded because the exhaustiveness of this
table is exactly what makes A3.3 safe.

This does not fail today only because the default test URL is passwordless
(`test_support.rs:351`). That is luck: `postgres_url_authority`
(`test_support.rs:354-359`) explicitly strips credentials from that URL, direct
evidence the codebase expects `JAUNDER_PG_TEST_URL` to be able to carry them.

**Acceptance for the enumeration itself:** after A3.4, `rg '\.to_string\(\)' `
over `storage/src` and `server/tests/storage/` returns no `DbConnectOptions`
receiver that is then used to connect or persist. This is the check that the
list is complete rather than merely long.

**Acceptance:**

- A test parses `postgre://u:hunter2@h/db` and asserts the error contains
  neither `hunter2` nor `u:hunter2`, and does contain `postgre`.
- Tests assert `{}` and `{:?}` of a password-bearing `DbConnectOptions` contain
  neither the password nor `:hunter2@`, for both the `url` and `options` arms.
- A test round-trips a **password-bearing** `DbConnectOptions` through
  `expose_url()` → `from_str` and asserts the password survives — the regression
  guard for the bug this correction found.
- The 5 existing `cli.rs` `to_string()` tests (`:478,488,499,513,525`) pass
  **unmodified**: 4 are sqlite, and the postgres one (asserted at `:499`,
  fixture parsed at `:494`) is passwordless, so a password-only redaction leaves
  all 5 unchanged. If any needs editing, the redaction is too broad.

### A4. Callers and tests this breaks

Enumerated so none is discovered mid-implementation. A first draft listed only
the first four and missed six; the list below was derived by grepping every call
site rather than by recall.

**Part A (the newtype threading):**

- `server/src/cli.rs:319-321` — asserts `pg.bootstrap_db`/`pg.app_db` as
  `String` and `pg.app_role_password == "secret"`. The last compares against the
  raw secret, which A1 deliberately prevents. **Decision:** compare via
  `.as_ref()`; do **not** add `PartialEq` to the secret type, which would
  re-open the extraction door A1 closes. The URL assertions compare via
  `Display`/the parsed options — **not** `expose_url()`, which does not exist
  until the second commit (see _Delivery_).
- `storage/src/postgres/bootstrap.rs:128,153` — bare `"secret"` literals; become
  `"secret".parse::<PgRolePassword>().unwrap()`. These are the only two in that
  file.
- `server/src/commands.rs:824-828,839-843` — two in-file `#[tokio::test]`s
  calling `cmd_create_pg_db` with three `&str` literals.
- `server/src/main.rs:346-350` — a `PgBootstrapArgs { … }` struct literal with
  three `String`s.
- `server/tests/misc/commands.rs:106` — integration call site.
- `server/tests/misc/postgres/commands.rs:32,102` — two integration call sites.

Also note `bootstrap.rs` returns `Result<(), PgBootstrapError>` (`:45`), **not**
`Result<(), sqlx::Error>`, and `PgBootstrapError::RoleExists`/`DatabaseExists`
(`:57`, `:68`) hold `String` — so `app_role.to_owned()` now yields a
`PgRoleName` and needs an explicit conversion at those two sites.

**Part B (the de-tupling):**

- `storage/src/users.rs:385` — `build_user_record((…))` with a 9-element
  positional tuple literal. **This is the site that matters:** it hand-assembles
  `email_verified, is_operator` from an `authenticate` row, so it is where the
  adjacent-`bool` swap is most live. Part B is not complete without it.
- `storage/src/helpers.rs:417-428` — the test's positional tuple literal;
  becomes named-field construction.
- `storage/src/helpers.rs:200-201` — `user_record_from_row` passes a `UserRow`
  tuple straight in; becomes an explicit positional→named mapping. See B2.

### A5. The `deferred-newtype` entry that names #693

`sqlx_newtype_decode_check.rs:735-745` is a `DeferredNewtype` entry for
`users.rs::authenticate` whose reason reads: _"the password_hash column decodes
as String; every other element is typed. Hashes are a secret-bearing value that
wants its own newtype — deferred to #693, which owns the secret-newtype
vertical."_

#777's acceptance requires every `deferred-newtype` entry to have its owning
issue closed **and the entry deleted**, or the issue explicitly re-scoped with
the reason recorded. So #693 cannot close while this stands, and the earlier
drafts of this spec missed it entirely.

- **A5.1** `PasswordHash` — a `#[str_newtype(secret, sqlx)]` type in `common/`.
  Unlike `PgRolePassword` this **is** a stored secret, so it takes the sqlx
  bridge, exactly like `SmtpPassword` (`common/src/smtp_password.rs:23-25`) —
  the `users` row then decodes `password_hash` straight into it.
- **A5.2** `users.rs::authenticate`'s `query_as` tuple element 6 becomes
  `PasswordHash`, and the allowlist entry at `:735-745` is **deleted**.

This is a scope increase over #693 as written, adopted because the allowlist
entry names #693 as its owner and #777's acceptance makes closing #693
conditional on discharging it. The alternative — re-pointing the reason at a
successor issue — would edit a surviving entry's `reason`, which the Non-goals
forbid.

**Acceptance:** the allowlist drops 59 → **56** (this entry plus B1's two).
`authenticate`'s decode target contains no bare `String`.

---

## Part B — `UserRecordParts`

`storage/src/helpers.rs:32` declares `UserRecordParts` as a 9-element tuple
**type alias**, used solely as `build_user_record`'s destructured parameter.
Elements `.7` and `.8` are `email_verified` and `is_operator` — **adjacent
`bool`s**. Swapping them compiles silently.

**B1** `UserRecordParts` becomes a **named struct** with named fields and no
`FromRow` derive. `build_user_record` takes it by name rather than by position.

**The motivation is the transposition hazard, not the entry count.** Two
allowlist entries fall out as a _consequence_; that is not the reason. Stating
it the other way round would be the burn-down-by-construction that #777 forbids.

**B2** `user_record_from_row` (`helpers.rs:200`) gains an explicit `UserRow` →
`UserRecordParts` mapping. This is honest about what B1 achieves: the hazard is
not eliminated, it is **concentrated** into one named, reviewable site where a
swapped pair is visible as `email_verified: row.8` rather than hidden in a
positional contract.

**B3** `UserRow` (`helpers.rs:188-198`) — the structurally identical twin —
**stays a tuple alias**, deliberately. It _is_ a genuine `query_as` target, so
it is legitimately policed, and its `.7`/`.8` keep their `FlagOrCounter`
entries. Converting it would remove real coverage.

### Why this does not silently shrink the gate's population

The gate polices **every** tuple alias unconditionally (`visit_item_type`,
`sqlx_newtype_decode_check.rs:1449-1451`) but polices a struct **only if** it
derives `FromRow` (`visit_item_struct`, `:1421-1425`). A plain non-`FromRow`
struct's fields are policed by nothing — a shape the gate's own module doc names
as the #728 defect (`:108-111`, on `FeedEventRecord` and `ColumnInfo`).

So B1 must be argued, not asserted. The argument: **no decode coverage is lost,
because `UserRecordParts` was never the decode target.** `UserRow` is, it stays
a policed tuple alias (B3), and its `.7`/`.8` entries continue to police the
actual `query_as`. `UserRecordParts` was only ever a positional shadow of
`UserRow` on the far side of a function boundary.

The general hazard the review raises — _what stops the next person deleting an
entry by de-`FromRow`-ing a struct?_ — is real and **not solved here**. It
belongs with #787, which already owns the gate's population boundary; this spec
records it as a known gap rather than pretending B1 closes it.

**B4** The gate's module doc names `UserRecordParts` at
`sqlx_newtype_decode_check.rs:125` as its canonical over-bite illustration. B1
destroys that example, so the doc must cite a surviving one or state that the
tuple-alias over-bite currently has no instance. This is the only permitted
`xtask/` prose change (see Non-goals).

**Acceptance:** the two `helpers.rs` `UserRecordParts.7`/`.8` entries (`:719`,
`:730`) are **deleted** — the gate's unmatched-entry check in `problems()`
(`:1515`, test at `:2203`) fails until they are. Combined with A5.2's deletion
the allowlist reaches **56**. `UserRow`'s two `FlagOrCounter` entries
(`:706-714`) are **unchanged**. `cargo xtask validate --no-e2e` is green.

---

## Non-goals — the burn-down must not cheat

Per #777: a lower count obtained by loosening the rule, widening a category, or
collapsing per-site entries into multiplicities is a regression wearing a green
tick.

**Mechanically checkable:**

- No surviving allowlist entry has its `category`, `count`, or `reason` edited.
  The only entry changes are **three deletions**: A5.2's
  `users.rs::authenticate` entry and B1's two `UserRecordParts` entries. 59
  → 56.
- No change to `target_index`, the rule 1/2/3 precedence, `POLICED_ROOT`,
  `BRIDGE_DERIVES`, `BRIDGE_ATTRIBUTES`, or `APPROVED_FOREIGN`.
- No entry `count` field is increased.

**Narrative, reviewed by a human:** the only prose change under `xtask/` is B4's
replacement of the over-bite example. (The first draft said the diff shows "only
entry deletions and their doc consequences"; "doc consequences" was an unbounded
escape hatch, since B1 _forces_ a module-doc rewrite. Splitting the mechanical
claim from the narrative one makes the falsifiable half actually falsifiable.)

---

## Delivery

Commits are one-per-concern and independently rebasable onto `main`, so each can
be split into its own PR:

1. **#693** — A1 + A2 + A4's Part-A caller fixes (newtype threading).
2. **#693** — A3 (`db.rs` log hygiene, including `expose_url` and the three
   consumer switches). Separate because it is a distinct defect from the
   transposition hazard and carries the security review.
3. **#693** — A5 (`PasswordHash` + its allowlist deletion).
4. **#777** — B1–B4 (`UserRecordParts`) + A4's Part-B caller fixes.

Each commit compiles, passes its tests, and passes `cargo xtask check` **on its
own**. The constraint that makes this real: **commit 1 may not reference
`expose_url()`**, which does not exist until commit 2 — so `cli.rs`'s URL
assertions use `Display`/the parsed options. Those assertions survive commit 2's
redaction because the `cli.rs` fixture URLs (`:310`, `:312`) are passwordless.

#693 is security-adjacent and takes a full branch review before merge regardless
of diff size.

## Decisions recorded

No ADR is required. A1–A2 apply ADR-0063 §1 (transposition hazard, trust
boundary, secrecy); A3 applies ADR-0011 (no secrets in telemetry). Neither is
novel. The one genuinely novel decision from the interview — _raw `get`/`set`
are the untyped seam and every function a storage trait exposes returns a parsed
value_ — belongs to **#687** and is recorded there.
