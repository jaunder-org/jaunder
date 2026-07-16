# Plan — Issue #455: `RetentionCount` newtype (min 1) for the retention wire arg

**Spec:**
[`docs/superpowers/specs/2026-07-15-issue-455-retention-count-wire-arg.md`](../specs/2026-07-15-issue-455-retention-count-wire-arg.md)
**Issue:** [#455](https://github.com/jaunder-org/jaunder/issues/455) ·
**Branch:** `worktree-issue-455-retention-count-wire-arg` · **Fork tag:**
`wt-base-issue-455` **For agentic workers:** drive with `jaunder-iterate`;
delegate a task via `jaunder-dispatch` when useful. Commit through
`jaunder-commit` (full `cargo xtask check` in the pre-commit hook). **No
`Co-Authored-By` trailer.**

## Review header

**Goal:** Type `update_backup_settings`'s `retention_count` as a
`RetentionCount(NonZeroUsize)` newtype (min-1), making `retention_count == 0`
unrepresentable — which fixes the `prune_backups` footgun (0 prunes every
backup) — with client `ValidatedInput` validation (ADR-0065). The last of the
#453 split.

**Scope**

- **In:** `RetentionCount` newtype + `parse_retention_count` test helper;
  `BackupConfig.retention_count` field type; storage read/write; `prune_backups`
  call site; typed wire arg; client `ValidatedInput`; server-test restructure;
  e2e.
- **Out:** typing `prune_backups`' param (config guarantees `>= 1`); upper cap;
  a reusable value-newtype macro (deferred → **#464**, already filed, P4,
  blocked-by #455); no ADR.

**Tasks**

1. Follow-on **#464** (value-newtype macro) — **already filed**. No action.
2. `common`: `RetentionCount` newtype + `InvalidRetentionCount` + tests;
   `parse_retention_count` in `test_support`;
   `BackupConfig.retention_count: RetentionCount` + `Default`.
3. `storage/src/site_config.rs`: read via `.parse::<RetentionCount>()`, write
   via `Display`; tests via helper; stored-`"0"`→default.
4. `server/src/backup.rs`: `prune_backups(…, config.retention_count.get())`;
   test configs via helper.
5. `web/src/backup/mod.rs`: type the wire arg; drop in-body parse; import
   unconditional.
6. `web/src/pages/backup.rs`: `Field`/`ValidatedInput<RetentionCount>`; gate
   submit on schedule + retention.
7. `server/tests/web/web_backup.rs`: move retention to the non-`Ok` test (+ a
   `retention_count=0` case); delete the now-empty message test; `.get()` in
   get-assertions.
8. `end2end/tests/backup.spec.ts`: retention-gating case.
9. Full `cargo xtask validate` (incl. e2e) green.

**Key risks / decisions**

- Numeric `#[server]` arg deserialization from the form is a trodden path here
  (`post_id: i64`, `page_size: u32`) — but verify the typed-arg reject path
  early (Task 7 exercises `retention_count=bogus`/`=0`).
- `BackupConfig.retention_count` type change ripples to **every** `BackupConfig`
  literal — sweep all `cfg(test)` sites (storage, server) via
  `parse_retention_count` (the newtype test-helper convention).
- Wire (serde → `NonZeroUsize`) and client (`FromStr`) both delegate the `>= 1`
  rule to `NonZeroUsize` — one source, not re-implemented (#416-clean); a test
  asserts both reject `0`.

## Global constraints

- Backend parity (ADR-0019): storage/server tests stay dual-backend
  (`backends`/`backends_matrix`); no dialect files.
- Coverage (ADR-0050): `RetentionCount`'s logic (`FromStr`, `Default`, `get`,
  `Display`, error) is host-tested in `common`; `backup_settings_form` stays
  `cov:ignore`.
- Import discipline; run the gate worktree-aware
  (`devtool run -- cargo xtask check`).

---

## Task 2 — `common`: the `RetentionCount` newtype

**Files**

- `common/src/backup.rs` (type + error + `BackupConfig` field + `Default` +
  tests).
- `common/src/test_support.rs` (`parse_retention_count`).

**Interfaces** — add to `common/src/backup.rs` (imports:
`use std::num::NonZeroUsize;`, `use std::fmt;`; `FromStr`, `thiserror::Error`,
serde are already in scope):

```rust
/// The number of most-recent backups to keep. Always >= 1, so retention pruning
/// (`server::backup::prune_backups`) can never remove every backup — including one just
/// created. Constructed via `FromStr`/serde (both reject 0 through `NonZeroUsize`) or `Default`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetentionCount(NonZeroUsize);

/// Error when a value is not a valid retention count (a whole number of at least 1).
#[derive(Debug, Error)]
#[error("backup retention count must be a whole number of at least 1")]
pub struct InvalidRetentionCount;

impl RetentionCount {
    /// The inner count (>= 1) for consumers needing a `usize` (e.g. pruning).
    #[must_use]
    pub fn get(self) -> usize {
        self.0.get()
    }
}

impl FromStr for RetentionCount {
    type Err = InvalidRetentionCount;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.trim()
            .parse::<NonZeroUsize>()
            .map(Self)
            .map_err(|_| InvalidRetentionCount)
    }
}

impl fmt::Display for RetentionCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for RetentionCount {
    fn default() -> Self {
        Self(NonZeroUsize::new(DEFAULT_BACKUP_RETENTION_COUNT).expect("default retention is nonzero"))
    }
}
```

- `BackupConfig`: `pub retention_count: usize` →
  `pub retention_count: RetentionCount`; `BackupConfig::default` →
  `retention_count: RetentionCount::default()`. Keep
  `DEFAULT_BACKUP_RETENTION_COUNT: usize = 7`.

**Test** (`common/src/backup.rs`, in-file):

```rust
#[test]
fn retention_count_parses_and_rejects_zero_and_non_integers() {
    assert_eq!("1".parse::<RetentionCount>().unwrap().get(), 1);
    assert_eq!("  7  ".parse::<RetentionCount>().unwrap().get(), 7);
    for bad in ["0", "", "-1", "abc", "1.5"] {
        assert!(bad.parse::<RetentionCount>().is_err(), "{bad} should be rejected");
    }
    // The domain message, not std's.
    assert!("0".parse::<RetentionCount>().unwrap_err().to_string().starts_with("backup retention count"));
}

#[test]
fn retention_count_default_is_seven_and_display_round_trips() {
    let d = RetentionCount::default();
    assert_eq!(d.get(), DEFAULT_BACKUP_RETENTION_COUNT);
    assert_eq!(d.to_string().parse::<RetentionCount>().unwrap(), d);
}

#[test]
fn retention_count_serde_rejects_zero_on_the_wire() {
    let r: RetentionCount = "5".parse().unwrap();
    assert_eq!(serde_json::to_string(&r).unwrap(), "5"); // plain integer, unchanged JSON shape
    assert_eq!(serde_json::from_str::<RetentionCount>("5").unwrap(), r);
    assert!(serde_json::from_str::<RetentionCount>("0").is_err()); // NonZero rejects on the wire
}
```

- `common/src/test_support.rs`: add beside the other `parse_*` helpers:

```rust
use crate::backup::RetentionCount;
#[must_use]
pub fn parse_retention_count(s: &str) -> RetentionCount {
    s.parse().expect("valid retention count")
}
```

(Match the exact import/attr style of `parse_email`/`parse_display_name` there.)

**Run**

- `cargo nextest run -p common retention_count` → PASS.

## Task 3 — `storage/src/site_config.rs`

**Files** — `storage/src/site_config.rs`.

**Interfaces**

- Read (in `get_backup_config`):
  `.and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(DEFAULT_BACKUP_RETENTION_COUNT)`
  → `.and_then(|v| v.parse::<RetentionCount>().ok()).unwrap_or_default()`
  (import `RetentionCount`; `FromStr` trims internally).
- Write (in `set_backup_config`): `&config.retention_count.to_string()` —
  unchanged (`RetentionCount: Display`).
- Round-trip test `retention_count: 14` → `parse_retention_count("14")` (import
  from `common::test_support`).

**Test**

- Existing dual-backend tests pass.
  `get_backup_config_ignores_invalid_stored_values` already sets
  `backup.retention_count = "daily"` → default; **add an assertion** (or a case)
  that a stored `"0"` also reads back as `Default` (7) — the footgun fix at the
  storage layer.

**Run**

- `cargo nextest run -p storage site_config` → PASS (dual-backend).

## Task 4 — `server/src/backup.rs`

**Files** — `server/src/backup.rs`.

**Interfaces**

- `prune_backups(destination_root, config.retention_count)` →
  `prune_backups(destination_root, config.retention_count.get())`.
  `prune_backups` keeps `retention_count: usize`.
- Test configs `retention_count: 1` (×2, and any `ok_config`/`bad_config`) →
  `parse_retention_count("1")` (import the helper).

**Run**

- `cargo nextest run -p server backup` → PASS.

## Task 5 — `web/src/backup/mod.rs`

**Files** — `web/src/backup/mod.rs`.

**Interfaces**

- Signature: `retention_count: String` → `retention_count: RetentionCount`;
  delete the `retention_count.trim().parse::<usize>()...` block; use
  `retention_count` directly in the `BackupConfig`.
- Add `RetentionCount` to the unconditional import
  (`use common::backup::{BackupConfig, BackupMode, BackupSchedule};` → add
  `RetentionCount`); extend the unconditional-import comment to include it.
- `instrument(skip(...))` keeps `retention_count`.

**Run** — compiles under Task 7.

## Task 6 — `web/src/pages/backup.rs`: client validation

**Files** — `web/src/pages/backup.rs`.

**Interfaces**

- Add `use common::backup::RetentionCount;` (or extend the existing
  `common::backup::{…}` import).
- In `backup_settings_form`:
  `let retention = Field::<RetentionCount>::prefilled(&settings.retention_count.to_string());`
  (alongside the `schedule` field).
- Replace the retention
  `<label>…<input type="number" min="0" name="retention_count" …/></label>`
  with:

```rust
<ValidatedInput<RetentionCount>
    label="Retention Count"
    name="retention_count"
    field=retention
    input_type="number"
    field_class="j-backup-field"
    class="j-backup-input"
/>
```

- Gate submit on both fields:
  `prop:disabled=move || !schedule.is_valid() || !retention.is_valid()`.
- Keep the whole helper in the single `cov:ignore` block.

**Run** — `cargo nextest run -p web` → PASS (host; leptosfmt may reformat the
generic tag — fast gate auto-fixes).

## Task 7 — `server/tests/web/web_backup.rs`

**Files** — `server/tests/web/web_backup.rs`.

**Interfaces**

- **Delete** `operator_update_backup_settings_rejects_invalid_input` (its only
  case, `invalid_retention_count`, now fails at deserialization; no
  in-body-validated fields remain).
- Rename `operator_update_backup_settings_rejects_invalid_schedule_or_mode` →
  `operator_update_backup_settings_rejects_invalid_typed_arg`; add cases:
  `#[case::invalid_retention_count("...&retention_count=bogus&mode=directory")]`
  and `#[case::zero_retention_count("...&retention_count=0&mode=directory")]`,
  both asserting `assert_ne!(status, StatusCode::OK)`. Update the doc comment to
  name schedule, mode, and retention as typed wire args.
- Get-assertions: `assert_eq!(settings.retention_count, 4)` →
  `assert_eq!(settings.retention_count.get(), 4)` (and the `== 7` default
  checks).

**Run** — `cargo nextest run -p server web_backup` → PASS (dual-backend).

## Task 8 — `end2end/tests/backup.spec.ts`

**Files** — `end2end/tests/backup.spec.ts`.

**Interfaces** — add a test (mirroring the schedule-gating case): load
`/admin/backups` as `testoperator`; the retention field
(`input[name="retention_count"]`) starts valid (prefilled 7 → submit enabled);
filling `0` (blur) shows the inline error and disables Save; filling `3` clears
it and re-enables Save.

**Run** — `cargo xtask e2e-local backup.spec.ts` → all backup specs PASS.

## Task 9 — Full gate

- `devtool run -- cargo xtask validate` (static + clippy + coverage + e2e).
  Foreground with a long timeout; if it exceeds the tool cap, gate
  `cargo xtask check` + the backup e2e via `e2e-local`, and let CI run the full
  matrix (as in #453/#454).

## Self-review

- Tasks 2–7 are interdependent (the `BackupConfig` field type change breaks
  compilation until every consumer threads through) — land them as one coherent
  commit; the e2e (Task 8) as a second.
- No placeholders; complete Rust throughout.
- Reuse-macro scope is filed (#464) and excluded, not silently dropped.
- Behavior change surfaced: a stored `"0"` now falls back to `Default(7)` —
  covered by a storage test.
