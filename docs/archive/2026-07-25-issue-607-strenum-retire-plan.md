# Plan — #607: Retire StrEnum

Spec:
[`docs/superpowers/specs/2026-07-25-issue-607-strenum-retire.md`](../specs/2026-07-25-issue-607-strenum-retire.md)
(the "what/why"). Issue: jaunder-org/jaunder#607.

## Review header

**Goal.** Migrate the six remaining `StrEnum` enums to the `strum` stack (each
with a named `thiserror` error + specific message, `PostFormat` shape),
de-stringly the DB-column and config crossings, delete the `StrEnum` macro, fix
the ADR-0075 prose.

**Scope — in:** AC-1..AC-7 from the spec. **Out:** the spec's Non-goals (no new
bespoke macro, no new host registrations, no typed setter / CLI rewrite,
`RegistrationPolicy` keeps its safe `Closed` default). No separable concerns to
file.

**Tasks (one commit each):**

1. Migrate `Channel` + `SubscriptionStatus` (visibility.rs; non-serde, no DB).
2. Migrate `TargetKind` (visibility.rs) + adopt `impl_text_column_enum!` in
   posts storage.
3. Migrate `AudienceBase` (visibility.rs; serde).
4. Migrate `MediaSource` (media.rs; serde + DB) — preserve custom message + host
   registration.
5. Migrate `RegistrationPolicy` (registration.rs; serde) + typed
   `get_registration_policy` accessor.
6. Delete the `StrEnum` macro + its tests; prove `rg 'StrEnum' -g '!docs/**'`
   empty.
7. Fix the stale ADR-0075 prose line.

**Key risks/decisions.**

- **Macro coexists until task 6** — a partially-migrated tree compiles because
  un-migrated enums still derive `StrEnum` while migrated ones use strum. So
  tasks 1–5 are independently landable in any order; **task 6 must be last**
  (macro has no users left). Task 7 is doc-only, anytime.
- **Representation-compat is the invariant**: identical wire tokens (round-trip
  tests) and byte-identical DB tokens (`impl_text_column_enum!` encodes
  `as_ref().to_owned()`). Any migration that would shift a token is wrong.
- Tasks 1–3 share `common/src/visibility.rs` (disjoint enum defs) — sequenced,
  each leaves the file compiling.
- Tasks 2/4 touch dual-backend storage — gate each on the existing posts/media
  storage tests (ADR-0053 whole-`TestEnv` binding).

**For agentic workers.** Drive with **`jaunder-iterate`**; the multi-file
storage wiring in tasks 2/4 and the macro deletion in task 6 are dispatch
candidates (**`jaunder-dispatch`**) but small enough to do inline. Tick
checkboxes in real time.

## Global constraints

- Each enum adopts the **`PostFormat` shape** (common/src/render.rs:25-84 is the
  copy-me):
  `#[derive(… strum::{VariantArray, AsRefStr, IntoStaticStr, EnumString, Display})]` +
  `#[strum(serialize_all = "snake_case")]` +
  `#[strum(parse_err_ty = InvalidX, parse_err_fn = x_parse_err)]`; a
  `#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)] #[error("…")] pub struct InvalidX;` +
  `fn x_parse_err(_: &str) -> InvalidX { InvalidX }`. **No `EnumMessage`** (none
  of the six carry UI labels).
- serde enums add `#[serde(into = "String", try_from = "String")]` +
  `From<T> for String` / `TryFrom<String>` (route deserialize through the named
  error). Non-serde enums derive no serde.
- **`as_str()`→`as_ref()` sweep (cold-review finding):** strum gives `as_ref()`,
  not the `StrEnum` inherent `as_str()`. In each enum's task,
  `rg '<Enum>' …'\.as_str\('` across the whole tree and migrate every hit to
  `.as_ref()` — including sites outside `common` (named per task).
- Remove each `use macros::StrEnum;` import (keep sibling macro imports in
  media.rs).
- Web checks add
  `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings`.
  **Storage tests are storage-crate unit tests** —
  `cargo nextest run -p storage <filter>` (NOT `-p jaunder --test integration`),
  except server-fn integration paths (`-p jaunder --test integration web_*`).
- Commit only after `cargo xtask check` clean (pre-commit hook runs it); **no
  `Co-Authored-By`**; request review before merge. No new `cov:ignore`.

---

## Task 1 — `Channel` + `SubscriptionStatus`

**Files:** `common/src/visibility.rs`.

- Swap both `#[derive(… StrEnum)]` for the PostFormat-shape strum derive (no
  serde). Tokens preserved: `Channel::Local`→`local`; `SubscriptionStatus`→
  `active`/`pending`/`blocked`.
- Named errors `InvalidChannel` ("channel must be \"local\"") and
  `InvalidSubscriptionStatus` ("subscription status must be \"active\",
  \"pending\", or \"blocked\"").
- Update the shared `display_matches_as_str` / round-trip tests (visibility.rs)
  to the new types if needed; add a `FromStr` reject test per enum.

**Verify:** `cargo nextest run -p common visibility` PASS; `cargo xtask check`
green. **Commit:**
`refactor(common): migrate Channel + SubscriptionStatus to strum (#607)`

## Task 2 — `TargetKind` (strum-only; FK-normalized, NOT impl_text_column_enum!)

**Files:** `common/src/visibility.rs`, `storage/src/posts.rs` (+
`sqlite/posts.rs` call site).

- Migrate `TargetKind` (non-serde) to the strum shape + `InvalidTargetKind`
  ("audience target kind must be \"public\", \"subscribers\", or \"named\"").
- **Do NOT add `impl_text_column_enum!`** (cold-review correction): `TargetKind`
  is a normalized FK, stored via
  `(SELECT kind_id FROM target_kinds WHERE name = ?)` and read through a join +
  `filter_map` that **drops** unknown kinds (`storage/src/posts.rs`
  `"bogus" => None` test). Keep that shape: the `String` + `filter_map` decode
  is **unchanged** (drop-unknown preserved). Only the enum definition changes.
- **Bind pattern (learned in Task 1):** the `storage/src/**` `sqlx-newtype-bind`
  guard forbids `.bind(x.as_ref())`. `StrEnum` was clean because it bound
  `.as_str()`; strum's `.as_ref()` is flagged. For an FK-name bind, derive
  `strum::IntoStaticStr` too and bind a `&'static str` local
  (`let name: &'static str = kind.into(); … .bind(name)`) — guard-clean, typed,
  no allocation, no unused `Decode`. (Non-storage `.as_str()` sites →
  `.as_ref()`.)

**Verify:** `cargo nextest run -p storage posts` PASS both backends (incl. the
drop-unknown test, unchanged); `cargo xtask check` green. **Commit:**
`refactor(common,storage): migrate TargetKind to strum (#607)`

## Task 3 — `AudienceBase`

**Files:** `common/src/visibility.rs`, `web/src/posts/component.rs` (`.as_str()`
site).

- Migrate `AudienceBase` (serde, `#[default] Private`) to the strum shape +
  `InvalidAudienceBase` ("audience must be \"private\", \"public\", or
  \"subscribers\"") + `#[serde(into/try_from = "String")]` + the
  `From`/`TryFrom` pair. Tokens `private`/`public`/`subscribers` preserved;
  `Default = Private`.
- Sweep `AudienceBase…as_str()` → `.as_ref()` (known:
  `web/src/posts/component.rs:379`; `rg` for others).
- Keep the existing serde-literal + reject-unknown + default tests; add a
  `serde_qs` form round-trip (it travels in the audience DTO).

**Verify:** `cargo nextest run -p common visibility` PASS; wasm-clippy green.
**Commit:** `refactor(common): migrate AudienceBase to strum (#607)`

## Task 4 — `MediaSource` + typed media column

**Files:** `common/src/media.rs`,
`storage/src/{media.rs, helpers.rs, sqlite/media.rs, postgres/media.rs}`,
`server/src/media.rs`, `web/src/media/api.rs` (`.as_str()` sites),
(host/src/error.rs unchanged — verify).

- Migrate `MediaSource` (serde) to the strum shape. **Preserve** the custom
  error message: `InvalidMediaSource` with
  `#[error("media source must be \"upload\" or \"cached\"")]`. serde via
  `into/try_from = "String"`.
- **Preserve** `host/src/error.rs:387`
  `validation_from!(common::media::InvalidMediaSource)`
  - its `check!` — the type name/path is unchanged, so this keeps compiling;
    confirm.
- Add `crate::db_enum::impl_text_column_enum!(MediaSource);` **in
  `common/src/media.rs`** (mirroring `PostFormat` in `render.rs`). Switch the
  media binds (`storage/src/media.rs`, `sqlite/media.rs`, `postgres/media.rs`)
  from `.bind(x.as_str())` to typed `.bind(*x)` (the value is `Copy`; Encode is
  on the value), and the decode in
  `storage/src/helpers.rs::media_record_from_row` from `col.parse()` → typed
  column. Byte-identical token; the decode still rejects an unknown as
  `ColumnDecode` (preserved).
- Sweep `MediaSource…as_str()` → `.as_ref()` (known: `server/src/media.rs:243`,
  `web/src/media/api.rs:87` & `:138`; `rg` for others).

**Verify:** `cargo nextest run -p common media` +
`cargo nextest run -p storage media` PASS both backends +
`cargo nextest run -p jaunder --test integration web_media`; wasm-clippy green.
Grep: no `MediaSource…as_str()` / `.parse()` strip at those sites. **Commit:**
`refactor(common,storage): MediaSource via strum + impl_text_column_enum! (#607)`

## Task 5 — `RegistrationPolicy` + typed config accessor

**Files:** `common/src/registration.rs`, `storage/src/site_config.rs`,
`storage/src/auth.rs`, `storage/src/atomic.rs` (inline read at :220 + seed write
at :226), `web/src/registration/api.rs`, `web/src/invites/api.rs`.

- Migrate `RegistrationPolicy` (serde) to the strum shape +
  `InvalidRegistrationPolicy` ("registration policy must be \"open\",
  \"invite_only\", or \"closed\"") + serde `into/try_from`. Tokens
  `open`/`invite_only`/`closed` preserved (snake_case).
- `storage/src/site_config.rs`: add `SITE_REGISTRATION_POLICY_KEY` const + a
  default trait method
  `get_registration_policy(&self) -> sqlx::Result<RegistrationPolicy>` =
  `Ok(self.get(KEY).await?.as_deref().and_then(|v| v.parse().ok()) .unwrap_or(RegistrationPolicy::Closed))`
  (sibling of `get_backup_config`).
- `storage/src/auth.rs`: delete `load_registration_policy` (+ its `pub use`);
  move its behavior tests to the site_config test module
  (absent/open/invite_only/garbage→Closed, dual-backend).
- Migrate callers: `web/src/registration/api.rs` (:47 read, :72
  read-in-register), `web/src/invites/api.rs:96`, and the inline read in
  `storage/src/atomic.rs:220` from `load_registration_policy(&*store).await` /
  `.get("site.registration_policy")` → `store.get_registration_policy().await?`
  (propagate the sqlx error; absent/invalid still `Closed`). Point the seed
  write at `atomic.rs:226` and any other inline literal at the new
  `SITE_REGISTRATION_POLICY_KEY` const.

**Verify:** `cargo nextest run -p storage site_config` +
`cargo nextest run -p storage auth` (the moved policy tests) PASS both backends;
`cargo nextest run -p jaunder --test integration web_auth` (registration-policy
paths); wasm-clippy green. Grep: `rg 'load_registration_policy'` empty.
**Commit:**
`refactor(common,storage,web): RegistrationPolicy via strum + typed config accessor (#607)`

## Task 6 — Delete the StrEnum macro

**Files:** `macros/src/str_enum.rs` (delete), `macros/src/lib.rs` (remove
`mod str_enum;`, the `#[proc_macro_derive(StrEnum, …)]` fn + its rustdoc, and
all `str_enum_*` in-crate tests), `macros/tests/str_enum.rs` (delete).

- Precondition: tasks 1–5 landed, so no `#[derive(StrEnum)]` remains.

**Verify:** `cargo nextest run -p macros` PASS (remaining macro tests);
`cargo xtask check` green; **`rg 'StrEnum' -g
'!docs/**'`empty**. **Commit:**`refactor(macros): delete the retired StrEnum
derive macro (#607)`

## Task 7 — Fix ADR-0075 prose

**Files:** `docs/adr/0075-adopt-strum-retire-str-enum.md`.

- Reword the ~:8 line calling ADR-0074 "still `Status: proposed`" to reflect its
  current `superseded` status (leave the factually-true "never reached
  accepted").

**Verify:** `cargo xtask check` green (adr-format + prettier). **Commit:**
`docs(adr): correct ADR-0075's stale reference to ADR-0074 status (#607)`

---

## Final conformance gate (before ship)

- `cargo xtask validate --no-e2e` green (verify-only; coverage Nix build).
- `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings`
  green.
- `rg 'StrEnum' -g '!docs/**'` empty; `rg 'load_registration_policy'` empty.
- Every AC observable satisfied; `git diff wt-base-issue-607..HEAD` shows only
  the strum migrations, typed bind/decode swaps, the config accessor, the macro
  deletion, and the ADR prose line — all representation-compatible.
