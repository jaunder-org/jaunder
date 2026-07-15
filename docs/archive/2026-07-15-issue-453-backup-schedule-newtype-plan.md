# Plan — Issue #453: adopt StrNewtype + client validation for `BackupSchedule`

**Spec:**
[`docs/superpowers/specs/2026-07-15-issue-453-backup-schedule-newtype.md`](../specs/2026-07-15-issue-453-backup-schedule-newtype.md)
**Issue:** [#453](https://github.com/jaunder-org/jaunder/issues/453) ·
**Branch:** `worktree-issue-453-backup-schedule-newtype` · **Fork tag:**
`wt-base-issue-453` **For agentic workers:** drive with `jaunder-iterate`;
delegate a task via `jaunder-dispatch` when useful. Commit through
`jaunder-commit` (full `cargo xtask check` in the pre-commit hook). **No
`Co-Authored-By` trailer.**

## Review header

**Goal:** Convert `BackupSchedule` to `#[derive(StrNewtype)]` (ADR-0063) and
validate it at both boundaries — a typed `BackupSchedule` wire arg on
`update_backup_settings` and client-side pre-validation of the schedule field
(ADR-0065) — mirroring the `Email` vertical (#397).

**Scope**

- **In:** `BackupSchedule` newtype conversion; `update_backup_settings` schedule
  wire arg; `ValidatedInput` help/class props; backup form client validation;
  the call-site + test updates that fall out.
- **Out:** typing the `mode` (→ #454) and `retention_count` (→ #455) wire args —
  filed, P3, blocked-by #453. Storage schema/migration (none needed — per-field
  string keys, already default-on-invalid).

**Tasks**

1. Follow-on issues #454/#455 — **already filed** (mode, retention_count wire
   args). No action.
2. `common`: convert `BackupSchedule` to `#[derive(StrNewtype)]` + `FromStr` +
   `InvalidBackupSchedule`; port tests; add serde-validates test.
3. `common` consumers: swap `BackupSchedule::parse`/`as_str` at
   `storage/src/site_config.rs` + `server/src/backup.rs` (and their tests) to
   `.parse()` / Deref.
4. `web`: type the `update_backup_settings` schedule wire arg as
   `BackupSchedule`; drop parse-on-entry; server-fn test asserts non-`Ok`.
5. `web/src/forms.rs`: extend `ValidatedInput` with optional `help`
   (+`aria-describedby`) and `class` props; add `field_error::<BackupSchedule>`
   tests.
6. `web/src/pages/backup.rs`: bind `Field::<BackupSchedule>` via the extended
   `ValidatedInput`; gate submit; preserve help + `j-backup-input`.
7. Full `cargo xtask validate` (incl. e2e) green.

**Key risks / decisions**

- Confirm croner's error type path (`croner::errors::CronError`) — Task 2
  verifies via a compile.
- Removing the `Serialize`/`Deserialize` derives while the generated bridge
  takes over: `BackupConfig` (which derives them and embeds `BackupSchedule`)
  must still compile + round-trip.
- `ValidatedInput` is a shared `#[component]` (Email et al. call it) — new props
  are `#[prop(optional)]`, defaulting to today's behavior; existing call sites
  must be untouched.
- e2e selector on the backup schedule input must survive — keep
  `name="schedule"` and the `j-backup-input` class.

## Global constraints

- Backend parity (ADR-0019) — storage tests stay dual-backend
  (`#[apply(backends)]`); this change touches no dialect files.
- Coverage policy (ADR-0050): logic-bearing code is host-tested (`FromStr`,
  `field_error`); `ValidatedInput`/`backup_settings_form` are
  `#[component]`/`cov:ignore` as today — don't add uncovered non-component host
  logic.
- Import discipline: import `BackupSchedule` etc. so call sites don't repeat
  `common::backup::`.
- Run the gate worktree-aware: `devtool run -- cargo xtask check`.

---

## Task 2 — `common/src/backup.rs`: StrNewtype conversion

**Files**

- `common/src/backup.rs` (edit type, add `FromStr` + error, port tests).

**Interfaces**

- Replace the struct + inherent impl:

```rust
use croner::Cron;
use macros::StrNewtype;
use thiserror::Error;
// keep `use serde::{Deserialize, Serialize};` — BackupMode + BackupConfig still derive them.

/// A validated six-field cron schedule expression. Constructed via `FromStr` (the single
/// validating chokepoint) or `Default`; the ADR-0063 trailer (Display, AsRef, Borrow,
/// Deref, owned conversions, PartialEq<str>, validating serde bridge) comes from the derive,
/// so it serializes as a plain string and rejects invalid input on the wire.
#[derive(Clone, Debug, Eq, PartialEq, StrNewtype)]
pub struct BackupSchedule(String);

/// Error when a string is not a valid six-field cron expression. Carries croner's reason
/// behind a stable label (mirrors `common::email::InvalidEmail`); the crate type stays private.
#[derive(Debug, Error)]
#[error("invalid backup schedule: {0}")]
pub struct InvalidBackupSchedule(croner::errors::CronError);

impl FromStr for BackupSchedule {
    type Err = InvalidBackupSchedule;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        Cron::new(trimmed)
            .with_seconds_required()
            .parse()
            .map_err(InvalidBackupSchedule)?;
        Ok(Self(trimmed.to_owned()))
    }
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self("0 0 0 * * *".to_owned())
    }
}
```

- Remove `Serialize, Deserialize` from the derive list; remove inherent
  `parse()` and `as_str()`.
- Add `use std::str::FromStr;`.
- **Confirm the croner error type** at implementation: if
  `croner::errors::CronError` is wrong, `cargo check -p common` names it; adjust
  the tuple field type. Keep the field private.

**Test** (in-file `#[cfg(test)]`, port existing + add serde):

```rust
#[test] fn backup_schedule_parse_accepts_valid_six_field_cron() {
    assert!("0 0 0 * * *".parse::<BackupSchedule>().is_ok());
    assert!("0 30 2 * * MON-FRI".parse::<BackupSchedule>().is_ok());
}
#[test] fn backup_schedule_parse_rejects_invalid_expressions() {
    assert!("".parse::<BackupSchedule>().is_err());
    assert!("not a cron".parse::<BackupSchedule>().is_err());
    assert!("* * * * *".parse::<BackupSchedule>().is_err()); // five-field
    assert!("99 0 0 * * *".parse::<BackupSchedule>().is_err());
}
#[test] fn backup_schedule_default_is_valid() {
    let s = BackupSchedule::default();
    assert_eq!(s, "0 0 0 * * *");                     // PartialEq<str> from the trailer
    assert!(s.to_string().parse::<BackupSchedule>().is_ok());
}
#[test] fn backup_schedule_parse_trims_whitespace() {
    assert_eq!("  0 0 0 * * *  ".parse::<BackupSchedule>().unwrap(), "0 0 0 * * *");
}
#[test] fn backup_schedule_serializes_as_plain_string_and_validates_on_deserialize() {
    let s: BackupSchedule = "0 0 0 * * *".parse().unwrap();
    assert_eq!(serde_json::to_string(&s).unwrap(), "\"0 0 0 * * *\"");
    assert!(serde_json::from_str::<BackupSchedule>("\"0 0 0 * * *\"").is_ok());
    assert!(serde_json::from_str::<BackupSchedule>("\"not a cron\"").is_err()); // the tightening
}
```

**Run**

- `cargo nextest run -p common backup` → PASS (after the consumer fixes below
  compile; run the whole crate at Task 3 if the removed `as_str()` breaks
  `common`-internal callers — there are none, so `-p common` should pass
  standalone).

## Task 3 — `common` consumers of `parse`/`as_str`

**Files**

- `storage/src/site_config.rs` — read path + `set_backup_config` + tests.
- `server/src/backup.rs` — test construction.

**Interfaces**

- `get_backup_config`:
  `…get(BACKUP_SCHEDULE_KEY).await?.as_deref().and_then(BackupSchedule::parse)`
  → `.and_then(|s| s.parse().ok())`.
- `set_backup_config`: `self.set(BACKUP_SCHEDULE_KEY, config.schedule.as_str())`
  → `self.set(BACKUP_SCHEDULE_KEY, &config.schedule)` (Deref coercion
  `&BackupSchedule` → `&str`).
- Tests using `BackupSchedule::parse("…").unwrap()` → `"…".parse().unwrap()`
  (site_config `set_and_get_backup_config_round_trips`; any in
  `server/src/backup.rs`). `grep` for `BackupSchedule::parse` and `\.as_str()`
  on a schedule to catch all sites.

**Test**

- Existing `set_and_get_backup_config_round_trips` and
  `get_backup_config_ignores_invalid_stored_values` must still pass (behavior
  unchanged: invalid stored value → default).

**Run**

- `cargo nextest run -p storage site_config` → PASS (dual-backend).
- `cargo nextest run -p server backup` → PASS.

## Task 4 — `web/src/backup/mod.rs`: typed schedule wire arg

**Files**

- `web/src/backup/mod.rs`.

**Interfaces**

- Signature: `schedule: String` → `schedule: BackupSchedule` (keep the other
  three args).
- Delete the
  `let schedule = BackupSchedule::parse(schedule.trim()).ok_or_else(…)?;` block;
  use `schedule` directly in the `BackupConfig`.
- The `tracing::instrument(skip(...))` list keeps `schedule`.
- Ensure `BackupSchedule` is imported unconditionally (like `Email` in
  `web/src/email/mod.rs`) — it's now a typed `#[server]` arg, so the generated
  request struct needs it on both client (serialize) and server (deserialize)
  sides. Move it out of the `#[cfg(feature = "server")]`-only `use` group.
- Add the ADR-0065 comment: the arg arrives pre-validated (its `Deserialize` ran
  `FromStr`); legitimate clients pre-validate the field, so an invalid value
  only reaches here from a non-browser caller.

**Test** (server-fn test — locate the existing `update_backup_settings` test;
likely `web/src/backup/server.rs` or a `#[cfg(test)]` in the module)

- An invalid schedule now fails at arg deserialization, not in-body. Assert the
  call resolves **non-`Ok`** (per the newtype-issue convention — don't assert
  the old validation message). Valid path unchanged.

**Run**

- `cargo nextest run -p web backup` → PASS.

## Task 5 — `web/src/forms.rs`: extend `ValidatedInput`

**Files**

- `web/src/forms.rs`.

**Interfaces**

- Add two optional props to `ValidatedInput<T>`:
  - `#[prop(optional)] help: Option<&'static str>`
  - `#[prop(optional)] class: Option<&'static str>`
- Render: when `help` is `Some`, emit a help
  `<span id={format!("{name}-help")} class="j-form-help">{help}</span>` and set
  the input's `aria-describedby` to that id (both `None`/absent otherwise —
  unchanged markup). Use `class.unwrap_or("j-form-input")` for the input class.
- Watch leptosfmt on the generic `<ValidatedInput<T>>` tag (#420) — keep the
  existing formatting idiom.

**Test**

- Existing `valid_input_is_none` / `invalid_input_is_the_newtypes_own_message`:
  add `BackupSchedule` cases —
  `assert_eq!(field_error::<BackupSchedule>("0 0 0 * * *"), None);`
  `assert!(field_error::<BackupSchedule>("not a cron").is_some_and(|m| m.starts_with("invalid backup schedule")));`
- Import `common::backup::BackupSchedule` in the test module (qualified;
  `common` has no top-level re-exports).
- `ValidatedInput` is a `#[component]` (coverage-exempt); the new prop branches
  need no separate host test beyond the existing component-render coverage, but
  if a pure helper is factored out, host-test it.

**Run**

- `cargo nextest run -p web forms` → PASS.

## Task 6 — `web/src/pages/backup.rs`: client validation

**Files**

- `web/src/pages/backup.rs`.

**Interfaces**

- In `backup_settings_form(settings, update_action)`: create
  `let schedule = Field::<BackupSchedule>::prefilled(&settings.schedule);`
  (Deref `&BackupSchedule` → `&str`).
- Replace the raw schedule `<label>…<input name="schedule" …/>…</label>` block
  with the extended component:

```rust
<ValidatedInput<BackupSchedule>
    label="Schedule"
    name="schedule"
    field=schedule
    class="j-backup-input"
    help="Use a six-field cron expression: second minute hour day-of-month month day-of-week. Example: 0 0 0 * * * runs daily at midnight."
/>
```

- Gate the submit:
  `<button type="submit" class="j-btn is-primary" prop:disabled=move || !schedule.is_valid()>`.
- Import `crate::forms::{Field, ValidatedInput}` and
  `common::backup::BackupSchedule`.
- Keep the surrounding `j-backup-*` layout classes on the containing elements;
  only the schedule field adopts the component. The `cov:ignore` block around
  `backup_settings_form` stays.

**Test**

- No host unit test (form helper is `cov:ignore`); covered by e2e in Task 7. If
  a backup e2e spec exists, assert an invalid cron disables the save button /
  shows the inline error, and a valid one submits — otherwise add that assertion
  to the existing backup spec.

**Run**

- `cargo nextest run -p web` → PASS (host).

## Task 7 — Full gate

- `devtool run -- cargo xtask validate` (static + clippy + coverage + e2e, all
  `{sqlite,postgres}×{chromium,firefox}` combos). Run **foreground** with a long
  timeout (coverage rebuild) — never Bash background.
- Confirm the `xtask-done:` sentinel + `ok:true`. Green → ready for
  `jaunder-ship`.

## Self-review

- Every task compiles before its commit (Task 2's `common` edits land with Task
  3's consumer fixes if `-p common` alone breaks — but `as_str()`/`parse()` have
  no in-`common` callers, so Task 2 is self-contained).
- No placeholders; each interface is complete Rust.
- Removed derives (`Serialize`/`Deserialize`) verified not double-defined
  against the generated bridge — Task 2's serde test proves the bridge is
  present and validating.
- Follow-on scope (#454/#455) is filed and excluded, not silently dropped.
