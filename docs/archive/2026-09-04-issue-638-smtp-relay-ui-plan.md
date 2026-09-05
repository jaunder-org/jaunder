# Operator-managed SMTP Relay Configuration Implementation Outline

> Execute with `jaunder-iterate` and delegate with `jaunder-dispatch`. This
> outline exists because the approved specification changes a secret boundary, a
> dual-backend transactional aggregate, server-function APIs, authorization, and
> concurrent read/write semantics.

## Scope

In:

- Host-only stored SMTP password with a client-reachable inbound twin and static
  flow enforcement.
- Atomic/coherent SMTP configuration storage on SQLite and PostgreSQL.
- Operator-only `/admin/smtp` API, page, navigation, validation, and browser
  coverage.
- Persisted settings that require an external process restart to become active.

Out:

- Runtime mailer reload, self-restart, service-manager integration, connectivity
  tests, schema changes, generic site-config UI, and CLI behavior changes.

## Shared contracts

- `common::smtp_password` owns `InvalidSmtpPassword`,
  `validate_smtp_password_shape`, `SmtpPasswordShape`, and
  `ProfferedSmtpPassword`.
- `host::smtp_password::SmtpPassword` is the only stored secret and implements
  `TryFrom<ProfferedSmtpPassword>`.
- `host::smtp_config` owns:
  - `SmtpConfigUpdate::{Disabled, Enabled { host, port, tls_mode, sender, credentials }}`;
  - `SmtpCredentialsUpdate::{Unauthenticated, Keep { username }, Replace { username, password }}`.
- `storage::SmtpConfigUpdateError` has exactly `MissingStoredPassword` (fixed
  and valueless) and `Database(#[source] sqlx::Error)`.
  `SiteConfigStorage::update_smtp_config(&mut WriteTransaction, &SmtpConfigUpdate) -> Result<(), SmtpConfigUpdateError>`
  acquires the SMTP aggregate's transaction-scoped PostgreSQL advisory lock
  before reading or writing; SQLite relies on `WriteScope`'s `BEGIN IMMEDIATE`.
  The lock is held through the caller's commit or rollback. `Keep` is checked
  before any mutation through that same transaction, then all relevant keys are
  updated or deleted atomically. The web boundary maps `MissingStoredPassword`
  to the fixed credential-validation response and `Database` through the
  existing public storage-error boundary.
- `SiteConfigStorage::get_smtp_config` remains the sole host-only aggregate
  reader. One aggregate SQL statement captures all six rows; per-key typed
  decode errors retain their labels, missing effective values retain their
  domain defaults, and absence of the host returns `None`.
- `web::smtp` owns `/api/smtp/get_settings` and `/api/smtp/update_settings`.
  `UpdateSettings` takes one `UpdateSettingsRequest` under
  `#[macros::server(skip_all)]`. Its secret-free `Settings` DTO contains
  `enabled`, `host: Option<SmtpHost>`, effective `port`, `tls_mode`, and
  `sender`, `authentication_enabled`, `username: Option<SmtpUsername>`, and
  `password_configured`. `None` maps to disabled, blank host/credentials, and
  domain defaults; a legacy credential remnant maps to authentication enabled.
- The route is `/admin/smtp`; the operator sidebar label is `SMTP Relay`.

## Task outline

- [x] Task 1: Establish the secret and storage aggregate
  - Contract: implement the `common`, `host`, and `SiteConfigStorage` contracts
    above; move `parse_smtp_password` to exactly
    `host::test_support::parse_smtp_password`; migrate generated mocks and every
    direct caller; leave no compatibility re-export. Coordinate concurrency
    regressions from inside their test-owned `WriteScope` closures after the
    mutation returns but before the closure permits commit; add no production
    test hooks.
  - Verification: common/host exact-byte and redaction tests; dual-backend
    aggregate update, full disable, paired clear, rollback, stale-keep,
    interleaving-writer serialization, and uncommitted-before/committed-after
    coherent-read tests; existing SMTP loader and mailer tests.
- [x] Task 2: Add the operator SMTP web vertical
  - Contract: implement
    `web::smtp::{Settings, UpdateSettingsRequest, get_settings, update_settings, SmtpSettingsPage}`
    against Task 1's exact types; require operator auth; convert the inbound
    password immediately; clear the password field at dispatch; never return or
    render it.
  - Verification:
    - Host/component: request-to-domain decision fold; disabled/default,
      unauthenticated, authenticated, legacy-partial, and
      password-configured-only render states; conditional field validity; and
      immediate raw/DOM password clearing after every dispatch.
    - `server/tests/web/web_smtp.rs` backend matrix: anonymous/member rejection
      by both functions, operator typed decode, every persistence transition,
      stale conflict, rollback, secret-free responses, and malformed-wire
      rejection.
    - Serial e2e: anonymous/member sidebar hiding and direct denial, operator
      accessibility, write-only password and browser-store redaction, save and
      external-restart feedback, in-app re-entry persistence, password
      replacement/keep, paired clearing, full disable, and one-document
      navigation. Restore/delete all six singleton keys after the test.
- [x] Task 3: Extend static API, write, and secret-flow inventories
  - Contract: add `ProfferedSmtpPassword` and both owner files to the
    `proffered-secret` registry; add `update_smtp_config` to the closed
    `SiteConfigStorage` write census (Site Config 9→10, total 60→61) and project
    that live census into `docs/ARCHITECTURE.md`; register
    `web::smtp::{GetSettings, UpdateSettings}` in
    `server/tests/helpers/registrar.rs` (68→70); add `/api/smtp/get_settings`
    and `/api/smtp/update_settings` to `server/tests/web/server_fn_wire.rs`.
  - Verification: focused xtask workspace tests for positive/negative secret
    positions, write-transaction census, registrar census, tracing `skip_all`,
    and server-function wire-argument errors; focused server wire-contract test.
- [x] Task 4: Integrate and prove the complete surface
  - Contract: route/sidebar/component/API/storage names remain exactly those
    above; no fallback aliases or duplicate paths.
  - Verification: repository static check, focused host/server/browser tests,
    security review, then the normal commit and PR gates.

## Parallel ownership

- Slice A owns `common`/`host` SMTP password and config files,
  `storage/src/site_config.rs`, storage SMTP tests, test-support relocation,
  generated-mock migration, and direct callsite migration.
- Slice B owns `web/src/smtp/**`, app route/sidebar wiring,
  `server/tests/web/web_smtp.rs`, SMTP e2e tests, and
  `end2end/playwright.config.ts`. The SMTP spec runs only in each browser's
  serial admin-settings project and participates in that project's dependency
  chain. Slice B consumes Slice A's exact contracts without editing Slice A
  files.
- Slice C owns `xtask` secret/write/API static gates,
  `server/tests/helpers/registrar.rs`, `server/tests/web/server_fn_wire.rs`, and
  the architecture census projection. It consumes the shared names without
  editing Slice A/B files. Registrar, wire, and source-shape checks run after
  Slice B creates the generated API types.
- The controller owns integration fixes, post-e2e trace-backed
  `docs/coverage/server-fns.json` regeneration, cross-slice verification,
  review, and shipping; coverage entries are never added before real trace
  evidence.

## Risk checks

- Neither password type or submitted bytes appears in a response, DOM prefill,
  browser storage, span, error, log, or ordinary helper surface.
- Dispatch clears the raw password field immediately; framework action input is
  absent when no request remains in flight.
- A stale `Keep` is checked before any writes and cannot create a partial
  credential pair; the PostgreSQL aggregate advisory lock and SQLite immediate
  writer lock serialize interleaving updates through commit or rollback, and a
  failed mutation leaves every old row intact.
- One aggregate SQL statement plus a test-controlled writer transaction proves
  that `get_smtp_config` yields the complete old aggregate before commit and the
  complete new aggregate after commit on both backends while retaining per-key
  decode labels and defaults.
- Disabled state deletes all six keys; enabled unauthenticated state deletes
  both credential keys.
- The live injected `MailSender` does not change until external restart, and UI
  feedback says so.
- Static router registration and direct-route denial match existing admin pages;
  only sidebar visibility depends on operator state.
- Server-function count, registrar, wire path, trace skip policy,
  write-capability census, architecture projection, proffered-secret registry,
  and trace-backed coverage inventory move together in their stated order.
