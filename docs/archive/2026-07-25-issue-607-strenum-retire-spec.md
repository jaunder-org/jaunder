# Spec — #607: Retire StrEnum (migrate remaining enums to strum + delete the macro)

Issue: jaunder-org/jaunder#607 · Labels: `tooling`, `dx` · Decision: ADR-0075
(supersedes ADR-0074). Precedent: `PostFormat` (#572), `BackupMode`.

## Problem

`StrEnum` (bespoke `#[derive(StrEnum)]`, ~300 lines in `macros/`) is being
retired in favor of `strum` (already a dependency; `parse_err_ty`/`parse_err_fn`
in 0.28 supply the named parse error that was `StrEnum`'s last unique edge).
`PostFormat` migrated first (#572). This issue completes the retirement: migrate
the remaining enums and delete the macro.

## Current surface (audited 2026-07-25)

Six enums still `#[derive(StrEnum)]` — the issue body says four, but ADR-0075
and `rg 'StrEnum'` confirm **six** (`PostFormat` already done). Acceptance is
`rg 'StrEnum'` empty, so all six migrate:

| Enum                 | File                       | serde? | DB / config crossing (today)                                                          |
| -------------------- | -------------------------- | ------ | ------------------------------------------------------------------------------------- |
| `Channel`            | common/src/visibility.rs   | no     | none                                                                                  |
| `SubscriptionStatus` | common/src/visibility.rs   | no     | none (field only, no bind/decode)                                                     |
| `TargetKind`         | common/src/visibility.rs   | no     | **DB column**, stringly `.as_str()`/`try_from` (storage/src/posts.rs)                 |
| `AudienceBase`       | common/src/visibility.rs   | yes    | none (travels in the audience DTO)                                                    |
| `MediaSource`        | common/src/media.rs        | yes    | **DB column**, stringly `.as_str()`/`.parse()` (storage/src/{media,helpers}.rs)       |
| `RegistrationPolicy` | common/src/registration.rs | yes    | **site_config value**, read via free fn `load_registration_policy` (`.get().parse()`) |

## Decisions (from the design interview)

1. **Every enum gets a named error type with a specific message** — not just
   `MediaSource`. Each follows the `PostFormat` shape (`strum` `parse_err_ty` +
   `parse_err_fn` + a `thiserror` struct with a written message).
2. **No stringly values.** DB-column enums (`TargetKind`, `MediaSource`) adopt
   `impl_text_column_enum!` for typed `.bind`/decode. The config enum
   (`RegistrationPolicy`) gets a typed accessor **method** on
   `SiteConfigStorage`, the way every other config value is read
   (`get_backup_config`, `get_default_audience`, …) — no free-function
   `.get().parse()` outlier.
3. **`PostFormat` shape throughout** (named error + serde
   `into/try_from="String"` where serde is derived + `impl_text_column_enum!`
   where DB-stored).
4. **Fix the ADR-0075 prose** (its body still calls ADR-0074 "still proposed";
   0074's header is now `superseded`).

**Judgment call for your review (AC-4):** `RegistrationPolicy` follows the
safe-default accessor pattern of `BackupMode`/`get_backup_config`
(`.parse().ok().unwrap_or(Closed)` — absent/garbage → `Closed`, the deliberate
"never accidentally open registration" default), **not** the reject-on-garbage
sqlx-bridge pattern of `get_smtp_credentials` (a secret has no safe default;
registration policy does). Same "typed accessor" principle, correct pattern for
a value with a safe default.

## Acceptance criteria

Umbrella: **representation-compatible** — identical wire tokens and identical
stored DB tokens for every enum; existing tests green;
`cargo xtask validate --no-e2e` + wasm-clippy green; no new `cov:ignore`.

### AC-1 — Six enums migrated to the strum stack

Each of `Channel`, `SubscriptionStatus`, `TargetKind`, `AudienceBase`,
`MediaSource`, `RegistrationPolicy`:

- Uses the minimal `strum` derives (`AsRefStr`, `Display`, `EnumString`; plus
  `IntoStaticStr` only where an FK-name bind needs a `&'static str` — the two
  FK-normalized enums; no `VariantArray`/`EnumMessage`, unused here) with
  `#[strum(serialize_all = "snake_case")]` — producing the **same tokens** as
  today (`local`, `active`/`pending`/`blocked`, `public`/`subscribers`/ `named`,
  `private`/`public`/`subscribers`, `upload`/`cached`, `open`/`invite_only`/
  `closed`). `AudienceBase` keeps `#[default] Private`.
- Carries a **named `thiserror` error** (`InvalidChannel`,
  `InvalidSubscriptionStatus`, `InvalidTargetKind`, `InvalidAudienceBase`,
  `InvalidMediaSource`, `InvalidRegistrationPolicy`) wired via
  `#[strum(parse_err_ty = …, parse_err_fn = …)]`, each with a **specific
  message** listing the valid tokens (`MediaSource` keeps its existing
  `media source must be "upload" or "cached"`).
- `Display`/`FromStr`/`TryFrom<&str>` call sites keep compiling (strum
  `Display`/`EnumString`). **`StrEnum` gave an inherent `as_str()`; strum gives
  `as_ref()`** — so every `<enum>.as_str()` call site migrates to `.as_ref()`
  (cold-review finding; the migrating task `rg`s `<Enum>` `.as_str(` and fixes
  each, incl. sites outside `common`: `web/src/posts/component.rs` for
  `AudienceBase`; `server/src/media.rs`, `web/src/media/api.rs` for
  `MediaSource`).
- `#[derive(StrEnum)]` and the `use macros::StrEnum;` imports are removed (keep
  the other macro imports in `media.rs`).

Observable: each enum's existing unit tests pass unchanged; a serde/`FromStr`
round-trip test asserts the tokens; `rg 'StrEnum' common/src` empty.

### AC-2 — serde preserved through the named error

`AudienceBase`, `MediaSource`, `RegistrationPolicy` derive
`serde::{Serialize, Deserialize}` with
`#[serde(into = "String", try_from = "String")]` (routing deserialize through
`FromStr` → the named error), plus `From<T> for String` / `TryFrom<String>`.
Wire tokens unchanged; a `serde` round-trip and a `serde_qs` form round-trip
(where the enum crosses form transport — the media DTO, the audience DTO) pass.

> **Intentional deviation from the issue text:** issue #607 suggests the leaner
> `BackupMode` shape (serde `rename_all`). We use `PostFormat`'s `into/try_from`
> proxy instead so a bad-wire value deserializes into the enum's **named error
> message** (decision: every enum gets a specific error) rather than serde's
> generic "unknown variant". Wire-token-identical; costs one `From`/`TryFrom`
> pair per serde enum.

### AC-3 — `MediaSource` uses `impl_text_column_enum!` (typed TEXT column)

- **`MediaSource` only.** It is stored AS its text token in a TEXT column, so it
  invokes `crate::db_enum::impl_text_column_enum!(MediaSource)` and its storage
  bind/decode sites switch from `.bind(x.as_str())` / `col.parse()` to typed
  `.bind(*x)` / typed column decode:
  `storage/src/{media.rs, sqlite/media.rs, postgres/media.rs}` binds and
  `storage/src/helpers.rs::media_record_from_row` decode. Stored token is
  **byte-identical** (`impl_text_column_enum!` encodes `as_ref().to_owned()`);
  its decode already rejects an unknown token (`ColumnDecode`), preserved.
- **`TargetKind` does NOT adopt `impl_text_column_enum!`** — a cold-review
  correction. It is **not** a text-token column: it is a normalized FK, stored
  via `(SELECT kind_id FROM target_kinds WHERE name = ?)` and read back through
  a join + `filter_map` that **drops** an unknown kind (asserted at
  `storage/src/posts.rs` `"bogus" => None`). A typed column decode would (a)
  turn that drop into a whole-fetch `ColumnDecode` error — a behavior change —
  and (b) mislabel an FK as a text token. So `TargetKind` migrates to strum for
  its _definition_ only; its storage bind stays the lookup-name string
  (`.as_ref()`), and its `String` + `filter_map` decode is unchanged
  (drop-unknown preserved).

Observable: the dual-backend storage tests for posts/media pass **unchanged**
(incl. the `TargetKind` drop-unknown test); no `.as_str()`/`.parse()` strip
remains at the `MediaSource` column boundary (`rg` the sites).

### AC-4 — `RegistrationPolicy` read via a typed config accessor

- Add
  `SiteConfigStorage::get_registration_policy(&self) -> sqlx::Result<RegistrationPolicy>`
  as a default trait method (sibling of `get_backup_config`):
  `self.get(KEY).await? .as_deref().and_then(|v| v.parse().ok()).unwrap_or(RegistrationPolicy::Closed)`.
  Introduce `SITE_REGISTRATION_POLICY_KEY` const (replacing the inline
  `"site.registration_policy"` literals in storage/production).
- Remove the free function `storage::load_registration_policy`; migrate callers
  (`web/src/registration/api.rs`, `web/src/invites/api.rs`) to the method; move
  its tests to the site_config test module.
- **Preserved:** absent value or unparseable value →
  `RegistrationPolicy::Closed`.
- **Deliberate change on the DB-error path:** the old free fn swallowed _even a
  genuine sqlx read error_ into `Closed` (`.get(...).await.ok().flatten()`). The
  method returns `sqlx::Result` like its siblings, so a real DB error now
  **propagates** (callers `?` it → a 500, not a silent `Closed`). This matches
  `get_backup_config` and is more correct; only the value-absent/unparseable
  cases default to `Closed`.
- Out of scope: the generic `jaunder site-config set <key> <value>` CLI path and
  the init default-seed literal stay string-based — they are the KV substrate
  itself, not a typed domain write (no typed setter added; matches that no
  `set_*` exists for a read-only-on-the-wire policy).

Observable: `rg 'load_registration_policy'` empty; the new method's dual-backend
test asserts open/invite_only/closed round-trip + garbage→Closed.

### AC-5 — host validation registration preserved (MediaSource only)

`InvalidMediaSource` stays registered in `host/src/error.rs`
(`validation_from!` + its `check!` assertion), with the same custom message. No
new host registrations for the other five (they weren't registered before; their
errors surface at DB-decode / wire-deserialize, not user form validation — see
Non-goals).

### AC-6 — StrEnum macro deleted

- Delete `macros/src/str_enum.rs`, its `mod str_enum;` +
  `#[proc_macro_derive(StrEnum…)]`
  - rustdoc in `macros/src/lib.rs`, all `str_enum_*` in-crate tests
    (`macros/src/lib.rs`), and `macros/tests/str_enum.rs`.
- `rg 'StrEnum' -g '!docs/**'` is **empty** across the code tree. (The ADRs
  0074/0075 and `docs/README.md`'s ADR index legitimately name it as the retired
  macro — those stay; the check is code-scoped, not repo-wide.)

### AC-7 — ADR-0075 prose corrected

Fix the genuinely-stale line in `docs/adr/0075-adopt-strum-retire-str-enum.md`
(~:8) that calls ADR-0074 "still `Status: proposed`" — 0074's header is now
`superseded`. (The line saying 0074 "never reached `accepted`" is factually true
— proposed→superseded — so leave it.) Front-matter and the README table are
already correct; prose only.

## Non-goals

- **No new bespoke macro** to shrink the per-enum `thiserror` + `parse_err_fn`
  boilerplate (that recreates what's being retired — ADR-0075). Reconsider a
  tiny shared helper only if the finished migration shows the pattern genuinely
  grating, with data.
- **No new host `validation_from!` registrations** beyond the existing
  MediaSource one, and **no behavior change** to which errors are user-facing.
- **No typed setter** / no rewrite of the generic `site-config set` CLI or the
  init seed literal.
- `RegistrationPolicy` does **not** adopt the strict reject-on-garbage
  sqlx-bridge (it keeps its safe `Closed` default).

## Verification

- Per enum: existing unit tests + a serde/`FromStr` round-trip (+ `serde_qs`
  form round-trip for the serde enums that cross form transport).
- `TargetKind`/`MediaSource`: dual-backend storage tests (posts/media) green.
- `RegistrationPolicy`: dual-backend `get_registration_policy` test.
- `cargo xtask validate --no-e2e` +
  `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings`
  green.
- `rg 'StrEnum' -g '!docs/**'` empty (code tree);
  `rg 'load_registration_policy'` empty.
