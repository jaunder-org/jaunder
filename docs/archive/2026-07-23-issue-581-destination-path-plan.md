# `DestinationPath` newtype (#581) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

Spec:
[`docs/superpowers/specs/2026-07-23-issue-581-destination-path.md`](../specs/2026-07-23-issue-581-destination-path.md).
Issue: [#581](https://github.com/jaunder-org/jaunder/issues/581).

**Goal:** Type the backup destination as a non-empty `DestinationPath` newtype
across the vertical, moving the non-empty check to construction and the web form
to the ADR-0065 dispatch pattern.

**Architecture:** A `StrNewtype` in `common::backup` (SiteTitle template)
becomes `BackupConfig.destination_path: Option<DestinationPath>` and the
`update_backup_settings` wire arg. The web form converts `<ActionForm>` → typed
`.dispatch` (mirroring `site.rs`), so an empty destination clears via omission
(`None`). Split into a behavior-preserving form refactor, then the type flip, so
every commit compiles and stays green.

**Tech Stack:** Rust, `macros::StrNewtype` (ADR-0063), Leptos (ADR-0065 `Field`/
`ValidatedInput`), `cargo nextest`, dual-backend storage tests.

## Review header

**Scope (in):** `common::backup` (newtype + field), `common::test_support`
(`parse_destination_path`), `storage/src/site_config.rs` (KV decode/encode),
`server/src/backup.rs` (test constructors only),
`web/src/backup/{api,component}.rs`, `server/tests/web/web_backup.rs`,
`end2end/tests/backup.spec.ts`.

**Scope (out):** `BackupExportOptions.destination_path: &Path` (derived per-run
path — spec decision 4); the form-boilerplate abstraction (#450 — spec decision
5); any absolute-path invariant (spec decision 1). No separable concerns to file
— #450 already exists; cross-reference it at ship.

**Tasks:**

1. `DestinationPath` newtype in `common::backup` + `parse_destination_path`
   helper.
2. Convert the backup form `<ActionForm>` → dispatch (behavior-preserving; wire
   arg stays `String`, in-body `non_empty` stays), landing ADR-0065 client
   validation + the e2e selector change.
3. Flip `BackupConfig.destination_path` + the wire arg to
   `Option<DestinationPath>`, delete in-body `non_empty`, switch the form
   dispatch to `.parsed()`, update storage decode + all test constructors +
   `web_backup.rs` to the omit/reject contract, add the clear-via-omit e2e.

**Key risks/decisions:**

- The empty-`destination_path=` wire contract changes (spec decision 6): clear =
  omit, empty-present = rejected. Mirrors the shipped `Option<AbsoluteUrl>`
  (#448) contract. The `web_backup.rs` omit/reject tests are the guard.
- The field-type flip is one atomic compile unit (Task 3): `common` +
  `storage` + `server` + `web` + integration tests move together. Task 2
  front-loads the form restructure so Task 3 is a one-line dispatch tweak, not a
  form rewrite.

## Global Constraints

- **Validation: non-empty only** (trim + reject empty/whitespace). No
  absolute-path rule.
- **No `Co-Authored-By` trailer** on commits.
- **Newtype test construction** goes through
  `common::test_support::parse_destination_path` (never an inline
  `.parse().unwrap()` at a fixture site).
- **Per-commit gate:** the pre-commit hook runs `cargo xtask check`. Run it
  first so it passes clean (**jaunder-commit**). For web changes touching
  client-only code, also run
  `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`; for
  server-gated web code, verify with `--all-features --all-targets`.
- **Storage tests are dual-backend** (`#[apply(backends)]` / `backends_matrix`)
  — never a bare `#[tokio::test]` on a storage-touching path.

---

### Task 1: `DestinationPath` newtype + test helper

**Files:**

- Modify: `common/src/backup.rs` (add the newtype beside
  `BackupSchedule`/`RetentionCount`)
- Modify: `common/src/test_support.rs` (add `parse_destination_path`)

**Interfaces:**

- Consumes: `macros::StrNewtype`, `std::str::FromStr`, `thiserror::Error`
  (already imported in `backup.rs`).
- Produces:
  - `common::backup::DestinationPath` —
    `#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)] pub struct DestinationPath(String);`
    with
    `impl FromStr for DestinationPath { type Err = InvalidDestinationPath; }`
    (trim, reject empty). Full ADR-0063 str trailer (`Display`, `AsRef<str>`,
    `Borrow<str>`, `Deref<Target = str>`, owned conversions, `PartialEq<str>`,
    validating serde). No `Default`.
  - `common::backup::InvalidDestinationPath` —
    `#[derive(Debug, Error)] #[error("backup destination path cannot be empty")] pub struct InvalidDestinationPath;`
  - `common::test_support::parse_destination_path(s: &str) -> DestinationPath`.

- [ ] **Step 1: Write the failing tests** (append to the
      `#[cfg(test)] mod tests` in `common/src/backup.rs`; mirror the `SiteTitle`
      suite):

```rust
#[test]
fn destination_path_parses_trims_and_preserves_inner() {
    assert_eq!("  /srv/backups  ".parse::<DestinationPath>().unwrap(), "/srv/backups");
    assert_eq!("/srv/my backups".parse::<DestinationPath>().unwrap(), "/srv/my backups");
    // The test helper is the single construction door (used by other crates' fixtures).
    assert_eq!(crate::test_support::parse_destination_path("/srv/x"), "/srv/x");
}

#[test]
fn destination_path_rejects_empty_and_whitespace_only() {
    assert!("".parse::<DestinationPath>().is_err());
    assert!("   ".parse::<DestinationPath>().is_err());
    assert_eq!(
        "".parse::<DestinationPath>().unwrap_err().to_string(),
        "backup destination path cannot be empty"
    );
}

#[test]
fn destination_path_serializes_as_plain_string_and_validates_on_deserialize() {
    let p: DestinationPath = "/srv/backups".parse().unwrap();
    assert_eq!(serde_json::to_string(&p).unwrap(), "\"/srv/backups\"");
    assert_eq!(
        serde_json::from_str::<DestinationPath>("\"/srv/backups\"").unwrap(),
        "/srv/backups".parse::<DestinationPath>().unwrap()
    );
    assert!(serde_json::from_str::<DestinationPath>("\"\"").is_err());
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p common backup::tests::destination_path` Expected:
FAIL — `DestinationPath` / `parse_destination_path` not defined (compile error).

- [ ] **Step 3: Implement against the tests**

In `common/src/backup.rs`, add (beside `BackupSchedule`) the newtype to the
signature above — a `SiteTitle`-style doc comment, the
`#[derive(… StrNewtype)] struct DestinationPath(String);`, the
`InvalidDestinationPath` thiserror struct, and:

```rust
impl FromStr for DestinationPath {
    type Err = InvalidDestinationPath;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(InvalidDestinationPath);
        }
        Ok(DestinationPath(trimmed.to_owned()))
    }
}
```

Every branch (empty-reject, ok, serde round-trip, deserialize-rejects-empty) is
pinned by Step 1, so the body follows the tests. In
`common/src/test_support.rs`, add the import
`use crate::backup::DestinationPath;` (extend the existing `crate::backup::`
import line) and:

```rust
/// Parse `s` into a valid [`DestinationPath`] for tests.
///
/// # Panics
///
/// Panics if `s` is empty or whitespace-only.
#[must_use]
pub fn parse_destination_path(s: &str) -> DestinationPath {
    s.parse().expect("valid test destination path")
}
```

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p common backup::tests::destination_path` Expected:
PASS.

- [ ] **Step 5: Commit**

```bash
git add common/src/backup.rs common/src/test_support.rs
git commit -m "feat(common): DestinationPath StrNewtype in common::backup (#581)"
```

Run `cargo xtask check` first (**jaunder-commit**) — it exercises the `common`
coverage including the new `FromStr` branches.

---

### Task 2: Convert the backup form `<ActionForm>` → dispatch (behavior-preserving)

Structural refactor only — no wire-contract change yet. Spec
§"`web/src/backup/component.rs`". Lands ADR-0065 client validation on the
destination field and the e2e selector change; the wire arg stays `String` and
the in-body `non_empty` stays, so behavior is identical (empty destination still
clears via the in-body check).

**Files:**

- Modify: `web/src/backup/component.rs:49-129` (`backup_settings_form`)
- Modify: `end2end/tests/backup.spec.ts` (button selector in the 3 tests)

**Interfaces:**

- Consumes: `common::backup::DestinationPath` (Task 1); the existing
  `UpdateBackupSettings` server-action struct (still
  `destination_path: String`); `crate::forms::Field`; `BackupMode::VARIANTS`.
- Produces: a dispatch-based `backup_settings_form` — no `<ActionForm>`; a
  `type="button"` submit labelled `"Save Backup Settings"` that
  `update_action.dispatch(UpdateBackupSettings { .. })`.

- [ ] **Step 1: Rewrite `backup_settings_form` to dispatch** (keep the
      `// cov:ignore-start/stop` block). Mirror
      `web/src/pages/site.rs::site_settings_form`:

```rust
fn backup_settings_form(
    settings: BackupConfig,
    update_action: ServerAction<UpdateBackupSettings>,
) -> impl IntoView {
    // Client-validated fields (ADR-0065). Destination is optional (empty clears);
    // schedule/retention are required and seeded from the persisted values.
    let destination = Field::<DestinationPath>::optional_prefilled(
        settings.destination_path.as_deref().unwrap_or_default(),
    );
    let schedule = Field::<BackupSchedule>::prefilled(&settings.schedule);
    let retention = Field::<RetentionCount>::prefilled(&settings.retention_count.to_string());
    let mode = RwSignal::new(settings.mode);
    let submit = move |_| {
        // Required fields are gated valid by the disabled button, so parsed() is Some here.
        if let (Some(schedule), Some(retention_count)) = (schedule.parsed(), retention.parsed()) {
            update_action.dispatch(UpdateBackupSettings {
                // Behavior-preserving: the wire arg is still `String`; the in-body
                // `non_empty` maps "" -> None. Task 3 flips this to `destination.parsed()`.
                destination_path: destination.value.get(),
                schedule,
                retention_count,
                mode: mode.get(),
            });
        }
    };
    // view!: a `<div class="j-card j-backup-form">` (not <ActionForm>) containing, in the
    // existing `j-backup-form-body`/`j-backup-form-actions` layout —
    //   * the destination direct-bind <label>/<input> below (keeps placeholder + classes);
    //   * <ValidatedInput<BackupSchedule>> and <ValidatedInput<RetentionCount>> unchanged
    //     (same props as component.rs:78-97 today);
    //   * the mode <select name="mode"> bound to `mode` (options from BackupMode::VARIANTS,
    //     `selected=m == mode.get()`), `on:change=move |ev|
    //       mode.set(event_target_value(&ev).parse::<BackupMode>().unwrap_or_default())`;
    //   * the submit button below.
}
```

The destination input replaces the current plain `<input>` (component.rs:70-76)
with the `profile.rs`-style direct-bind wiring (still inside the
`j-backup-field-wide` label):

```rust
<label class="j-backup-field j-backup-field-wide">
    <span class="j-edit-form-label">"Destination Path"</span>
    <input
        class="j-backup-input"
        type="text"
        name="destination_path"
        placeholder="/srv/jaunder/backups"
        prop:value=destination.value
        on:input=move |ev| {
            let v = event_target_value(&ev);
            destination.value.set(v.clone());
            destination.error.set(destination.error_for(&v));
        }
        on:blur=move |_| destination.touch()
    />
</label>
{move || {
    destination.is_touched().then(|| destination.error.get()).flatten()
        .map(|msg| view! { <p class="error">{msg}</p> })
}}
```

And the submit button:

```rust
<div class="j-backup-form-actions">
    <button
        type="button"
        class="j-btn is-primary"
        prop:disabled=move || {
            !destination.is_valid() || !schedule.is_valid() || !retention.is_valid()
        }
        on:click=submit
    >
        "Save Backup Settings"
    </button>
</div>
```

Every field's `name=` is preserved (`destination_path`, `schedule`,
`retention_count`, `mode`) for the e2e selectors. Remove the
`use crate::forms::ValidatedInput;` only if unused — it stays
(schedule/retention still use it). Add `use common::backup::DestinationPath;`.

- [ ] **Step 2: Update the e2e button selector** in
      `end2end/tests/backup.spec.ts` — the button is now `type="button"`, so in
      all three tests replace `page.locator(SEL.submit)` with
      `page.locator('button:has-text("Save Backup Settings")')` (matching
      `admin-site.spec.ts`). The `SEL` import may become unused → drop it if so.

- [ ] **Step 3: Verify the host gate + wasm clippy**

Run: `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`
Expected: clean (catches `must_use`/reactive-closure issues the default check
skips). Run: `cargo xtask check` Expected: PASS — `web_backup.rs` integration
tests are unchanged and still green (the wire arg is still `String`).

- [ ] **Step 4: Verify the form in a browser** (behavior unchanged)

Run: `cargo xtask e2e-local backup` Expected: PASS — the 3 tests locate the
button by text; schedule/retention gating and the mode select behave as before.

- [ ] **Step 5: Commit**

```bash
git add web/src/backup/component.rs end2end/tests/backup.spec.ts
git commit -m "refactor(web): backup form to ADR-0065 dispatch (no ActionForm) (#581)"
```

---

### Task 3: Flip to `Option<DestinationPath>` — the type migration + clear contract

The atomic type flip. Spec §"`BackupConfig`", §"storage",
§"`web/src/backup/api.rs`", §"Test support & sweep", decision 6. Everything here
compiles as one unit.

> **Correction (applied during implementation):** the steps below (and the header
> "key risks") were written assuming an empty `destination_path=` would be **rejected**.
> The gate falsified that — the form decoder maps an empty `Option<DestinationPath>`
> field to `None`, so empty-present **clears** exactly like omit. As delivered: no
> reject case for the destination; Step 1(c)'s reject-matrix case was replaced by a
> second positive test, `operator_can_update_backup_settings_clears_via_empty_destination`,
> alongside `..._omits_destination_as_none`. See the corrected spec decision 6.

**Files:**

- Modify: `common/src/backup.rs:126` (the field) + its default test (~line 253)
- Modify: `storage/src/site_config.rs:66-69` (decode)
- Modify: `server/src/backup.rs` (test constructors ~281/306/323)
- Modify: `storage/src/site_config.rs:418` (test constructor)
- Modify: `web/src/backup/api.rs:64-93` (wire arg + delete in-body `non_empty`)
- Modify: `web/src/backup/component.rs` (one line: dispatch
  `destination.parsed()`)
- Modify: `server/tests/web/web_backup.rs` (compile fixes + omit test + reject
  case)
- Modify: `end2end/tests/backup.spec.ts` (add clear-via-omit test)

**Interfaces:**

- Consumes: `common::backup::DestinationPath`,
  `common::test_support::parse_destination_path` (Task 1); the dispatch form
  (Task 2).
- Produces:
  - `common::backup::BackupConfig.destination_path: Option<DestinationPath>`.
  - `web::backup::update_backup_settings(destination_path: Option<DestinationPath>, …)`.

- [ ] **Step 1: Write/adjust the failing integration tests**
      (`server/tests/web/web_backup.rs`), encoding the decision-6 contract:

```rust
// (a) compile fixes — the typed Option has no PartialEq<Option<String>>:
//   operator_gets_configured_backup_settings (~line 62) and
//   operator_gets_defaults_for_invalid_backup_settings (~line 98):
assert_eq!(settings.destination_path.as_deref(), Some("/srv/backups"));

// (b) replace operator_can_update_backup_settings_with_empty_destination (~line 358)
//     with the omit template (mirrors update_site_identity_omits_base_url_as_none):
#[apply(backends)]
#[tokio::test]
async fn operator_can_update_backup_settings_omits_destination_as_none(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/update_backup_settings",
        "schedule=0+0+0+*+*+*&retention_count=5&mode=directory", // destination_path omitted
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let (get_status, get_body) =
        post_form(Arc::clone(&state), "/api/get_backup_settings", "", Some(&cookie)).await;
    assert_eq!(get_status, StatusCode::OK);
    let config: BackupConfig = serde_json::from_str(&get_body).unwrap();
    assert_eq!(config.destination_path, None);
}

// (c) add to the operator_update_backup_settings_rejects_invalid_typed_arg matrix (~line 189):
#[case::empty_destination(
    "destination_path=&schedule=0+0+0+*+*+*&retention_count=5&mode=directory"
)]
```

- [ ] **Step 2: Run the integration tests, verify they fail**

Run: `cargo nextest run -p jaunder --test integration web_backup` Expected: FAIL
— compile error (`destination_path` is still `Option<String>`; the `.as_deref()`
assertions and the omit test don't yet hold).

- [ ] **Step 3: Flip the field and every consumer**

1. `common/src/backup.rs:126` — `pub destination_path: Option<DestinationPath>,`
   (update the field doc). The `backup_config_default_has_no_destination` test
   is unchanged (`None`).
2. `storage/src/site_config.rs:66-69` — decode via the newtype, matching the
   siblings below:

   ```rust
   let destination_path = self
       .get(BACKUP_DESTINATION_PATH_KEY)
       .await?
       .as_deref()
       .and_then(|v| v.parse::<DestinationPath>().ok());
   ```

   (drops `common::text::non_empty_owned` here — still used elsewhere). Add
   `use common::backup::DestinationPath;`. Encode (`:197`
   `config.destination_path.as_deref().unwrap_or("")`) is unchanged (Deref). The
   `set` test (~800-802, `""` → `None`) is unchanged.

3. `storage/src/site_config.rs:418` and `server/src/backup.rs` (~281/306/323) —
   wrap each test literal:
   `destination_path: Some(parse_destination_path("/srv/backups"))` (import
   `common::test_support::parse_destination_path`). `server/src/backup.rs:28`
   (`.as_deref().map(PathBuf::from)`) is unchanged (Deref).
4. `web/src/backup/api.rs` —
   `update_backup_settings(destination_path: Option<DestinationPath>, …)`;
   **delete** the `let destination_path = common::text::non_empty(...)…;` line;
   construct
   `BackupConfig { destination_path, schedule, retention_count, mode }`
   directly. Update the `use common::backup::{…}` to add `DestinationPath`. Keep
   the `#[tracing::instrument(skip(destination_path, …))]`.
   `backup_warning_visible` (`.is_none()`) is unchanged.
5. `web/src/backup/component.rs` — change the one dispatch line
   `destination_path: destination.value.get(),` →
   `destination_path: destination.parsed(),` and delete the now-stale "Task 3
   flips this" comment.
6. `server/tests/web/web_backup.rs` — Step 1's edits now compile.

- [ ] **Step 4: Run the gate, verify green**

Run: `cargo nextest run -p jaunder --test integration web_backup` Expected: PASS
(omit → `None`; empty-present → non-OK; configured round-trip via
`.as_deref()`). Run: `cargo xtask check` Expected: PASS — `common` (field +
newtype), `storage` (decode/encode + dual-backend `site_config` tests), and the
integration suite all green. Also run:
`cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` and a
`cargo check -p web --all-features --all-targets` (the default check skips
server-gated web). Verify AC3 directly:
`git grep -n non_empty -- web/src/backup` returns nothing (the in-body check is
gone).

- [ ] **Step 5: Add the clear-via-omit browser test** to
      `end2end/tests/backup.spec.ts`, mirroring `admin-site.spec.ts` #448: log
      in as `testoperator`, go to `/admin/backups`, fill
      `input[name="destination_path"]` with `/srv/jaunder/backups`, click
      `button:has-text("Save Backup Settings")`, reload, assert the input
      round-trips; then clear the input, save, reload, assert
      `input[name="destination_path"]` is empty and the `BackupBanner` "Backups
      are not configured" alert is visible again.

Run: `cargo xtask e2e-local backup` Expected: PASS (set-then-clear round-trips;
clearing dispatches `None` → omitted → `None`).

- [ ] **Step 6: Commit**

```bash
git add common/src/backup.rs common/src/test_support.rs storage/src/site_config.rs \
  server/src/backup.rs web/src/backup/api.rs web/src/backup/component.rs \
  server/tests/web/web_backup.rs end2end/tests/backup.spec.ts
git commit -m "refactor(common,storage,web): type backup destination as DestinationPath (#581)"
```
