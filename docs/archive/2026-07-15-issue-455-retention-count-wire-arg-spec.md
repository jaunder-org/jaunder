# Spec — Issue #455: type `retention_count` as a `RetentionCount` newtype (min 1)

- **Issue:** [#455](https://github.com/jaunder-org/jaunder/issues/455) (Task,
  milestone _Domain-value type safety (newtypes)_, P3)
- **Branch:** `worktree-issue-455-retention-count-wire-arg` · fork tag
  `wt-base-issue-455`
- **Family:** the last of the #453 split (schedule #453, mode #454, retention
  #455).

## Goal

Type `update_backup_settings`'s `retention_count` wire arg (ADR-0065) **and** —
per the design decision — make it a `RetentionCount` newtype with a **min-1
invariant** rather than a bare `usize`. This fixes a real footgun:
`server::backup::prune_backups` does
`backups.len().saturating_sub(retention_count)` with no guard, so
`retention_count == 0` prunes **every** backup, including a freshly-created one.
A bare `usize` arg would preserve that; a `RetentionCount(NonZeroUsize)` makes 0
unrepresentable end-to-end.

## Design

### 0. `common/src/backup.rs` — the `RetentionCount` newtype

Back it with `NonZeroUsize` so the `>= 1` invariant is enforced by the inner
type (and serde rejects 0 on the wire for free). A hand-written `FromStr` gives
the domain error the client `ValidatedInput` surfaces; both paths delegate the
rule to `NonZeroUsize` (not re-implemented, #416-clean).

```rust
use std::num::NonZeroUsize;
use std::fmt;

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
    /// The inner count (>= 1) for consumers that need a `usize` (e.g. pruning).
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

- **Wire:** derived `Deserialize` → `NonZeroUsize` (an integer, matching the
  current `retention_count: usize` JSON number shape of the
  `get_backup_settings` response) → rejects `0` and non-integers on the wire.
  `Serialize` emits the integer.
- **Client:** `FromStr` (used by `ValidatedInput<RetentionCount>`) → the domain
  message.
- Numeric `#[server]` args already deserialize from the form encoding in this
  codebase (`post_id: i64`, `page_size: u32`), so the numeric wire arg is a
  trodden path.

### 1. `common/src/test_support.rs` — a `parse_retention_count` helper

Add
`pub fn parse_retention_count(s: &str) -> RetentionCount { s.parse().expect("valid retention count") }`
beside `parse_email`/`parse_audience_name`/`parse_display_name`, and use it at
**every** `cfg(test)` construction site (per the newtype test-helper
convention).

### 2. `BackupConfig` field + `Default`

- `common/src/backup.rs`: `pub retention_count: usize` →
  `pub retention_count: RetentionCount`; `BackupConfig::default` uses
  `RetentionCount::default()`. `DEFAULT_BACKUP_RETENTION_COUNT` (usize, 7) stays
  and backs `RetentionCount::default`.

### 3. `storage/src/site_config.rs`

- Read:
  `.and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(DEFAULT_BACKUP_RETENTION_COUNT)`
  → `.and_then(|v| v.parse::<RetentionCount>().ok()).unwrap_or_default()`
  (`FromStr` trims internally, like `BackupSchedule`). A stored `"0"` now falls
  back to `Default` (7) instead of being kept — the footgun fix at the storage
  layer too.
- Write: `&config.retention_count.to_string()` unchanged
  (`RetentionCount: Display`).
- Test `retention_count: 14` → `parse_retention_count("14")`.

### 4. `server/src/backup.rs`

- `prune_backups(destination_root, config.retention_count)` →
  `…, config.retention_count.get()`. `prune_backups` keeps its `usize` param —
  the config layer now guarantees `>= 1`, so the "prune everything" branch is
  unreachable (typing the fn param too is out of scope).
- Test configs `retention_count: 1` → `parse_retention_count("1")`.

### 5. Wire arg — `web/src/backup/mod.rs`

- `retention_count: String` → `retention_count: RetentionCount`; delete the
  in-body `retention_count.trim().parse::<usize>()` block. Move `RetentionCount`
  to the unconditional import beside `BackupSchedule`/`BackupMode` (typed
  `#[server]` arg); extend the comment.

### 6. Client field — `web/src/pages/backup.rs`

- Drive the retention field through `Field`/`ValidatedInput<RetentionCount>`
  (ADR-0065), mirroring the schedule field:
  `let retention = Field::<RetentionCount>::prefilled(&settings.retention_count.to_string());`
  and a
  `<ValidatedInput<RetentionCount> label="Retention Count" name="retention_count" input_type="number" field=retention field_class="j-backup-field" class="j-backup-input" />`,
  replacing the plain `<input type="number">`.
- Gate submit on **both** fields:
  `prop:disabled=move || !schedule.is_valid() || !retention.is_valid()`.
- (The old `min="0"` HTML hint is dropped; the newtype validation enforces
  `>= 1` with an inline message — no `min` prop is added to `ValidatedInput`,
  keeping its surface stable.)

### 7. Server integration test — `server/tests/web/web_backup.rs`

- The `invalid_retention_count` case now fails at request deserialization (the
  arg is typed), not in-body — so move it to the non-`Ok` test. After the move,
  `operator_update_backup_settings_rejects_invalid_input` (the message-based
  test #454 left with only that case) has **no cases** and is **deleted** — no
  in-body-validated fields remain (`destination_path` uses `non_empty`, which
  never errors).
- Rename `operator_update_backup_settings_rejects_invalid_schedule_or_mode` →
  `…_rejects_invalid_typed_arg`, adding retention cases: `retention_count=bogus`
  **and** `retention_count=0` (the newtype's key win — `0` is rejected on the
  wire). All assert `assert_ne!(status, StatusCode::OK)`.
- Get-tests comparing `settings.retention_count` to an integer use `.get()`
  (`assert_eq!(settings.retention_count.get(), 4)`).

## Non-goals

- Typing `prune_backups`' param as `RetentionCount` (config now guarantees
  `>= 1`; internal polish, out of scope).
- An upper cap on the count (min-1 only, per the decision).
- No ADR (applies ADR-0065; no new decision).
- **No reusable value-newtype macro yet** (decided). `RetentionCount` is the
  first validated integer _value_ newtype (IDs already use `IdNewtype`). The
  other candidates — `feeds.min_items`/`min_days`, media quotas, `page_size` —
  aren't form-typed today and carry differing invariants (allow-0 vs min-1,
  `u32`/`i64`), so a macro designed from this single case risks the wrong shape.
  Build `RetentionCount` as a focused one-off; **the plan's first task files a
  follow-up issue** to extract a home-grown numeric-value-newtype macro (a
  sibling to `IdNewtype`/`StrNewtype`) once a second/third case gives real
  invariants to design against.

## Tests

- **`common/src/backup.rs`:** `RetentionCount` `FromStr` accepts `"1"`/`"7"`,
  rejects `"0"`, `""`, `"-1"`, `"abc"` (domain error message prefix); serde
  deserializes a positive integer and **rejects `0`**; `Default` is 7;
  `Display`/`get` round-trip.
- **`storage/src/site_config.rs`:** existing dual-backend tests pass;
  add/confirm that a stored `"0"` (and a non-integer) reads back as `Default`
  (7).
- **`server/tests/web/web_backup.rs`:** `retention_count=bogus` and
  `retention_count=0` both resolve non-`Ok`; happy path (`retention_count=5`)
  unchanged.

## Verify

- `cargo xtask validate --no-e2e` clean (static + clippy + coverage + tests incl
  PostgreSQL); affected storage/server tests stay dual-backend.
- e2e: extend `backup.spec.ts` — an invalid/zero retention entry disables Save
  and shows the inline message; a valid one re-enables it (mirroring the
  schedule-gating case). Full `validate` (with e2e) at ship.
