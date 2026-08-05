# A closed, validating registry for config keys — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`../specs/2026-08-05-issue-687-site-config-key.md`](../specs/2026-08-05-issue-687-site-config-key.md)
**Issue:** [#687](https://github.com/jaunder-org/jaunder/issues/687) · absorbs
#777 · **Milestone** #13

**Goal:** Config keys become a closed registry carrying validators; config
values become specific types reached through typed APIs; raw `get`/`set` stop
being a public seam.

**Architecture:** A `macro_rules!` table in `common` emits `SiteConfigKey` (a
`#[text_enum(sqlx, …)]`) plus a `validate` fn. The storage trait's primitives
retype to take that enum. SMTP values become newtypes, which needs one new
option on the `SqlxBridge` derive. The hand-rolled `InMemorySiteConfig` is
deleted so bridge decoding has a single home.

**Tech Stack:** Rust; `macro_rules!` + the workspace `macros` proc-macro crate;
`strum` via `#[text_enum]` (ADR-0091/0075); `sqlx` over dual SQLite/Postgres;
`rstest`/`rstest_reuse` + `nextest`; `clap`; `cargo xtask` as the gate.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Backend parity (ADR-0019/0053):** any test touching a live pool is
  `#[apply(backends)]`. "It currently hardcodes SQLite" is not a valid reason
  for `sqlite_only`.
- **ADR-0069:** nothing here touches `client/` (filed as #827).
- **The registry is the only way to name a config key** after Task 5.
- **Empty means unset** for `{ optional }` keys — pre-existing contract (spec
  D1b).
- **`SqlxBridge` emits no constructor.**
  `macros/src/sqlx_bridge_derive.rs:56-66` has a live test asserting the derive
  emits no `From`/`FromStr`/`Display`/`Deref` — "a derive that cannot leak a
  constructor" is its charter. Any newtype using it needs a **hand-written**
  `FromStr`/`Display`. Do not "fix" that test.
- The pre-commit hook runs the full `cargo xtask check`; run it first
  (**`jaunder-commit`**).

## Review header

**Scope in:** `common/` (registries, SMTP value types,
`parse_default_audience`), `macros/` (SqlxBridge option), `storage/` (trait,
impls, smtp, user_config, test_support), `server/` (cli, commands, mailer),
`xtask/` (allowlist), `CONTRIBUTING.md`, two ADR drafts. **Scope out:**
`client/src/storage.rs` (#827), `MockSiteConfigStorage`, the 14 defaulted
accessors, the gate's `visit_trait_item_fn` blind spot (Task 1 files it).

**Tasks:**

1. File separable concerns as issues.
2. **Spike:** prove `macro_rules!` can emit `#[macros::text_enum(sqlx, …)]` with
   a substituted `$lit`.
3. `SqlxBridge` derive gains `decode_inner`.
4. Strengthen the 3 SMTP error tests to assert the offending **value** (not the
   variant).
5. SMTP value types **and** the `SiteConfigKey` registry — merged, they are
   mutually dependent.
6. **Retype the seam** — `get`/`set`/`delete` take `SiteConfigKey`; delete 14
   consts; fix every caller including the CLI's clap arg. Compiler-forced,
   atomic, large.
7. `get_smtp_config` required; `get_smtp_credentials` merged away.
8. Delete `InMemorySiteConfig`; 10 tests onto the harness, 2 marked
   `guard:no-backend`.
9. CLI behaviour: validate-on-set, `list` reporting, doc rewrite.
10. `user_config` registry.
11. Allowlist burn-down + `CONTRIBUTING.md` + two ADR drafts.
12. Full `validate` with e2e.

**Key risks / decisions:**

- **Task 2 gates Task 5's shape.** If `$lit` in `#[strum(serialize = $lit)]`
  does not survive `syn`'s parse, fall back to a plain enum plus hand-written
  `strum` derives. Prove it on one entry before building 19.
- **Tasks 4/5/7 are one argument about errors.** Task 4 strengthens the SMTP
  tests to assert the offending value **in the message**; Task 7 then moves
  parsing into the bridges, which makes
  `SmtpConfigError::{InvalidPort,InvalidTlsMode,InvalidSender}` unconstructible.
  A variant-shaped assertion would make Task 7 unimplementable — spec A14 says
  so explicitly. Assert messages, never variants.
- **Task 6 is unavoidably one large commit.** No overloading in Rust, so the
  signature flip, ~40 test sites, the accessor bodies, and the CLI's clap arg
  land together or nothing compiles. Mechanical, not subtle.
- **`macros` has no `sqlx` dependency** (`macros/Cargo.toml:19-24` declares the
  feature as a dep-less cfg). Task 3's tests are therefore token-stream
  assertions in the existing style, not round-trips. Real decode behaviour is
  proven in Task 7 against a live database.

---

### Task 1: File separable concerns

**Files:** none in-tree — tracker only. **Interfaces:** produces an issue number
Task 11 cites.

- [x] **Step 1: File the gate blind-spot issue** — already tracked as
      [#787](https://github.com/jaunder-org/jaunder/issues/787)
      ("sqlx-newtype-decode never scans trait default bodies"). No new issue
      filed.

Check first:
`gh issue list --repo jaunder-org/jaunder --search "visit_trait_item_fn" --state all`

If absent, file via **`jaunder-issues`** (`--type Task`, label `tooling`): the
`sqlx-newtype-decode` `Scanner`
(`xtask/src/steps/sqlx_newtype_decode_check.rs:1356`) implements `visit_item_fn`
and `visit_impl_item_fn` but not `visit_trait_item_fn`, so trait **default**
bodies are never scanned — a decode there is invisible rather than approved.
Note #687 works around it by making `get_smtp_config` required.

- [x] **Step 2: Confirm #827** — OPEN, type Task, milestone "Domain-value type
      safety (newtypes)".

`gh issue view 827 --repo jaunder-org/jaunder --json number,issueType,milestone`
Expected: Task, milestone "Domain-value type safety (newtypes)". Already filed.

- [x] **Step 3: No commit** — tracker-only.

---

### Task 2: Spike — `macro_rules!` emitting `#[macros::text_enum]`

**Files:** Create (temporary) `common/src/config_key_spike.rs`; modify
`common/src/lib.rs`. **Interfaces:** produces the proven macro shape Task 5
builds on.

- [x] **Step 1: Write the spike**

```rust
macro_rules! spike_keys {
    ($($variant:ident => $lit:literal),+ $(,)?) => {
        #[macros::text_enum(sqlx, error = InvalidSpikeKey, message = "unknown spike key")]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::VariantArray)]
        pub enum SpikeKey {
            $( #[strum(serialize = $lit)] $variant, )+
        }
    };
}

spike_keys! { SiteTitle => "site.title", FeedsMinDays => "feeds.min_days", }

#[cfg(test)]
mod tests {
    use super::SpikeKey;
    use std::str::FromStr;

    #[test]
    fn spike_key_round_trips_its_dotted_form() {
        assert_eq!(SpikeKey::from_str("site.title").unwrap(), SpikeKey::SiteTitle);
        assert_eq!(SpikeKey::SiteTitle.as_ref(), "site.title");
        assert!(SpikeKey::from_str("nope").is_err());
    }
}
```

- [x] **Step 2: Run it** — **PASSED.** Both halves hold:
      `#[macros::text_enum(sqlx, …)]` survives `macro_rules!` expansion in
      first-attribute position, and `$lit` in `#[strum(serialize = $lit)]`
      parses. No fallback needed; Task 5 builds the table on this shape.

`devtool run --cwd <worktree> -- cargo nextest run -p common spike_key_round_trips`
Expected: **PASS.** Proves two things — `#[macros::text_enum]` survives as the
item's first active attribute through `macro_rules!` expansion (ADR-0091's
requirement), and `$lit` substituted into `#[strum(serialize = $lit)]` parses.

**If it FAILS**, record which half, and fall back to a plain enum plus
hand-written
`#[derive(strum::AsRefStr, strum::EnumString, strum::Display, strum::IntoStaticStr)]`.
Report before proceeding — Task 5's shape depends on the answer.

- [x] **Step 3: Delete the spike.** The knowledge is the deliverable, not the
      file.
- [x] **Step 4: No commit.** Record the outcome when ticking the box.

---

### Task 3: `SqlxBridge` gains a text-decode option

**Files:** Modify `macros/src/sqlx_bridge_derive.rs`, `macros/src/lib.rs` (the
`proc_macro_derive` attribute list). Test: `macros/src/sqlx_bridge_derive.rs`'s
existing `#[cfg(test)] mod tests` (:41).

**Interfaces:**

- Produces: `#[sqlx_bridge(decode_inner = String)]`. Decode: column text →
  `String` → `FromStr` → `Self`. Encode: `Self` → text. A parse failure becomes
  `ColumnDecode` with the `FromStr` error as `source`.
- Consumed by Task 5's `SmtpPort`.
- **Requires adding `attributes(sqlx_bridge)`** to the `proc_macro_derive` in
  `macros/src/lib.rs` — the derive currently parses no attributes at all.

- [x] **Step 1: Write the failing tests**

Follow the file's existing style — **token-stream assertions**, because `macros`
has no `sqlx` dependency (`macros/Cargo.toml:19-24` declares the feature as a
dep-less cfg) and so cannot execute a real decode. `sqlx_bridge_derive.rs:70-77`
is the model.

```rust
#[test]
fn decode_inner_option_emits_a_string_decode_and_a_fallible_convert() {
    let out = expand_ok(
        "#[sqlx_bridge(decode_inner = String)] struct TextPort(u16);",
    );
    assert!(out.contains("String"), "decode must go through String: {out}");
    assert!(out.contains("from_str"), "convert must route through FromStr: {out}");
}

#[test]
fn without_the_option_the_bridge_is_unchanged() {
    let out = expand_ok("struct IntPort(u16);");
    assert!(!out.contains("from_str"), "no option means no parse step: {out}");
    assert!(out.contains("u16"), "decode inner stays the field type: {out}");
}

#[test]
fn the_derive_still_emits_no_constructor() {
    // The charter (see :56-66): a bridge must not leak a way to build the type.
    let out = expand_ok("#[sqlx_bridge(decode_inner = String)] struct TextPort(u16);");
    for leaked in ["impl ::core::str::FromStr", "impl ::core::fmt::Display",
                   "impl ::core::ops::Deref", "impl ::core::convert::From"] {
        assert!(!out.contains(leaked), "{leaked} must not be emitted: {out}");
    }
}

#[test]
fn an_unknown_option_is_rejected() {
    assert!(expand_err("#[sqlx_bridge(nonsense = String)] struct X(u16);")
        .contains("sqlx_bridge"));
}
```

Adapt `expand_ok`/`expand_err` to the helpers the file already uses; read :41-80
first.

- [x] **Step 2: Run, verify FAIL** — red on exactly the two option-dependent
      tests.

`devtool run --cwd <worktree> -- cargo nextest run -p macros decode_inner`
Expected: **FAIL** — the option is unparsed, so `expand_ok` errors.

- [x] **Step 3: Implement** — spelled `#[sqlx_bridge(text)]`, a bare flag,
      **not** `decode_inner = <ty>`: a TEXT-stored value must move `Type`,
      `Encode` **and** `Decode` together, so naming one of the three would
      under-specify the intent. Deviation from spec A12's wording, deliberate.

Parse the optional attribute; when present, pass `decode_inner` to the existing
`macros::sqlx_bridge::bridge` (`sqlx_bridge.rs:43`, already `pub(crate)`;
`BridgeSpec` already carries `decode_inner` at :38 and a `Result`-yielding
`convert` at :39, so no plumbing is new) with a `FromStr`-routing convert. When
absent, emit exactly today's tokens.

- [x] **Step 4: Run, verify PASS** — 126/126 `macros` tests green.

`devtool run --cwd <worktree> -- cargo nextest run -p macros` Expected: **PASS**
— new tests plus every existing `macros` test, including the no-constructor
charter test.

- [x] **Step 5: Commit** — `ca3bc74c`. **Note:** the auto-staging pre-commit
      hook swept the (already-deleted) Task 2 spike file into this commit
      despite an explicit `git add macros/…`. Its deletion lands in Task 5's
      commit, so the end state is correct; the commit carries a transient file.
      Watch for this on later tasks.

```bash
git add macros/
git commit -m "feat(macros): SqlxBridge text option for text-stored scalars (#687)"
```

---

### Task 4: Strengthen the SMTP error tests

Small, independent, and **must precede Task 7** so the tests lock behaviour
across the read path rewrite rather than being written to fit it.

**Files:** Modify `storage/src/smtp.rs:298-333` (the three tests).

**Interfaces:** produces the regression lock Task 7 must keep green.

- [x] **Step 1: Rewrite the three assertions**

They assert only variant shape today (`:305,:317,:329`). Replace with
**message** assertions. Per spec A14, do **not** assert the variant: Task 7
moves parsing into the bridges, which makes those three variants
unconstructible, and a `matches!` assertion would make Task 7 unimplementable.

```rust
// storage/src/smtp.rs, in load_smtp_config_returns_err_for_invalid_port
let err = /* existing setup, unchanged */;
assert!(
    err.to_string().contains("not-a-port"),
    "the error must echo the offending value; got: {err}"
);
```

…and the same shape for `..._invalid_tls_mode` and `..._invalid_sender`, each
asserting its own seeded bad value. **Keep the existing seeds**; only the
assertion changes.

- [x] **Step 2: Run, verify PASS** — 5/5 green immediately; the values already
      reach the messages, so no production error change was needed (the
      stop-condition did not fire).

`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p storage load_smtp_config_returns_err`
Expected: **PASS** immediately — the variants already carry the value
(`smtp.rs:100,103,106`, constructed at `:145,:152,:175`) and `Display` renders
it.

**If any fails, stop.** It means the value is not actually reaching the message,
and A14 needs the production error changed — a bigger change than this task.

- [x] **Step 3: Commit** — `aaf937c8`.

```bash
git add storage/src/smtp.rs
git commit -m "test(storage): pin the offending value in SMTP config errors (#687)"
```

---

### Task 5: SMTP value types and the `SiteConfigKey` registry

Merged deliberately: the registry's table needs the four SMTP types, and the
types' only initial consumer is the registry. Split, neither half is
independently verifiable.

**Files:**

- Create: `common/src/config_key.rs`; SMTP types beside
  `SmtpUsername`/`SmtpPassword` (find with `rg -n 'SmtpUsername' common/src` and
  follow that module)
- Modify: `common/src/lib.rs`; `common/src/visibility.rs` (receives
  `parse_default_audience`); `storage/src/site_config.rs:315` (parser moves
  out); `storage/src/smtp.rs` (delete the hand-rolled `SmtpTlsMode` `Display`
  :25 / `FromStr` :40, re-export from `common`)

**Interfaces:**

- Produces `SmtpHost`, `SmtpSender` (`StrNewtype`, validates
  Mailbox-parseability), `SmtpPort` (`SqlxBridge` + `decode_inner = String`,
  holds `u16`, **hand-written `FromStr` and `Display`** — the derive emits
  none), `SmtpTlsMode` (`#[text_enum(sqlx, …)]`, `plain`/`starttls`/`tls`).
- Produces `SiteConfigKey`: 19 variants,
  `validate(self, &str) -> Result<(), InvalidSiteConfigValue>`,
  `is_optional(self) -> bool`, `VARIANTS`, `FromStr`/`AsRef<str>`/`Display`,
  sqlx `Encode`/`Decode`.
- Depends on Tasks 2 and 3.

- [x] **Step 1: Move `parse_default_audience` into `common`**

It is a private fn in the **storage** crate (`site_config.rs:315`), and
`AudienceTarget` (`common/src/visibility.rs:127`) has a `Named(_)` variant and
no `FromStr` — so a registry in `common` cannot reach its parser where it sits.
Move it (and `default_audience_str`, its inverse) beside the type. Not a cycle:
`storage → common` is the only edge and this removes a use of it.

- [x] **Step 2: Write the failing tests**

```rust
#[test]
fn every_key_round_trips_its_dotted_form() {
    for key in SiteConfigKey::VARIANTS {
        assert_eq!(SiteConfigKey::from_str(key.as_ref()).ok().as_ref(), Some(key));
        assert!(key.as_ref().contains('.'), "{} must be namespace.name", key.as_ref());
    }
    assert_eq!(SiteConfigKey::VARIANTS.len(), 19);
}

#[test]
fn unknown_keys_are_rejected() {
    for bad in ["", "site", "site.nope", "nope.title", " site.title"] {
        assert!(SiteConfigKey::from_str(bad).is_err(), "{bad} must reject");
    }
}

/// A4. There is no universal junk string: `SiteTitle`, `SmtpUsername`, `SmtpPassword`
/// and `DestinationPath` reject only the empty string, so each key carries its own
/// known-bad example in the table and the test reads it back.
#[test]
fn every_key_rejects_its_known_bad_value() {
    for key in SiteConfigKey::VARIANTS {
        let bad = key.known_bad_example();
        assert!(key.validate(bad).is_err(), "{} must reject {bad:?}", key.as_ref());
    }
}

/// A5: the empty-means-unset contract, pinned per key.
#[test]
fn optional_keys_accept_empty_and_others_reject_it() {
    for key in SiteConfigKey::VARIANTS {
        let got = key.validate("");
        assert_eq!(got.is_ok(), key.is_optional(),
            "{} optional={} but validate(\"\")={:?}", key.as_ref(), key.is_optional(), got);
    }
}
```

`known_bad_example` is `#[cfg(test)]`-only or `#[doc(hidden)]`, emitted by the
macro from a per-entry column. It exists because a single junk string cannot
fail every validator — four of the value types reject only `""`, which is
precisely what A5 covers.

- [x] **Step 3: Run, verify FAIL**

`devtool run --cwd <worktree> -- cargo nextest run -p common config_key smtp`
Expected: **FAIL** — nothing exists yet.

- [x] **Step 4: Write the types, the macro, and the table** — 19 entries,
      verified against this table. Row syntax is
      `Variant => "dotted" : <value> { optional }?, bad: "…";` because
      `macro_rules!` follow-set rules make the illustrative form below
      unparseable next to the custom-parser escape; semantics are unchanged.
      `InvalidSiteConfigValue` carries the key and a reason but **not** the
      offending value — `smtp.password` is in the table, and echoing it would
      leak a secret. `SmtpPort` also rejects `0` and gained a `value()` reader
      (a bridge-only newtype with no reader is unusable).

The `{ optional }` marker drives both the empty-accepting validator and
`is_optional`, so the two cannot disagree. The 19 entries, with the type each
validator parses into:

| key                         | type                                  | optional |
| --------------------------- | ------------------------------------- | -------- |
| `backup.destination_path`   | `DestinationPath`                     | yes      |
| `backup.schedule`           | `BackupSchedule`                      | no       |
| `backup.retention_count`    | `RetentionCount`                      | no       |
| `backup.mode`               | `BackupMode`                          | no       |
| `feeds.min_items`           | `FeedMinItems`                        | no       |
| `feeds.min_days`            | `FeedMinDays`                         | no       |
| `feeds.websub_hub_url`      | `AbsoluteUrl`                         | yes      |
| `posts.default_audience`    | via `parse_default_audience` (Step 1) | no       |
| `site.registration_policy`  | `RegistrationPolicy`                  | no       |
| `site.title`                | `SiteTitle`                           | no       |
| `site.base_url`             | `AbsoluteUrl`                         | yes      |
| `media.max_file_size_bytes` | `MaxFileSize`                         | no       |
| `media.user_quota_bytes`    | `UserQuota`                           | no       |
| `smtp.host`                 | `SmtpHost`                            | no       |
| `smtp.port`                 | `SmtpPort`                            | no       |
| `smtp.tls_mode`             | `SmtpTlsMode`                         | no       |
| `smtp.sender`               | `SmtpSender`                          | no       |
| `smtp.username`             | `SmtpUsername`                        | no       |
| `smtp.password`             | `SmtpPassword`                        | no       |

The table needs a custom-parser escape for `posts.default_audience` (it is not a
`FromStr`) and a `known_bad_example` column per entry.

- [x] **Step 5: Run, verify PASS** — 525/525 `common` tests green, including all
      four required registry tests. Full `cargo xtask check` also green.

`devtool run --cwd <worktree> -- cargo nextest run -p common` Expected:
**PASS**, all four registry tests plus the SMTP type tests.

- [ ] **Step 6: Commit**

```bash
git add common/ storage/src/smtp.rs storage/src/site_config.rs
git commit -m "feat(common): typed SMTP values and the SiteConfigKey registry (#687)"
```

---

### Task 6: Retype the seam

The large, compiler-forced, mechanical one.

**Files:**
`storage/src/{site_config,media,test_support,atomic,media_manager}.rs`,
`storage/src/postgres/migrations.rs:15`, `server/src/{commands,cli}.rs`,
`server/tests/storage/mod.rs`, `server/tests/misc/commands.rs`,
`server/tests/web/{web_auth,web_account,web_backup,web_media}.rs`,
`server/tests/feed/feed_worker.rs`, `server/tests/helpers/mod.rs`.

**Interfaces:** produces `get(SiteConfigKey) -> Result<Option<String>>`,
`set(SiteConfigKey, &str) -> Result<()>`,
`delete(SiteConfigKey) -> Result<bool>`. `list()` unchanged. Consumes Task 5's
registry.

- [ ] **Step 1: Write the failing test**

```rust
#[apply(backends)]
#[tokio::test]
async fn site_config_round_trips_through_typed_keys(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state.site_config.set(SiteConfigKey::SiteTitle, "My Site").await.unwrap();
    assert_eq!(
        state.site_config.get(SiteConfigKey::SiteTitle).await.unwrap().as_deref(),
        Some("My Site")
    );
    assert_eq!(state.site_config.get(SiteConfigKey::FeedsMinDays).await.unwrap(), None);
    assert!(state.site_config.delete(SiteConfigKey::SiteTitle).await.unwrap());
    assert_eq!(state.site_config.get(SiteConfigKey::SiteTitle).await.unwrap(), None);
}
```

- [ ] **Step 2: Run, verify FAIL**

`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p jaunder site_config_round_trips_through_typed_keys`
Expected: **FAIL** — compile error; `set` takes `&str`.

- [ ] **Step 3: Flip the signatures and follow the compiler**

Four rules for the mechanical pass:

- **Accessor bodies** swap `SITE_TITLE_KEY` → `SiteConfigKey::SiteTitle`.
  Behaviour otherwise **unchanged** — `unwrap_or_default` / `unwrap_or(Closed)`
  / purge paths stay (spec D7).
- **Bind the key directly** — `.bind(key)`, never `.bind(key.as_ref())`, which
  `sqlx_newtype_bind_check.rs:12-22` forbids across `storage/src`.
- **The CLI's clap arg is included here**, not deferred: `key: String` →
  `key: SiteConfigKey` at `cli.rs:391,404,419`, and
  `cmd_site_config_{set,get,unset}` retyped. Only the _signature_ moves in this
  task — validate-on-set and `list` reporting are Task 9. The three CLI tests
  asserting an error for an unknown key (`cli.rs:917,934,986`;
  `commands.rs:895`) become `Cli::try_parse_from(...)` rejection tests **now**,
  because they cannot compile otherwise.
- **Bogus-key test sites** (`"example.setting"`, `"test.key"`,
  `"nonexistent.key"`, `"site.name"`, `"missing"`, `"some.key"`, `"only.key"`)
  move onto real keys — they exercise primitive semantics and do not care which
  key. `postgres/migrations.rs:15` likewise.

- [ ] **Step 4: Run the full suites**

`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p storage`
`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p jaunder`
Expected: **PASS** on both backends. Breadth is the verification here; the
change is mechanical.

- [ ] **Step 5: Count the remaining consts**

`devtool run --cwd <worktree> -- rg -n 'const [A-Z_]+_KEY' storage/src`
Expected: exactly **3** hits — `TEMPLATE_LOCK_KEY` (`test_support.rs:539`, an
advisory-lock id, never removed) plus `USER_MEDIA_CACHE_POLICY_KEY` and
`DEFAULT_POST_FORMAT_KEY` (`user_config.rs:26,29`), which Task 10 deletes. Spec
A1's final state of **1** hit is reached at Task 10, not here.

- [ ] **Step 6: Commit**

```bash
git add storage/ server/
git commit -m "refactor(storage): site-config get/set/delete take SiteConfigKey (#687)"
```

---

### Task 7: One required SMTP read

**Files:** `storage/src/site_config.rs` (delete `get_smtp_credentials` :54 and
its impl :384-401; add required `get_smtp_config`), `storage/src/smtp.rs`,
`storage/src/test_support.rs`, plus any namer of `get_smtp_credentials`
(`rg -n get_smtp_credentials`).

**Interfaces:** produces required
`async fn get_smtp_config(&self) -> sqlx::Result<Option<SmtpConfig>>`;
`SmtpConfig` grows the credential fields;
`get_smtp_credentials`/`SmtpCredentials` deleted. Consumes Tasks 3 and 5.

- [ ] **Step 1: Write the failing tests**

```rust
#[apply(backends)]
#[tokio::test]
async fn get_smtp_config_returns_none_when_host_unset(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    assert!(state.site_config.get_smtp_config().await.unwrap().is_none());
}

#[apply(backends)]
#[tokio::test]
async fn get_smtp_config_reads_every_value_typed(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cfg = &state.site_config;
    cfg.set(SiteConfigKey::SmtpHost, "mail.example.com").await.unwrap();
    cfg.set(SiteConfigKey::SmtpPort, "2525").await.unwrap();
    cfg.set(SiteConfigKey::SmtpTlsMode, "tls").await.unwrap();
    cfg.set(SiteConfigKey::SmtpSender, "Jaunder <noreply@example.com>").await.unwrap();

    let got = cfg.get_smtp_config().await.unwrap().expect("host is set");
    assert_eq!(got.host.as_ref(), "mail.example.com");
    assert_eq!(got.port.value(), 2525);
    assert_eq!(got.tls_mode, SmtpTlsMode::Tls);
}

/// A bad stored value fails at the query boundary, not silently — and still echoes
/// the offending value, which is what Task 4's tests lock.
#[apply(backends)]
#[tokio::test]
async fn get_smtp_config_rejects_a_bad_stored_port(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state.site_config.set(SiteConfigKey::SmtpHost, "mail.example.com").await.unwrap();
    state.site_config.set(SiteConfigKey::SmtpPort, "not-a-port").await.unwrap();
    let err = state.site_config.get_smtp_config().await.unwrap_err();
    assert!(err.to_string().contains("not-a-port")
            || format!("{:?}", err).contains("not-a-port"),
        "the decode error must echo the offending value; got {err}");
}
```

The third test can only be set up because `set` takes a raw `&str` value — the
CLI validator (Task 9) would refuse it. Deliberate: the read path stays
defensive about rows the CLI did not write.

- [ ] **Step 2: Run, verify FAIL; implement; run, verify PASS**

`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p jaunder get_smtp_config`
Implement in the generic impl reading through the bridges — the deleted
`get_smtp_credentials` (`site_config.rs:384-401`) is the model — then rewrite
`load_smtp_config` to call it. Afterwards
`rg -n 'self\.get\(|store\.get\(' storage/src/smtp.rs` must return nothing.

- [ ] **Step 3: Reconcile the three error variants**

`SmtpConfigError::{InvalidPort,InvalidTlsMode,InvalidSender}` are constructed
only inside the parsing this task just deleted (`smtp.rs:145,152,175`), so they
are now unconstructible. Either delete them, or keep them as a mapping applied
to the `ColumnDecode` — **your choice**, but Task 4's tests must stay green
either way. Run them:

`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p storage load_smtp_config_returns_err`
Expected: **PASS.** That they survive a rewritten read path is the whole point
of having strengthened them in Task 4 rather than here. If they cannot be made
green without asserting a variant, stop and report — that contradicts spec A14
and needs a decision.

- [ ] **Step 4: Commit**

```bash
git add storage/ web/ server/
git commit -m "refactor(storage): one required get_smtp_config replaces the raw reads (#687)"
```

---

### Task 8: Delete the in-memory fake

**Files:** `storage/src/test_support.rs` (delete `InMemorySiteConfig`
:1405-1479), `storage/src/smtp.rs` (8 tests), `storage/src/site_config.rs` (1
test), `server/src/mailer/mod.rs` (3 tests).

**Interfaces:** consumes the harness —
`Backend::setup() -> TestEnv { state, base }` (`test_support.rs:247`);
`backends` is `#[template] #[export]` (:299-304) so `#[apply(backends)]` works
cross-crate; `server/Cargo.toml:72` already dev-depends on `storage` with
`["test-utils", "test-support"]`.

- [ ] **Step 1: Convert the tests**

10 of 12 move onto `#[apply(backends)]` + `backend.setup()`, dropping
`guard:no-backend`. The other **2** — the `server/src/mailer/mod.rs` tests that
construct the fake empty and never read config — keep a marker:
`// guard:no-backend — builds a mailer from a literal config, touches no store`.
Assertions do not change; only the store's provenance does.

- [ ] **Step 2: Delete the fake** — the struct and its `impl SiteConfigStorage`.

- [ ] **Step 3: Run, verify PASS**

`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p storage`
`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p jaunder`
Expected: **PASS** both backends. The SMTP error tests now exercise the **real**
parser; previously they proved the fake's hand-rolled mirror rejected a bad
port.

`devtool run --cwd <worktree> -- rg -n InMemorySiteConfig` Expected: no hits
outside `docs/archive/` (A15).

- [ ] **Step 4: Commit**

```bash
git add storage/ server/
git commit -m "test(storage): delete InMemorySiteConfig for the real harness (#687)"
```

---

### Task 9: CLI behaviour

Task 6 already retyped the clap arg. This task adds the behaviour.

**Files:** `server/src/commands.rs:703-760` (set/get/unset/list +
`format_entries` :727,:743), `server/src/cli.rs:379-382` (the doc comment).
Test: `server/tests/misc/commands.rs`.

**Interfaces:** consumes `SiteConfigKey::{validate, is_optional}`.

- [ ] **Step 1: Write the failing tests**

```rust
/// A7: a known key with an invalid value is rejected before the write.
#[apply(backends)]
#[tokio::test]
async fn site_config_set_rejects_an_invalid_value(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let before = state.site_config.list().await.unwrap().len();
    assert!(cmd_site_config_set(&args, SiteConfigKey::SiteBaseUrl, "nonsense").await.is_err());
    assert_eq!(state.site_config.list().await.unwrap().len(), before, "no row written");
}

/// A8: empty-means-unset survives at the CLI door.
#[apply(backends)]
#[tokio::test]
async fn site_config_set_accepts_empty_for_an_optional_key(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    assert!(cmd_site_config_set(&args, SiteConfigKey::SiteBaseUrl, "").await.is_ok());
}

/// A9: list is a faithful dump that judges without hiding.
#[apply(backends)]
#[tokio::test]
async fn site_config_list_flags_unknown_keys_and_invalid_values(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    // A row the registry does not know. `set` cannot express it any more, which is
    // exactly the legacy case `list` exists to surface -- so write it as raw SQL
    // through the harness pool.
    base.pool()
        .execute("INSERT INTO site_config (key, value) VALUES ('legacy.orphan', 'x')")
        .await
        .unwrap();
    state.site_config.set(SiteConfigKey::SiteBaseUrl, "nonsense").await.unwrap();
    state.site_config.set(SiteConfigKey::SiteTitle, "My Site").await.unwrap();

    let rendered = format_entries(&state.site_config.list().await.unwrap());
    assert!(rendered.contains("legacy.orphan") && rendered.contains("UNKNOWN KEY"));
    assert!(rendered.contains("site.base_url") && rendered.contains("INVALID"));
    assert!(rendered.contains("site.title=My Site"));
    assert!(!rendered.lines().any(|l| l.starts_with("site.title") && l.contains("INVALID")));
}
```

`base.pool()` is `TestBase::pool() -> &CloseablePool` (`test_support.rs:203`)
and raw SQL goes through `CloseablePool::execute` (:94) — note this test keeps
`base` rather than discarding it as `base: _base`. Adapt `&args` to
`cmd_site_config_set`'s real `StorageArgs` parameter; read `commands.rs:703`
first.

- [ ] **Step 2: Run, verify FAIL; implement; run, verify PASS**

`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p jaunder site_config_`

`cmd_site_config_set` calls `key.validate(value)?` before `set`.
`format_entries` parses each key and, when it parses, runs the validator —
skipping the invalid flag when the value is empty and the key `is_optional`.

- [ ] **Step 3: Rewrite the doc comment**

`cli.rs:379-382` says "free-form key/value store; values are not validated" and
carries a stale key list. Both halves are now false.

- [ ] **Step 4: Commit**

```bash
git add server/
git commit -m "feat(cli): validated site-config writes and a judging list (#687)"
```

---

### Task 10: The `user_config` registry

**Files:** `common/src/config_key.rs` (add `user_config_keys!`),
`storage/src/user_config.rs`, `server/tests/storage/mod.rs:5378-5444`,
`server/tests/misc/backup_fixture.rs:143,231`.

**Interfaces:** produces `UserConfigKey` with one variant,
`DefaultPostFormat => "posts.default_format" : PostFormat`.

- [ ] **Step 1: Write the failing tests**

```rust
#[apply(backends)]
#[tokio::test]
async fn user_config_round_trips_through_typed_keys(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user = SeedUser::new().seed(&state).await.user_id;
    state.user_config.set(user, UserConfigKey::DefaultPostFormat, "markdown").await.unwrap();
    assert_eq!(
        state.user_config.get(user, UserConfigKey::DefaultPostFormat).await.unwrap().as_deref(),
        Some("markdown")
    );
}

#[test]
fn user_config_key_validates_its_value() {
    assert!(UserConfigKey::DefaultPostFormat.validate("markdown").is_ok());
    assert!(UserConfigKey::DefaultPostFormat.validate("hieroglyphs").is_err());
}
```

- [ ] **Step 2: Run, verify FAIL; implement; run, verify PASS**

`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p jaunder user_config`
Test sites using `"theme"`, `"editor.theme"`, `"some.key"` move onto the real
key. Delete `USER_MEDIA_CACHE_POLICY_KEY` (:26, referenced nowhere) and
`DEFAULT_POST_FORMAT_KEY` (:29).

- [ ] **Step 3: Reach spec A1's final state**

`devtool run --cwd <worktree> -- rg -n 'const [A-Z_]+_KEY' storage/src`
Expected: exactly **1** hit — `TEMPLATE_LOCK_KEY`. All 16 config-key consts are
gone.

- [ ] **Step 4: Commit**

```bash
git add common/ storage/ server/
git commit -m "refactor(storage): user-config get/set/delete take UserConfigKey (#687)"
```

---

### Task 11: Allowlist burn-down and docs

**Files:**
`xtask/src/steps/sqlx_newtype_decode_check.rs:121-122,755,765,783-900`,
`CONTRIBUTING.md:177`, `docs/adr/drafts/` (two new).

- [ ] **Step 1: Delete the seven entries**

All seven `Category::NotADecodeTarget` entries (:789, :800, :809, :818, :880,
:890, :899) — **deleted, not recategorised**; the staleness check enforces this.
Update the module doc at :121-122 ("four `load_smtp_config` reads and three in
`test_support.rs`") — both populations are gone.

Then the two `DeferredNewtype` entries naming #687: `site_config::delete` (:765)
should die — it now takes `SiteConfigKey`. `site_config::list` (:755)
**survives** — `list` deliberately stays `Vec<(String, String)>` (spec D4) — so
rewrite its reason to say why it is permanent rather than pointing at #687 as
pending. The third `DeferredNewtype` (`subscriptions.rs:828`) is unrelated;
leave it.

- [ ] **Step 2: Verify the remaining criteria that no test covers**

```
rg -n 'media\.cache_policy_default' --glob '!docs/archive/**' --glob '!docs/superpowers/**'
```

Expected: **no matches** (A3's second half — the literal, not just the const
name).

```
rg -n 'expect_get\b|expect_set\b|expect_delete\b' server/src storage/src
```

Expected: **no matches** (A18 — D9's argument is that no `MockSiteConfigStorage`
site touches the three retyped primitives, which is why all 16 compile
unchanged).

- [ ] **Step 3: Run the gate**

`devtool run --cwd <worktree> -- cargo xtask check` Expected: **PASS**, with
`sqlx-newtype-decode` and `sqlx-newtype-bind` green. A stale allowlist entry
fails the staleness check, so green here proves the entries were genuinely
obsolete rather than merely deleted.

- [ ] **Step 4: `CONTRIBUTING.md` and the ADR drafts**

`CONTRIBUTING.md:177` still says "For tests requiring a database, use
`sqlite::memory:`", contradicting both the harness and ADR-0033. Rewrite to
point at `backend.setup()` and `#[apply(backends)]`.

Two numberless drafts via **`jaunder-adr`** (numbered at ship by
`cargo xtask adr promote`):

1. **Config keys are a closed registry carrying validators.** Context:
   `set(&str, &str)` was transposable and the key space had accreted across
   three files. Decision: a `macro_rules!` table emits the key enum plus a
   mandatory per-key validator; raw `get`/`set` are legal only inside accessor
   bodies. Consequences: adding a key is one table line and cannot omit a
   validator; the CLI rejects unknown keys and invalid values at the door; read
   paths stay defensive because rows can predate the registry.
2. **Prefer the real harness over a mirroring fake.** Context:
   `InMemorySiteConfig` hand-mirrored the backend's decode-failure behaviour, so
   tests proved the mirror rather than the thing, and the mirror needed three
   allowlist entries. Decision: when a fake would have to reproduce backend
   behaviour to be useful, use `backend.setup()`. Consequence: such tests become
   dual-backend; a fake stays right where it stands in for a collaborator that
   is _not_ under test (`MockSiteConfigStorage` asserting non-interaction).

- [ ] **Step 5: Commit**

```bash
git add xtask/ CONTRIBUTING.md docs/adr/drafts/
git commit -m "chore(xtask): burn down the site-config decode allowlist (#687)"
```

---

### Task 12: Full-gate verification

**Files:** none — verification only, no commit.

- [ ] **Step 1: Verify-only gate (A21)**

`devtool run --cwd <worktree> -- cargo xtask validate --no-e2e` Expected:
**PASS**. Not redundant with Task 11's `check`: `check` auto-fixes formatting,
so a green `check` can leave the tree mutated after the commit. `validate` is
verify-only.

- [ ] **Step 2: Full gate with e2e (A22)**

`devtool run --cwd <worktree> -- cargo xtask validate` (Bash background mode;
~25 min) Expected: **PASS**, all four `{sqlite,postgres}×{chromium,firefox}`
combos.

Required rather than optional: e2e seeding shells out to
`jaunder site-config set` for `site.registration_policy`, `site.base_url`, and
`feeds.websub_hub_url` (`tools/devtool/src/seed_e2e.rs:38-49`) — the exact CLI
surface Tasks 6 and 9 retype. All three keys are in the registry and their
values pass their validators, so this should be green; if it is not, the CLI
contract broke and no unit test would have caught it.

- [ ] **Step 3: On failure, read the sidecar**

`devtool run --cwd <worktree> -- jq '.steps[] | select(.ok == false)' .xtask/last-result.json`
