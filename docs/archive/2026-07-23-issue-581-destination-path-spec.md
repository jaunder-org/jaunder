# Spec — #581: `DestinationPath` newtype for the backup destination

Issue: [#581](https://github.com/jaunder-org/jaunder/issues/581) — milestone #13
(Domain-value type safety). Family: #453/#454/#455 (`BackupSchedule` /
`BackupMode` / `RetentionCount` — the typed siblings on the same server fn),
#414 / ADR-0065 (client-side domain validation), ADR-0063 (newtype trailer).

## Problem

`update_backup_settings(destination_path: String, …)`
(`web/src/backup/api.rs:65`) is the last untyped argument on a server fn whose
other three arguments are all domain newtypes. Its non-empty invariant is
enforced in the fn body via `common::text::non_empty`, and the value is stored
as `BackupConfig.destination_path: Option<String>`. The destination is
**optional/clearable**: an empty submission clears it (`None`), which disables
scheduled backups.

## Decisions (resolved in design)

1. **`DestinationPath` is a non-empty `StrNewtype`**, modeled exactly on
   `SiteTitle` (#580): trim surrounding whitespace, reject
   empty/whitespace-only, preserve interior content. Validation is **non-empty
   only** — the issue text, `SiteTitle`, and the current code (server just does
   `PathBuf::from`, no absolute-path requirement) all agree; adding an
   absolute-path rule would introduce a _new_ invariant beyond the issue and
   could reject configs that work today. **Out of scope.**
2. **No `Default`.** Unlike `SiteTitle`, an unconfigured destination is `None`,
   not a default value. The field stays `Option<DestinationPath>`.
3. **The wire arg becomes `Option<DestinationPath>`, which forces the web form
   off `<ActionForm>` onto the ADR-0065 direct-bind `.dispatch` pattern.** An
   `<ActionForm>` submits each field as a string; an empty text input for an
   `Option<DestinationPath>` arg deserializes to `Some("")` → the non-empty
   parse fails → the user gets an error instead of the intended _clear_.
   `profile.rs` and `site.rs` both migrated off `<ActionForm>` for exactly this
   reason (their comments say so). The backup form mirrors
   `web/src/pages/site.rs::site_settings_form` — the near-exact twin (required
   typed fields + one optional-clearable typed field, `ValidatedInput`s inside a
   dispatch form, `type="button"` submit).
4. **`BackupExportOptions.destination_path: &Path` is out of scope.** It is
   _not_ the config value — it is a per-run **derived** path,
   `backup_path_for_mode(destination_root, mode)` =
   `root.join("<timestamp>[.tar.gz]")` (`server/src/backup.rs:152`), and is also
   fed by the CLI (`cmd_backup`) and a computed default (`default_backup_path`),
   neither of which touches the config. It needs `Path` semantics and models a
   different concept (the exact location one backup is written to). If it ever
   earns a newtype, that is a _different_ type in a separate issue.
5. **The boilerplate-reducing form abstraction is out of scope — it is #450**
   ("web(forms): boilerplate-reducing abstraction … ADR-0065 sanctioned
   addition", milestone #9). ADR-0065 and #450 direct that the abstraction be
   extracted from _multiple_ real adopters, not one example. #581 makes the
   backup form a 4th adopter (audiences ×2, posts, site, backup) that #450 can
   later mine. Cross-reference the backup form on #450 at ship.
6. **The clear contract is preserved — clearing works via BOTH omit and an empty
   value.** _(Corrected during implementation: an earlier draft claimed
   empty-present would be rejected; the gate falsified that.)_ Today an empty
   `destination_path=` clears the destination via the in-body `non_empty`. After
   typing the arg `Option<DestinationPath>`, **both** wire forms still clear it:
   the form decoder maps an empty `Option<DestinationPath>` field to `None` (it
   never constructs the newtype from an empty value), and an omitted field also
   decodes to `None`. So the browser client's dispatch-`None` (omitted key) and
   a raw `destination_path=` POST both land as `None` — no rejection case, and
   no observable change from the pre-typing behavior. (The newtype's own serde
   still rejects a bare empty `DestinationPath` — e.g. JSON `""` — but the
   `Option` form layer absorbs the empty field as `None` before that door.) The
   omit path mirrors `Option<AbsoluteUrl>` (#448),
   `server/tests/web/web_site.rs::update_site_identity_omits_base_url_as_none`;
   both clear forms are locked by
   `operator_can_update_backup_settings_omits_destination_as_none` and
   `operator_can_update_backup_settings_clears_via_empty_destination`.

## Design

### `common::backup` — the newtype

- `#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)] pub struct DestinationPath(String);`
  beside `BackupSchedule`/`RetentionCount`, with a `SiteTitle`-style doc
  comment.
- `impl FromStr`:
  `let t = s.trim(); if t.is_empty() { return Err(InvalidDestinationPath) } Ok(Self(t.to_owned()))`.
- `#[derive(Debug, Error)] #[error("backup destination path cannot be empty")] pub struct InvalidDestinationPath;`
- No `Default`.

### `common::backup::BackupConfig`

- `pub destination_path: Option<DestinationPath>` (was `Option<String>`); update
  the field doc. `Default` stays derived (`None`).

### `storage/src/site_config.rs` — the (de)serialization boundary

- **Decode** (`get_backup_config`, ~line 66): replace
  `.and_then(common::text::non_empty_owned)` with
  `.as_deref().and_then(|v| v.parse::<DestinationPath>().ok())`, matching the
  schedule/retention/mode siblings immediately below it. Empty/whitespace →
  `None` (the newtype's parse rejects it). This _is_ the "move the check to
  construction" boundary. `common::text::non_empty_owned` remains used elsewhere
  (web auth/posts) — not dead.
- **Encode** (`set_backup_config`, ~line 197):
  `config.destination_path.as_deref() .unwrap_or("")` is unchanged —
  `.as_deref()` works via the newtype's `Deref<str>`.

### `server/src/backup.rs`

- Line 28: `config.destination_path.as_deref().map(PathBuf::from)` unchanged
  (Deref).
- `BackupExportOptions.destination_path` (the struct lives in
  `storage/src/backup.rs:58`) stays `&Path` (decision 4).

### `web/src/backup/api.rs`

- `update_backup_settings(destination_path: Option<DestinationPath>, …)`;
  **delete** the in-body
  `common::text::non_empty(&destination_path).map(str::to_owned)` line;
  construct `BackupConfig { destination_path, schedule, retention_count, mode }`
  directly. Keep the `#[tracing::instrument(skip(destination_path, …))]`.
- `backup_warning_visible` (`config.destination_path.is_none()`) is unchanged.

### `web/src/backup/component.rs` — `<ActionForm>` → dispatch (mirror `site_settings_form`)

- Destination: a **direct-bind** `<input>` seeded via
  `Field::<DestinationPath>::optional_prefilled(settings.destination_path.as_deref() .unwrap_or_default())`,
  with the 4-line wiring (`on:input`/`on:blur`/error node) as in `profile.rs`.
  Kept direct-bind (not `ValidatedInput`) to **preserve the existing
  `placeholder="/srv/jaunder/backups"`** and the `j-backup-*` classes —
  `ValidatedInput` has no `placeholder` prop (a placeholder-capable
  `ValidatedInput` is #450 territory).
- Schedule / retention: stay `ValidatedInput<BackupSchedule>` /
  `<RetentionCount>` with the existing `Field::prefilled` seeds.
- Mode: the `<select name="mode">` binds a `RwSignal<BackupMode>` seeded from
  `settings.mode`, updated `on:change` (parse the option value).
- Submit: `<button type="button" … on:click=submit>` with
  `prop:disabled=move || !dest_field.is_valid() || !schedule.is_valid() || !retention.is_valid()`.
  `submit` dispatches
  `UpdateBackupSettings { destination_path: dest_field.parsed(), schedule, retention_count, mode: mode.get() }`,
  guarding the two required fields via
  `if let (Some(schedule), Some(retention_count)) = (schedule.parsed(), retention.parsed())`.
- The form body stays inside the existing `// cov:ignore-start/stop` block.

### Test support & sweep

- Add `common::test_support::parse_destination_path(&str) -> DestinationPath`
  (per the newtype test-helper convention).
- Update `cfg(test)` constructors of `destination_path`: `server/src/backup.rs`
  (~281/306/323), `storage/src/site_config.rs` (~418) — wrap the string in
  `Some(parse_destination_path(...))`. The clear test (site_config ~800–802, KV
  `""` → `None`) is unchanged.
- **`server/tests/web/web_backup.rs`** (integration tests, run by
  `validate --no-e2e`) — three required changes:
  1. `operator_gets_configured_backup_settings` (~line 62) and
     `operator_gets_defaults_for_invalid_backup_settings` (~line 98): the
     assertion `settings.destination_path == Some("/srv/backups".to_string())`
     no longer compiles (`Option<DestinationPath>` has no
     `PartialEq<Option<String>>`). Change to
     `assert_eq!(settings.destination_path.as_deref(), Some("/srv/backups"))` —
     the `.as_deref()` idiom already used for `base_url` in `web_site.rs:89`.
  2. `operator_can_update_backup_settings_with_empty_destination` (~line 358)
     becomes the two clear-form tests (decision 6): rename it to
     `operator_can_update_backup_settings_omits_destination_as_none` (POST
     `"schedule=0+0+0+*+*+*&retention_count=5&mode=directory"` with no
     `destination_path` key — mirrors
     `web_site.rs::update_site_identity_omits_base_url_as_none`), and add a
     sibling `operator_can_update_backup_settings_clears_via_empty_destination`
     (POST `destination_path=` empty-present). Both assert OK, then
     `get_backup_settings` → `destination_path == None`.
  3. No reject case for the destination: an empty `destination_path=` decodes to
     `None` (a clear), not a rejection — the `rejects_invalid_typed_arg` matrix
     stays schedule/retention/mode-only, with a comment noting why destination
     is absent.

### e2e

- `end2end/tests/backup.spec.ts` (3 tests) currently locate the button via
  `SEL.submit` (`button[type="submit"]`). After the dispatch conversion the
  button is `type="button"`, so switch those to
  `page.locator('button:has-text("Save Backup Settings")')`, matching
  `admin-site.spec.ts`. The destination field becomes client-validated but
  **optional** (empty stays valid), so the existing schedule/retention/mode
  gating assertions are unaffected.
- **Add** a destination clear-via-omit browser test mirroring
  `admin-site.spec.ts` #448 (set a destination, save, reload, confirm persisted;
  clear it, save, reload, confirm empty). The server-side clear contract is
  already covered by the `web_backup.rs` omit integration test (under the
  `--no-e2e` gate); this additionally exercises the dispatch-omit _client_
  wiring for the restructured form (which today has no destination e2e at all).
  Verified via `cargo xtask e2e-local backup`.

## Acceptance criteria

1. `DestinationPath` exists in `common::backup` with the ADR-0063 `StrNewtype`
   trailer; `"/srv/x".parse::<DestinationPath>()` succeeds and round-trips,
   `"".parse()` and `"   ".parse()` fail with
   `"backup destination path cannot be empty"`; it serializes as a plain JSON
   string and rejects `""` on deserialize.
2. `BackupConfig.destination_path` is `Option<DestinationPath>`;
   `BackupConfig::default() .destination_path == None`.
3. `update_backup_settings`'s wire arg is `Option<DestinationPath>`; the in-body
   `common::text::non_empty(...)` call is gone
   (`git grep non_empty web/src/backup` returns nothing).
4. The backup settings form dispatches typed `UpdateBackupSettings` (no
   `<ActionForm>`); an empty destination input dispatches `None` (clears the
   destination, omitted on the wire), a non-empty valid value dispatches
   `Some(DestinationPath)`, and a whitespace-only value reads as empty
   (cleared). Save stays enabled for an empty destination.
5. Wire contract (decision 6): a POST to `/api/update_backup_settings` that
   **omits** `destination_path`, **and** one that sends an empty
   `destination_path=`, both succeed and clear it to `None`. Both are asserted
   in `web_backup.rs` (`..._omits_destination_as_none` and
   `..._clears_via_empty_destination`).
6. Storage round-trips: a stored non-empty `backup.destination_path` decodes to
   `Some(DestinationPath)`; an empty/absent value decodes to `None`.
7. `parse_destination_path` exists in `common::test_support` and every
   `cfg(test)` `destination_path` constructor uses a typed value.
8. `cargo xtask validate --no-e2e` is clean (including the updated
   `web_backup.rs` integration tests); `backup.spec.ts` (incl. the new
   clear-via-omit test) and `admin-site.spec.ts` remain green (verified via
   `cargo xtask e2e-local`).

## Verification

- Unit: `DestinationPath` parse/serde tests in `common::backup` (mirror the
  `SiteTitle` suite); the `BackupConfig` default test.
- Host: storage decode/encode covered by existing `site_config` tests (updated
  constructors + the unchanged clear test).
- Browser: `cargo xtask e2e-local backup` and `… admin-site` green; manually
  confirm the clear path (set a destination, save; clear it, save;
  `backup_warning_visible` flips).
