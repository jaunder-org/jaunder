# Issue #687 — a closed, validating registry for config keys

**Issue:** [#687](https://github.com/jaunder-org/jaunder/issues/687) — _types:
SiteConfigKey enum — retire the untyped, transposable config get/set_
**Absorbs:** [#777](https://github.com/jaunder-org/jaunder/issues/777) (the
allowlist burn-down) **Milestone:** #13 Domain-value type safety (newtypes)
**Branch:** `worktree-issue-687-site-config-key`

## The goal, stated once

Every config value has a specific type, and is reached through an API that
returns that type. Raw `get`/`set` stop being a public seam anyone can reach
for; they survive only as the primitive the accessors are built from. The set of
supported config keys becomes a thing you can point at, rather than something
that accretes.

## What the issue got wrong

Verified against the tree, not assumed.

1. **The key set is not 11.** `site_config.rs` declares 11 `*_KEY` consts, but
   the namespace also holds `media.max_file_size_bytes` and
   `media.user_quota_bytes` (`storage/src/media.rs:432,434`) and six `smtp.*`
   keys written as bare literals (`smtp.rs:137,141,148,167`;
   `site_config.rs:390,396`). **19 live keys.** A twentieth,
   `media.cache_policy_default` (`media.rs:436`), is declared and never read or
   written anywhere in the workspace.
2. **The allowlist has 7 `not-a-decode-target` entries, not 9**, and all 7 are
   this seam — not "7 of 9"
   (`sqlx_newtype_decode_check.rs:789,800,809,818,880,890,899`).
3. **The three SMTP error tests do not assert messages.** They assert
   `matches!(err, SmtpConfigError::InvalidPort(_))` — variant shape with a
   wildcard (`smtp.rs:305,317,329`). The issue's criterion "still assert the
   **same value-carrying** messages" describes a test that does not exist.
4. **There are 14 defaulted accessors, not three** (10 getters, 4 setters). Only
   `get_smtp_credentials` is required.
5. **One untyped site the issue missed:** `site_config.rs:44` trait
   `delete(&self, key: &str)`. Cited line numbers have drifted
   (`test_support.rs` set at :1440, `cmd_site_config_set` at :703, clap args at
   :391/:395/:404/:419).

## Decisions

### D1 — A `macro_rules!` table generates the key enum and its validator

Rust has no dependent types, so `get(key)` cannot vary its return type by key.
The workable substitute is a **validator** with a fixed return type — it runs
the real parser and discards the value:

```rust
site_config_keys! {
    BackupDestinationPath => "backup.destination_path" : DestinationPath  { optional },
    SiteTitle             => "site.title"              : SiteTitle,
    SiteBaseUrl           => "site.base_url"           : AbsoluteUrl      { optional },
    // ... 19 entries
}
```

The macro emits the `SiteConfigKey` enum (per-variant
`#[strum(serialize = "...")]`, since the dotted form is not snake_case of the
variant name) and
`fn validate(self, raw: &str) -> Result<(), InvalidSiteConfigValue>`.

**Why a table rather than a hand-written enum plus match:** a key physically
cannot exist without a validator, and the whole supported surface is one
scannable block. **Why `macro_rules!` rather than a const table:** a const table
gives no way to _name_ an entry — `SITE_TITLE_KEY` would become an index const
or a parallel list, restoring the two-lists-that-drift problem it was meant to
remove. Enum variants are nameable for free. **Why not a proc macro:**
`macro_rules!` adds no proc-macro surface.

### D1a — The registry lives in `common`, not `storage`, and uses the `sqlx` form

`storage/Cargo.toml` depends on neither `strum` nor `macros`, and has no `sqlx`
feature. `#[text_enum]` injects `::strum::*` and emits its bridge inside
`#[cfg(feature = "sqlx")]` (`macros/src/sqlx_bridge.rs:22,53`) — in `storage` it
would compile the bridge out **silently** and trip `unexpected_cfgs`. `common`
already carries both (precedent: `FeedEventStatus`,
`common/src/feed/event_status.rs:17`), and is where `SmtpUsername`,
`SmtpPassword`, and `Mailbox` already live per ADR-0063.

**The `sqlx` form is required, not optional.** `get`/`set`/`delete` all
`.bind(key)` (`site_config.rs:366,377,419`). Without `Encode` the impls must
write `.bind(key.as_ref())` — exactly the pattern
`xtask/src/steps/sqlx_newtype_bind_check.rs:12-22` polices across `storage/src`.
`#[text_enum(sqlx, ...)]` yields `Encode` so the key binds directly.

### D1b — Optional keys accept the empty string

`set_identity` stores `""` for an absent `base_url` (`site_config.rs:219-220`),
`set_feeds_config` for an absent hub URL (:268-272), and `set_backup_config` for
an absent destination path (:226-230). **Empty means unset** is an existing,
load-bearing contract. Keys marked `{ optional }` in the table have validators
that accept `""`, and `list` must not report those rows as invalid. Without this
rule D4 and A6 would break three shipped behaviours.

### D2 — The registry is all 19 live keys; the dead one is deleted

`media.cache_policy_default` is removed rather than enshrined. `smtp.username`
and `smtp.password` **are** in the table, so the CLI can set and validate them,
even though they are read through a direct SQL bridge rather than `get`.

### D3 — Raw `get`/`set`/`delete` take `SiteConfigKey`; the consts are deleted

This kills the transposition defect: `set(key, value)` with two `&str` becomes
`set(SiteConfigKey, &str)`, and swapping the arguments stops compiling. All 14
config-key consts (11 `site_config.rs`, 3 `media.rs` — the third being the dead
one) are replaced.

### D4 — `list()` stays raw; the CLI judges

`list() -> Vec<(String, String)>` is unchanged — a dump of what is _physically_
stored; typing it would hide the orphan rows an operator needs to see. The CLI
parses each key and runs its validator, reporting **two** classes of junk:

```
$ jaunder site-config list
site.title=My Site
site.base_url=nonsense://x        INVALID (not an absolute URL)
media.cache_policy_default=x      UNKNOWN KEY
```

The second class — a recognised key whose stored value no longer parses — is
newly detectable _because_ every key now has a validator. Not hypothetical:
`get_identity` and `get_feeds_websub_hub_url` already **purge** unparseable
values on read.

### D5 — SMTP values get the full treatment; the `SqlxBridge` derive gains a text-decode option

Into `common`: `SmtpHost` and `SmtpSender` as `StrNewtype`s with value-carrying
errors (`SmtpSender` validates Mailbox-parseability; `Mailbox` is not in the
gate's `APPROVED_FOREIGN`, which holds only `DateTime`, so widening that is not
the route). `SmtpTlsMode` converted from its hand-rolled `Display`/`FromStr` to
`#[text_enum(sqlx, ...)]`.

**`SmtpPort` needs a macro change.** `macros/src/sqlx_bridge_derive.rs:27-38`
derives `type_inner`, `encode_inner` _and_ `decode_inner` from the single field
type, with an infallible `convert`. So `SmtpPort(u16)` produces an
**integer-column** bridge, while `site_config.value` is `TEXT NOT NULL` on both
backends. The parameterised `macros::sqlx_bridge::bridge` that could express
text-in/u16-held is `pub(crate)`.

So the derive gains a text-decode option:

```rust
#[derive(SqlxBridge)]
#[sqlx_bridge(decode_inner = String)]
pub struct SmtpPort(u16);
// decode: TEXT -> String -> parse::<u16>() -> SmtpPort;  encode: -> String
```

This is **not** `SmtpPort`-specific and is why the macro is the right place:
every numeric config value is stored as TEXT — `backup.retention_count`,
`feeds.min_items`, `feeds.min_days`, `media.max_file_size_bytes`,
`media.user_quota_bytes`. A hand-written bridge would instead need its own
allowlist entry, which is the opposite of A14.

Rejected: holding a `String` and exposing `as_u16()` (the field would lie about
its meaning, and the trick repeats per numeric key); `CAST(value AS INTEGER)` in
SQL (SQLite yields `0` for non-numeric text where Postgres errors — a parity
hazard of exactly the kind ADR-0019 exists to prevent).

### D5a — One required SMTP read, not two

`get_smtp_credentials` is **already** a required method doing exactly the
direct-SQL-bridge decode D5 proposes (`site_config.rs:54,384-401`). Adding
`get_smtp_config` alongside would leave two overlapping required decode methods
over the same six keys. They **merge**: `get_smtp_config()` is the single
required method returning all six values, and `get_smtp_credentials` is deleted.
`load_smtp_config` then contains no `SiteConfigStorage::get` call at all.

### D6 — `InMemorySiteConfig` is deleted, not rewritten

The fake is a `Mutex<BTreeMap<String,String>>` that **re-implements the bridge
decode by hand** to mirror the real backend's decode-failure behaviour
(`test_support.rs:1462-1478`). That mirror is why 3 of the 7 allowlist entries
exist, and it means `load_smtp_config_returns_err_for_invalid_port` currently
proves the _fake's_ parsing rejects a bad port — not the real one.

It was never a considered choice: commit `5c463477` created it purely by
de-duplicating two byte-identical doubles. ADR-0033's problem statement names
exactly this gap — storage's own unit tests "hardcode `sqlite::memory:` and run
SQLite-only, leaving Postgres unexercised for backend-common contract behavior
asserted inside `storage` (e.g. `site_config` get/set semantics…)".

Delete it, and move its **12** tests — `storage/src/smtp.rs` ×8,
`storage/src/site_config.rs` ×1, `server/src/mailer/mod.rs` ×3 — onto
`#[apply(backends)]` + `backend.setup()`. **Except:** 2 of the 3 mailer tests
never touch site config (they construct the fake empty), so they take a
`guard:no-backend` marker rather than pay a DB setup for nothing.

One implementation of the trait's primitives means bridge decoding has one home
and drift is structurally impossible.

**In-memory SQLite is not the alternative:** `sqlite::memory:` gives each
_connection_ its own database, so a multi-connection `SqlitePool` over it is not
a coherent shared store; and WAL — which production and the harness both set —
is unavailable for it.

### D7 — The 14 defaulted accessors stay defaulted

The gate's `Scanner` has no `visit_trait_item_fn`, so trait default bodies are
never scanned — which matters for accessors that **decode from SQL**, since
their decodes would be invisible. That is the argument for `get_smtp_config`
being required, and it holds. It does **not** apply to the other 14: the 10
getters call `get()`, which returns an already-decoded `Option<String>` (and
`get` itself _is_ scanned), then `.parse()` in plain Rust; the 4 setters call
`set()`. Nothing for the gate to miss.

Their tolerance of bad values (`unwrap_or_default`, `unwrap_or(Closed)`,
purge-and-return-`None`) is existing intended behaviour and is **not** changed
by this cycle.

### D8 — `user_config` gets the same treatment

The claim that its key space is "genuinely open-ended" does not survive
examination: `site_config` had the same `&str` signature, the same arbitrary
test keys, and a single dominant key too. The real asymmetry is only that user
config has **no CLI door**, so it lacks the free-form surface where a typo
silently writes a garbage row.

```rust
user_config_keys! {
    DefaultPostFormat => "posts.default_format" : PostFormat,
}
```

`USER_MEDIA_CACHE_POLICY_KEY` (`user_config.rs:26`) is referenced nowhere —
deleted. `DEFAULT_POST_FORMAT_KEY` is replaced by the variant.

### D9 — `MockSiteConfigStorage` is untouched

Not because the sites are all bare — 6 of the 16 do set expectations
(`feed/regenerate.rs:192,238`, `feed/handlers.rs:219`,
`feed/worker.rs:524,580,628`). The reason is narrower and sufficient: **none of
the 16 touches `expect_get`, `expect_set`, or `expect_delete`**, so retyping
those three primitives leaves every call site compiling unchanged. This cycle
removes one of the three test doubles, not two.

## Scope

### In

- `common/` — the `site_config_keys!` and `user_config_keys!` tables;
  `SiteConfigKey`, `UserConfigKey`; `SmtpHost`, `SmtpSender`, `SmtpPort`;
  `SmtpTlsMode` moved here and converted to `#[text_enum(sqlx, ...)]`.
  **Also `parse_default_audience`**, today a private fn in the _storage_ crate
  (`site_config.rs:315`). `AudienceTarget` (`common/src/visibility.rs:127`) has a
  `Named(_)` variant and no `FromStr`, so the registry — which lives in `common` — cannot
  reach its parser where it currently sits. The parser moves to `common` beside the type
  it parses. (Not a dependency cycle: `storage → common` is the only edge, and this
  removes a use of it.)
- `macros/src/sqlx_bridge_derive.rs` — the `decode_inner` option (D5).
- `storage/src/site_config.rs` — `get`/`set`/`delete` retyped; 11 consts
  deleted; required `get_smtp_config`; `get_smtp_credentials` deleted.
- `storage/src/media.rs` — 3 consts deleted; `media.cache_policy_default` gone.
- `storage/src/smtp.rs` — `load_smtp_config` stops calling raw `get`; 8 unit
  tests become dual-backend.
- `storage/src/user_config.rs` — trait retyped; dead const deleted.
- `storage/src/test_support.rs` — `InMemorySiteConfig` deleted.
- `storage/src/postgres/migrations.rs:15` — `get("missing")` is no longer
  expressible.
- `server/src/cli.rs` — `key` parses to `SiteConfigKey` at the clap boundary;
  **the doc comment at :379-382 ("free-form key/value store; values are not
  validated", plus a stale key list) becomes false and must be rewritten.**
- `server/src/commands.rs` — `set` validates before writing; `list` reports
  unknown keys and invalid values.
- `server/src/mailer/mod.rs` — 1 test onto the harness, 2 marked
  `guard:no-backend`.
- `xtask/src/steps/sqlx_newtype_decode_check.rs` — delete the 7
  `not-a-decode-target` entries; re-examine the 2 `DeferredNewtype` entries
  naming #687 (a third, `subscriptions.rs:828`, is unrelated and stays).
- Test call sites using bogus keys (~40) rewritten onto real keys; CLI
  unknown-key tests become parse-rejection tests.
- `CONTRIBUTING.md:177` — stale; still says to use `sqlite::memory:`,
  contradicting both the harness and ADR-0033.

### Out — verified rejects

- **`MockSiteConfigStorage`** — see D9.
- **The `guard:no-backend` mechanism** — ~41 markers across 13 files; only ~11
  are ours. The rest cover genuinely non-DB async tests (password hashing,
  `DbConnectOptions` routing, the mock-based tests).
- **`client/src/storage.rs:53,65,77`** — browser `localStorage`. **Not** because
  its keys are open-ended: the whole workspace uses exactly **two**
  (`MARKER_KEY`, via `common::session_user`; `THEME_KEY = "jaunder_theme"`,
  `web/src/app/component.rs:12`). It deserves the same discipline and is filed
  as [#827](https://github.com/jaunder-org/jaunder/issues/827). Out of _this_
  cycle because the seam falls differently: ADR-0069 charters `client` as raw
  browser infrastructure with **no domain types**, so the registry cannot sit
  beside the primitive the way `SiteConfigKey` does — it must live in
  `web`/`common`, with `client::storage` reached only through it. That is a
  design question of its own, not a copy-paste of D1.
- **Converting the 14 defaulted accessors** — see D7.
- **The gate's `visit_trait_item_fn` blind spot** — real, but xtask work in
  another subsystem. Filed separately.

### Known implementation risk

`macro_rules!` expands before attribute macros and the emitted attribute list is
written literally in the macro body, so `#[macros::text_enum(...)]` will still
be the item's first active attribute (ADR-0091's requirement). The residual
unknown is whether a `$lit:literal` metavariable substituted into
`#[strum(serialize = $lit)]` survives `syn`'s parse. **The plan's first
implementation task is a spike proving this**, because the whole registry shape
depends on it; if it fails, the fallback is the macro emitting a plain enum plus
hand-written `strum` derives.

## Decisions to record

Two ADR-shaped conventions, drafted numberless in `docs/adr/drafts/`, numbered
at ship by `cargo xtask adr promote`:

1. **Config keys are a closed registry carrying validators**; raw `get`/`set`
   are legal only inside accessor bodies.
2. **Prefer the real harness over a hand-rolled fake** when the fake would
   mirror backend behaviour.

## Acceptance criteria

- **A1** `rg -n 'const [A-Z_]+_KEY' storage/src` returns exactly **one** hit —
  `TEMPLATE_LOCK_KEY: i64` (`test_support.rs:539`), which is a Postgres
  advisory-lock id, not a config key. All **16** config-key consts are gone — 11 in
  `site_config.rs`, 3 in `media.rs`, 2 in `user_config.rs`. (Baseline today: 17 hits,
  being those 16 plus `TEMPLATE_LOCK_KEY`.)
- **A2** `SiteConfigStorage::{get,set,delete}` and
  `UserConfigStorage::{get,set,delete}` take their key enum, not `&str`.
  `trybuild` is **not** a dependency of this workspace, so the compile-failure
  of a transposed `set(value, key)` is not pinned by a test; it follows from the
  signature, and A2 is satisfied by the signature alone.
- **A3** The `site_config_keys!` table has exactly **19** entries.
  `media.cache_policy_default` appears nowhere outside `docs/archive/`.
- **A4** A test walks `SiteConfigKey::VARIANTS` and, for **every** variant
  without exception, asserts `validate` rejects a known-bad string. The macro
  makes a validator mandatory, so there is no key to exempt.
- **A5** A test walks `SiteConfigKey::VARIANTS` and asserts each `{ optional }`
  key's validator **accepts** `""`, and each non-optional key's **rejects** it —
  pinning D1b's empty-means-unset contract.
- **A6** `jaunder site-config set <unknown-key> v` exits non-zero with an error
  naming the key. Rejection happens at clap parse, so the DB is never opened —
  the test asserts the exit code and message, **not** a row count.
- **A7** `jaunder site-config set site.base_url nonsense` exits non-zero without
  writing a row (this one does reach the DB layer, so the row-count assertion is
  meaningful here).
- **A8** `jaunder site-config set site.base_url ""` **succeeds** and clears the
  value — the regression lock on D1b.
- **A9** `jaunder site-config list` marks unrecognised keys and
  recognised-but-invalid values distinctly, still prints every stored row, and
  does **not** flag an empty optional value.
- **A10** `list()`'s signature is unchanged:
  `sqlx::Result<Vec<(String, String)>>`.
- **A11** `get_smtp_config()` is a **required** trait method;
  `get_smtp_credentials` no longer exists; `load_smtp_config` contains no call
  to `SiteConfigStorage::get`.
- **A12** The `SqlxBridge` derive accepts `decode_inner`, with its own unit test
  in `macros/tests/` covering a text-decoded numeric round-trip and a decode
  rejection.
- **A13** `SmtpHost`, `SmtpSender`, `SmtpPort` exist in `common` with
  value-carrying errors; `SmtpTlsMode` is `#[text_enum(sqlx, ...)]` with no
  hand-written `Display`/`FromStr`.
- **A14** The three SMTP error tests are **strengthened** to assert the offending value
  appears in the error's **message** — the criterion the issue meant — and still pass
  after the read path moves into the sqlx bridges.

  **They must not assert the error's variant.** `SmtpConfigError::{InvalidPort,
  InvalidTlsMode, InvalidSender}` are constructed only inside `load_smtp_config`'s own
  parsing (`smtp.rs:145,152,175`); once that parsing moves into the bridges, a bad value
  surfaces as a `ColumnDecode` whose `source` is the newtype's `FromStr` error, and those
  three variants become unconstructible. That is a deliberate consequence of D5, not a
  regression — **the value echo is the property being protected, not the variant identity.**
  A test written as `matches!(err, InvalidPort(_))` would make D5 unimplementable, which is
  precisely the trap the issue's own wording set. Whether the three variants are deleted or
  retained as a mapped wrapper is an implementation choice; the message assertion holds
  either way.
- **A15** `InMemorySiteConfig` does not exist; `rg -n 'InMemorySiteConfig'`
  returns zero hits outside `docs/archive/`.
- **A16** The 10 tests moved onto the harness are `#[apply(backends)]` and pass
  on **both** backends; the 2 config-free mailer tests carry `guard:no-backend`
  with a reason.
- **A17** All **7** `Category::NotADecodeTarget` entries are **deleted** from
  the allowlist, not recategorised — the gate's staleness check enforces this.
  The 2 `DeferredNewtype` entries naming #687 are deleted or their reason text
  updated to say why they survive.
- **A18** `MockSiteConfigStorage` call sites are unchanged (16 sites).
- **A19** `UserConfigStorage::{get,set,delete}` take `UserConfigKey`;
  `USER_MEDIA_CACHE_POLICY_KEY` and `DEFAULT_POST_FORMAT_KEY` are gone.
- **A20** `CONTRIBUTING.md` no longer instructs the use of `sqlite::memory:`;
  `server/src/cli.rs`'s `site-config` doc comment no longer says values are
  unvalidated.
- **A21** `devtool run -- cargo xtask validate --no-e2e` green — including
  `doc-links`, `coverage`, `test-backend-pattern`, **`sqlx-newtype-bind`**,
  `sqlx-newtype-decode`, and `adr-format`.
- **A22** Full `devtool run -- cargo xtask validate` (with e2e) green before the
  PR opens. Required because e2e seeding drives `jaunder site-config set` for
  three keys (`tools/devtool/src/seed_e2e.rs:38-49`) — the CLI surface this
  cycle retypes. All three keys and their values pass their validators.
