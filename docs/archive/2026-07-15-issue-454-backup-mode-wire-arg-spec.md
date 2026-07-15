# Spec — Issue #454: type the `mode` wire arg as `BackupMode` + enum-driven select

- **Issue:** [#454](https://github.com/jaunder-org/jaunder/issues/454) (Task,
  milestone _Domain-value type safety (newtypes)_, P3)
- **Branch:** `worktree-issue-454-backup-mode-wire-arg` · fork tag
  `wt-base-issue-454`
- **Family:** the `mode` sibling of #453's schedule wire-arg change.

## Goal

Type `update_backup_settings`'s `mode` wire arg as `BackupMode` (the issue's
core), **and** make `BackupMode` the single source of truth for its variants and
string forms so the admin `<select>` is generated from the enum rather than
hardcoded. Today the `directory`/`archive` mapping is duplicated four ways —
serde `rename_all`, `storage::{parse_backup_mode, backup_mode_str}`, the web
server-fn match (being deleted), and `pages/backup.rs`'s hardcoded `<option>`s +
`mode_str` match. A new variant would need edits in all of them.

## Design

### 0. Add `strum` as a workspace dependency

Enum-driven UI/wire mappings will recur (this is the first of several), so adopt
`strum`'s derives rather than hand-rolling per enum — it removes the "forget to
append to `ALL`" footgun for good and establishes a reusable pattern. Add
`strum = { version = "0.27", features = ["derive"] }` to
`[workspace.dependencies]` in the root `Cargo.toml`, then
`strum.workspace = true` to:

- `common/Cargo.toml` — for the derives on `BackupMode`.
- `web/Cargo.toml` — the `<select>` reads `BackupMode::VARIANTS`, whose access
  needs the `strum::VariantArray` trait in scope (`use strum::VariantArray;`).

`storage` needs **no** strum dependency — it uses only `str::parse` (the
`EnumString`-derived `FromStr`) and `AsRef<str>`, both std traits.
(MIT/Apache-2.0 — cargo-deny clean; confirm the exact latest 0.x and that the
gate accepts the new transitive `strum_macros`/`rustversion` at implementation.)

### 1. `common/src/backup.rs` — `BackupMode` owns its variants + string forms via strum

`BackupMode` keeps its derives and `#[serde(rename_all = "snake_case")]`, and
gains strum derives whose `serialize_all = "snake_case"` mirrors serde by the
**same rule applied to the same variant names** — so the wire token has a single
behavioral source (snake_case of the variant), not a hand-maintained string
list:

```rust
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize,
    strum::VariantArray, strum::AsRefStr, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BackupMode {
    #[default]
    Directory,
    Archive,
}

impl BackupMode {
    /// Human-facing label for the admin UI (distinct from the wire token). Exhaustive match
    /// → the compiler forces a new variant to be handled here.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            BackupMode::Directory => "Directory",
            BackupMode::Archive => "Archive",
        }
    }
}
```

This yields, with no hand-written variant/string list:

- **Enumeration** — `BackupMode::VARIANTS` (`&'static [BackupMode]`, from
  `VariantArray`) for the select.
- **Wire token** — `m.as_ref()` (`AsRefStr`) → `"directory"`/`"archive"`
  (`&'static str`, no alloc), matching serde.
- **Parse** — `"directory".parse::<BackupMode>()` (`EnumString` → `FromStr`),
  replacing `parse_backup_mode`.
- **Label** — the one genuinely hand-authored, compiler-guarded bit.

A unit test asserts, for every `VARIANTS` entry, that `m.as_ref()` equals
serde's serialized token and that `m.as_ref().parse() == Ok(m)` — locking the
strum⇄serde⇄parse agreement.

### 2. Wire arg — `web/src/backup/mod.rs`

- Signature `mode: String` → `mode: BackupMode`; delete the in-body
  `match mode.trim()` block (the generated `Deserialize` rejects a non-variant
  on the wire).
- Move `BackupMode` to the unconditional import next to `BackupSchedule` (typed
  `#[server]` arg → needed on both sides); extend the existing
  unconditional-import comment.

### 3. Client select — `web/src/pages/backup.rs`

- Delete the `mode_str` match.
- Generate the options from `BackupMode::ALL`, following the `profile.rs`
  per-option `selected` idiom:

```rust
<select class="j-backup-input" name="mode">
    {BackupMode::VARIANTS
        .iter()
        .copied()
        .map(|m| view! {
            <option value=m.as_ref() selected=m == settings.mode>{m.label()}</option>
        })
        .collect_view()}
</select>
```

(`BackupMode` is `Copy + PartialEq`; `settings.mode` is used by copy. `VARIANTS`
needs `strum::VariantArray` in scope — a `use strum::VariantArray;` in the page
module.)

### 4. `storage/src/site_config.rs` — retire the duplicate helpers

- Delete `parse_backup_mode` and `backup_mode_str`.
- Read: `.and_then(parse_backup_mode)` →
  `.and_then(|v| v.trim().parse::<BackupMode>().ok())`.
- Write: `backup_mode_str(config.mode)` → `config.mode.as_ref()`.

## Non-goals

- `retention_count` stays a bare `String` wire arg (follow-on #455).
- No newtype (BackupMode is already an enum); no ADR (applies ADR-0065, no new
  decision).
- The `server::cli` `CliBackupMode → BackupMode` conversion is a separate clap
  enum, untouched.

## Tests

- **`common/src/backup.rs`:** for every `BackupMode::VARIANTS` entry —
  `m.as_ref()` equals serde's serialized token (**strum⇄serde agreement**) and
  `m.as_ref().parse() == Ok(m)` (round-trip); an unknown token fails to
  `parse::<BackupMode>()`; `label()` per variant.
- **`server/tests/web/web_backup.rs`:** move the `invalid_mode` case
  (`mode=surprise`) out of
  `operator_update_backup_settings_rejects_invalid_input` (it now fails at
  deserialization, not in-body) into the non-`Ok` test alongside the schedule
  cases (`operator_update_backup_settings_rejects_invalid_schedule` → rename to
  `…_rejects_invalid_schedule_or_mode`), asserting `assert_ne!(status, OK)`. The
  remaining `invalid_retention_count` case keeps its 500 + message contract.
  Happy-path (`operator_can_update_backup_settings*`) unchanged.
- **`storage/src/site_config.rs`:** existing tests unchanged (behavior
  preserved: invalid stored mode → default).

## Verify

- `cargo xtask validate --no-e2e` clean (static + clippy + coverage + tests incl
  PostgreSQL); affected server/storage tests stay dual-backend
  (`backends_matrix`/`backends`).
- e2e: the admin backup flow's mode `<select>` is now enum-generated but posts
  the same values; the existing suite covers it. Full `validate` (with e2e) at
  ship.
