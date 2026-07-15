# Plan — Issue #454: type the `mode` wire arg as `BackupMode` + enum-driven select

**Spec:**
[`docs/superpowers/specs/2026-07-15-issue-454-backup-mode-wire-arg.md`](../specs/2026-07-15-issue-454-backup-mode-wire-arg.md)
**Issue:** [#454](https://github.com/jaunder-org/jaunder/issues/454) ·
**Branch:** `worktree-issue-454-backup-mode-wire-arg` · **Fork tag:**
`wt-base-issue-454` **For agentic workers:** drive with `jaunder-iterate`;
delegate a task via `jaunder-dispatch` when useful. Commit through
`jaunder-commit` (full `cargo xtask check` in the pre-commit hook). **No
`Co-Authored-By` trailer.**

## Review header

**Goal:** Make `BackupMode` the single source of truth for its variants + string
forms via `strum`, type the `update_backup_settings` `mode` wire arg as
`BackupMode` (ADR-0065), generate the admin `<select>` from the enum, and retire
the duplicated string helpers.

**Scope**

- **In:** `strum` workspace dep (+ `common`, `web`); `BackupMode` strum
  derives + `label()`; typed `mode` wire arg; enum-driven mode `<select>`;
  retire `storage::{parse_backup_mode, backup_mode_str}`; the server-fn test
  move.
- **Out:** `retention_count` wire arg (follow-on #455); the `server::cli`
  `CliBackupMode` clap enum; any newtype/ADR.

**Tasks**

1. `strum` dep (root `[workspace.dependencies]` + `common` + `web`) and
   `BackupMode` strum derives + `label()` + agreement test — in `common`.
2. `storage/src/site_config.rs` — delete `parse_backup_mode`/`backup_mode_str`;
   read via `str::parse`, write via `as_ref()`.
3. `web/src/backup/mod.rs` — type `mode: BackupMode`; drop the in-body match;
   move the import unconditional.
4. `web/src/pages/backup.rs` — generate the mode `<select>` from
   `BackupMode::VARIANTS`; delete `mode_str`; collapse the `cov:ignore` block.
5. `server/tests/web/web_backup.rs` — move the `invalid_mode` case to the
   non-`Ok` test.
6. Full `cargo xtask validate` (incl. e2e) green.

**Key risks / decisions**

- Confirm the latest `strum` 0.x and that **cargo-deny** accepts the new
  transitive deps (`strum_macros`, `rustversion`, `heck`) — a `deny.toml` tweak
  may be needed; a real risk since deps are gated.
- `strum` must compile for `web`'s wasm target too (`common`/`web` are
  dual-target). The derives are pure; `strum` default features are wasm-safe —
  add with `features = ["derive"]` only.
- serde `rename_all` and strum `serialize_all` are both `snake_case` of the same
  variant names → agree by construction; the Task 1 test locks it so a future
  divergence fails loudly.
- The mode `<select>` posts the same `directory`/`archive` values → the
  happy-path server/e2e tests are unaffected; only the invalid-mode rejection
  path changes (deserialization, not in-body).

## Global constraints

- Backend parity (ADR-0019): storage/server tests stay dual-backend
  (`backends`/`backends_matrix`); no dialect files touched.
- Coverage (ADR-0050): `backup_settings_form` is `cov:ignore` and stays so;
  `BackupMode`'s logic-bearing bits (`label`, the strum-derived impls) are
  host-tested in `common`.
- Import discipline; run the gate worktree-aware
  (`devtool run -- cargo xtask check`).

---

## Task 1 — `strum` dep + `BackupMode` single-sourced (in `common`)

**Files**

- Root `Cargo.toml` (`[workspace.dependencies]`), `common/Cargo.toml`,
  `web/Cargo.toml`, `common/src/backup.rs`.

**Interfaces**

- Root `Cargo.toml` `[workspace.dependencies]`: add
  `strum = { version = "0.27", features = ["derive"] }` (confirm the latest 0.x
  via `cargo add`/registry; adjust if 0.27 isn't current).
- `common/Cargo.toml` `[dependencies]`: add `strum.workspace = true`.
- `web/Cargo.toml` `[dependencies]`: add `strum.workspace = true` (for
  `use strum::VariantArray;` at the select).
- `common/src/backup.rs` — extend the `BackupMode` derive list and add
  `label()`:

```rust
/// How a backup is written to its destination.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize,
    strum::VariantArray, strum::AsRefStr, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BackupMode {
    /// An expanded directory (…). The default.
    #[default]
    Directory,
    /// A single `backup-<timestamp>.tar.gz` archive (…).
    Archive,
}

impl BackupMode {
    /// Human-facing label for the admin `<select>` (distinct from the wire token).
    /// Exhaustive match → the compiler forces a new variant to be handled here.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            BackupMode::Directory => "Directory",
            BackupMode::Archive => "Archive",
        }
    }
}
```

(Keep the existing doc comments on the variants.)

**Test** (`common/src/backup.rs`, in-file `#[cfg(test)]`):

```rust
#[test]
fn backup_mode_string_forms_agree_across_strum_serde_and_parse() {
    use strum::VariantArray;
    for &m in BackupMode::VARIANTS {
        // strum wire token == serde's serialized token (single snake_case rule).
        let serde_token = serde_json::to_value(m).unwrap();
        assert_eq!(serde_token, serde_json::Value::String(m.as_ref().to_owned()));
        // Parse round-trips through the same token.
        assert_eq!(m.as_ref().parse::<BackupMode>().ok(), Some(m));
        // Label is present and non-empty.
        assert!(!m.label().is_empty());
    }
    // An unknown token is rejected.
    assert!("floppy".parse::<BackupMode>().is_err());
    // Sanity: the two known variants are enumerated.
    assert_eq!(BackupMode::VARIANTS.len(), 2);
}
```

**Run**

- `cargo nextest run -p common backup_mode` → PASS.
- `cargo tree -p common -i strum` (sanity: strum resolves) — optional.

## Task 2 — `storage/src/site_config.rs`: retire the helpers

**Files**

- `storage/src/site_config.rs`.

**Interfaces**

- Read (in `get_backup_config`): `.as_deref().and_then(parse_backup_mode)` →
  `.as_deref().and_then(|v| v.trim().parse::<BackupMode>().ok())`.
- Write (in `set_backup_config`):
  `self.set(BACKUP_MODE_KEY, backup_mode_str(config.mode))` →
  `self.set(BACKUP_MODE_KEY, config.mode.as_ref())`.
- Delete the `parse_backup_mode` and `backup_mode_str` free fns.
- `BackupMode` is already imported; no strum import needed here
  (`parse`/`as_ref` are std traits).

**Test**

- Existing dual-backend tests unchanged and must still pass — especially
  `get_backup_config_ignores_invalid_stored_values` (`backup.mode = "floppy"` →
  `Default`) and `set_and_get_backup_config_round_trips`.

**Run**

- `cargo nextest run -p storage site_config` → PASS (dual-backend).

## Task 3 — `web/src/backup/mod.rs`: typed `mode` arg

**Files**

- `web/src/backup/mod.rs`.

**Interfaces**

- Signature: `mode: String` → `mode: BackupMode` (keep the other three args).
- Delete the
  `let mode = match mode.trim() { "directory" => … "archive" => … _ => Err(validation) };`
  block; use `mode` directly in the `BackupConfig`.
- Move `BackupMode` from the `#[cfg(feature = "server")]`-only `use` group to
  the unconditional import beside `BackupSchedule`:
  `use common::backup::{BackupConfig, BackupMode, BackupSchedule};`, and extend
  the existing unconditional-import comment to name both (typed `#[server]`
  args).
- The `instrument(skip(...))` list keeps `mode`.

**Test**

- Covered by Task 5's server-fn tests (happy path + invalid-mode rejection).

**Run**

- Compiles under Task 5's run.

## Task 4 — `web/src/pages/backup.rs`: enum-driven `<select>`

**Files**

- `web/src/pages/backup.rs`.

**Interfaces**

- Add `use strum::VariantArray;` (module imports) so `BackupMode::VARIANTS`
  resolves.
- Delete the `let mode_str = match settings.mode { … };` binding (currently
  lines ~52–56) **and** the redundant mid-function `// cov:ignore-stop` /
  `// cov:ignore-start` around it — leaving the single `// cov:ignore-start` at
  the fn top and `// cov:ignore-stop` after the fn, so the whole helper is one
  ignore block.
- Replace the hardcoded mode `<select>`:

```rust
<label class="j-backup-field">
    <span class="j-edit-form-label">"Mode"</span>
    <select class="j-backup-input" name="mode">
        {BackupMode::VARIANTS
            .iter()
            .copied()
            .map(|m| view! {
                <option value=m.as_ref() selected=m == settings.mode>{m.label()}</option>
            })
            .collect_view()}
    </select>
</label>
```

(`settings.mode` is `Copy`; used after other `settings` fields move — fine.)

**Test**

- No host unit test (form helper is `cov:ignore`); covered by e2e in Task 6 (the
  admin flow selects a mode).

**Run**

- `cargo nextest run -p web` → PASS (host build; leptosfmt may reformat the
  generic-free `<select>` block — the fast gate auto-fixes).

## Task 5 — `server/tests/web/web_backup.rs`: move the invalid-mode case

**Files**

- `server/tests/web/web_backup.rs`.

**Interfaces**

- Remove the `#[case::invalid_mode(…"mode=surprise"…, "backup mode")]` from
  `operator_update_backup_settings_rejects_invalid_input` (leaving only
  `invalid_retention_count`, which keeps its 500 + message contract).
- Add a `mode` case to the non-`Ok` test and rename it
  `operator_update_backup_settings_rejects_invalid_schedule` →
  `…_rejects_invalid_schedule_or_mode`, with a
  `#[case::invalid_mode("destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=5&mode=surprise")]`
  asserting `assert_ne!(status, StatusCode::OK)`.
- Update the test's doc comment to note both schedule and mode are now typed
  wire args rejected at deserialization.
- Happy-path (`operator_can_update_backup_settings*`, `operator_gets_*`)
  unchanged — they post `mode=directory`/`archive`, which now deserialize into
  `BackupMode`.

**Run**

- `cargo nextest run -p server web_backup` → PASS (dual-backend).

## Task 6 — Full gate

- `devtool run -- cargo xtask validate` (static + clippy + coverage + e2e).
  Foreground with a long timeout; if it exceeds the tool cap, gate
  `cargo xtask check` locally + the backup e2e via
  `cargo xtask e2e-local backup.spec.ts`, and let CI run the full matrix (as in
  #453).
- Confirm `xtask-done: … ok=true` + coverage clean.

## Self-review

- Every task compiles before its commit — but Tasks 1→5 are interdependent
  (removing the helpers/hardcoded strings breaks compilation until consumers
  update), so land them as one coherent commit (or 1 = common+dep, then 2–5
  together) rather than per-task commits that don't build.
- No placeholders; each interface is complete Rust.
- `strum` is the one new dep — cargo-deny is the gate to watch (Task 1 run
  surfaces it).
- Scope stays off `retention_count` (#455) and the clap `CliBackupMode`.
