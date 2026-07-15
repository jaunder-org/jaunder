# Spec — Issue #453: adopt StrNewtype + client validation for `BackupSchedule`

- **Issue:** [#453](https://github.com/jaunder-org/jaunder/issues/453) (Task,
  milestone _Domain-value type safety (newtypes)_, P2)
- **Branch:** `worktree-issue-453-backup-schedule-newtype` · fork tag
  `wt-base-issue-453`
- **Family:** same ADR-0063 / ADR-0065 shape as the just-landed `Email` vertical
  (#397).

## Goal

`BackupSchedule` is already a validated single-`String` newtype (a six-field
cron expression) but predates `#[derive(StrNewtype)]` and hand-rolls a reduced
surface with a non-validating serde derive. Convert it to the standard machinery
and validate it at both boundaries per ADR-0063 (newtype convention) and
ADR-0065 (client-side domain validation):

1. Convert `BackupSchedule` to `#[derive(StrNewtype)]` with a hand-written
   `FromStr`.
2. Type the `update_backup_settings` **wire arg** as `BackupSchedule` (typed
   arg, not `String` + parse-on-entry).
3. **Client-side pre-validation** of the schedule field via
   `Field`/`ValidatedInput`.

Scope is **schedule only** (per interview). `mode` and `retention_count` remain
bare `String` wire args; follow-on issues will type them (see _Follow-ons_).

## Non-goals / already-safe

- **Storage needs no migration.** `site_config` persists each field as its own
  string key (`config.schedule.as_str()` in;
  `…get(KEY).and_then(BackupSchedule::parse).unwrap_or_default()` out) — it
  never round-trips `BackupConfig` through serde. Invalid stored values already
  fall back to `Default` (`get_backup_config_ignores_invalid_stored_values`). So
  the "deserialize doesn't validate" hole exists only on the **wire**, which the
  typed arg closes. The read path just swaps `BackupSchedule::parse` →
  `str::parse().ok()`.
- No new xtask gate: non-secret, non-security type → plain
  `#[derive(StrNewtype)]`.

## Design

### 1. `common/src/backup.rs` — the newtype

Mirror `common/src/email.rs` (the definitive plain-StrNewtype template):

```rust
use macros::StrNewtype;
use thiserror::Error;

/// A validated six-field cron schedule expression. Constructed via `FromStr` (the single
/// validating/normalizing chokepoint) or `Default`; the ADR-0063 trailer (Display, AsRef,
/// Borrow, Deref, owned conversions, PartialEq<str>, validating serde bridge) comes from
/// the derive, so it serializes as a plain string and rejects invalid input on the wire.
#[derive(Clone, Debug, Eq, PartialEq, StrNewtype)]
pub struct BackupSchedule(String);

/// Error when a string is not a valid six-field cron expression. Carries croner's reason
/// behind a stable label (mirrors `InvalidEmail`); the crate type stays private.
#[derive(Debug, Error)]
#[error("invalid backup schedule: {0}")]
pub struct InvalidBackupSchedule(croner::errors::CronError);

impl FromStr for BackupSchedule {
    type Err = InvalidBackupSchedule;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        Cron::new(trimmed).with_seconds_required().parse().map_err(InvalidBackupSchedule)?;
        Ok(Self(trimmed.to_owned()))
    }
}

impl Default for BackupSchedule { /* unchanged: Self("0 0 0 * * *".to_owned()) */ }
```

- **Remove** `Serialize`/`Deserialize` from the derive list (the bridge is
  generated) and the hand-rolled `parse()` + `as_str()` inherent methods
  (Deref/`.parse()` replace them).
- Confirm croner's error type path (`croner::errors::CronError` or equivalent)
  during implementation; keep the field private so the dependency doesn't leak
  into `common`'s API.

### 2. Wire arg — `web/src/backup/mod.rs`

`update_backup_settings(schedule: String, …)` → `schedule: BackupSchedule`.
Delete the `BackupSchedule::parse(schedule.trim()).ok_or_else(…validation…)`
block — the arg's `Deserialize` (serde bridge → `FromStr`) validates on the
wire. Add the ADR-0065-style comment noting the arg arrives pre-validated and
legitimate clients pre-validate the field.
`mode`/`retention_count`/`destination_path` handling is unchanged.

### 3. Extend `ValidatedInput` — `web/src/forms.rs`

Per interview, extend the shared component with reusable, optional props
(benefit future fields, not just this one):

- `#[prop(optional)] help: Option<&'static str>` — renders a help line and wires
  `aria-describedby` to it (id derived from `name`, e.g. `{name}-help`).
- `#[prop(optional)] class: Option<&'static str>` — override/extend the input
  class so a form with bespoke styling (here `j-backup-input`) keeps its look.

Both default to today's behavior when omitted (no help line; `j-form-input`).
Existing call sites (Email etc.) are unaffected.

### 4. Client form — `web/src/pages/backup.rs`

In `backup_settings_form`, replace the raw schedule `<input>` with a
`Field::<BackupSchedule>::prefilled(&settings.schedule)` bound to the extended
`ValidatedInput<BackupSchedule>` (label "Schedule", `name="schedule"`, help =
the existing cron guidance text, class = `j-backup-input`). Gate the submit
button with `prop:disabled=move || !schedule.is_valid()`. Preserve the
`name="schedule"` so the `#[server]` struct field and any e2e selector still
match.

## Tests

- **`common/src/backup.rs`:** port the existing `parse` tests to `FromStr`
  (`"…".parse::<BackupSchedule>()`), keep trim + six-field + default-valid
  coverage, and add a serde test asserting it serializes as a plain string **and
  rejects an invalid cron string on deserialize** (the new tightening) — mirror
  `email_serde_…`.
- **`web/src/forms.rs`:** add `field_error::<BackupSchedule>` valid + invalid
  cases; add a test for the new `help`/`class` props if the component gains
  testable behavior.
- **Server fn:** the `update_backup_settings` test for an invalid schedule
  asserts the request resolves **non-`Ok`** (not a specific message) — the guard
  is now the serde bridge, not an in-body validation string.
- Update `storage/src/site_config.rs` + its tests and `server/src/backup.rs`
  call sites from `BackupSchedule::parse(x)` to `x.parse().ok()` /
  `.parse().unwrap()`.

## Verify

- `cargo xtask validate --no-e2e` clean (host static + clippy + coverage).
- e2e exercises the client-gated backup form (invalid cron disables submit /
  shows the inline message); full `validate` before ship.
- Coverage: the `backup_settings_form` helper is currently `// cov:ignore`; keep
  parity — the new logic-bearing pieces (`FromStr`,
  `field_error::<BackupSchedule>`) are host-tested.

## Follow-ons (file via jaunder-issues at plan step)

- Type `update_backup_settings`'s `mode` wire arg as `BackupMode` (typed enum
  arg, drop the string match on entry).
- Type its `retention_count` wire arg (currently bare `String` parsed to `usize`
  on entry) — the same pre-ADR-0065 stopgap the user flagged.

## Open (minor, resolve in implementation)

- `InvalidBackupSchedule` carries croner's underlying error behind a stable
  label (`"invalid backup schedule: …"`), consistent with `InvalidEmail`. The
  client field message will therefore surface croner's wording after the label;
  the `forms.rs` negative test asserts the **prefix**, not the crate's exact
  text.
