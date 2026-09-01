# Issue #841: typed site-config setup builder

## Outcome

Dual-backend tests receive a useful site baseline from bare
`Backend::setup().await`: registration is Open and the site base URL is
`https://example.com/`. Tests state only deliberate overrides or absence
requirements, while production configuration defaults remain unchanged.

## Load-bearing decisions

- `Backend::setup()` remains the storage-owned, feature-gated, dual-backend
  fixture entry point and still awaits directly to `TestEnv` at every unchanged
  caller.
- Calling bare setup atomically seeds two typed site-config values after
  migration and before returning the environment: Open registration and
  `https://example.com/` as the base URL.
- The setup builder owns its options and exposes typed overrides for
  registration policy, optional base URL, aggregate backup configuration, and
  aggregate media limits.
- The base-URL option accepts the domain's optional value. `None` omits only the
  base-URL row while preserving the remaining baseline.
- Backup configuration is supplied as its domain aggregate. Media limits are
  supplied together as their two validated domain values. The builder does not
  expose raw key/value configuration.
- `.pristine()` means no site-config rows are seeded. It is mutually exclusive
  with every override and fails loudly when combined with one in either call
  order.
- Specifying the same override twice fails loudly rather than applying
  order-dependent last-write-wins behavior.
- All selected baseline and override rows commit through one confirmed write
  scope. Setup never returns a partially seeded fixture.
- Production accessors retain their safe behavior for absent or invalid data,
  including Closed registration. The Open policy is a test-fixture baseline, not
  a production default.
- Valid behavior setup continues through typed accessors, including
  `set_identity` and `set_backup_config`; the builder does not absorb or remove
  those behavior surfaces. Tests of deliberately invalid legacy rows inject
  physical database state through test support rather than widening the raw
  config primitive beyond ADR-0102's typed-accessor boundary.
- The redundant `setup_with_base_url()` convention is deleted and every caller
  migrates to bare setup or a deliberate override.
- The durable fixture-baseline contract is recorded in
  `docs/adr/0170-typed-test-site-config-baseline.md` and projected into
  `docs/ARCHITECTURE.md`.

## Acceptance

- Existing bare `backend.setup().await` call sites compile unchanged and still
  receive `TestEnv` on SQLite and PostgreSQL.
- A newly added HTTP registration test can register without setting registration
  policy or a base URL.
- Bare setup reads back Open registration and the canonical
  `https://example.com/` base URL on both backends.
- Registration, optional base URL, aggregate backup configuration, and aggregate
  media-limit overrides read back exactly on both backends.
- `base_url(None)` leaves the base-URL row absent without suppressing Open
  registration.
- Pristine setup contains no site-config rows and preserves the
  absent-registration result of Closed on both backends.
- Pristine-plus-override and duplicate-override configurations fail
  deterministically before exposing an environment.
- A dual-backend rollback test forces fixture seeding to fail after its first
  selected row, retains access to the database through the failure seam, and
  observes that none of the selected rows persisted.
- No test under `server/tests/web/` writes a site-config key solely as setup
  ceremony. Valid behavior setup uses typed accessors; deliberately invalid
  stored rows use test-only physical storage injection.
- Existing typed identity and backup setters remain exercised by tests whose
  subject is setting, transition, or persistence behavior.
- No caller uses or defines `setup_with_base_url()`.
- Tests that require absent configuration state that requirement through
  `.pristine()` or the selective optional base-URL override rather than relying
  on an incidental empty table.

## Boundaries

- This change does not alter production site-config defaults, validators, keys,
  wire formats, or storage schema.
- It does not make the builder a general raw-config seeding API or accept
  invalid typed values.
- It does not replace or remove behavior-focused typed config setters.
- It does not change database provisioning, migration, pool ownership,
  PostgreSQL teardown, SQLite file/WAL behavior, backend templates, or `TestEnv`
  ownership.
- It does not add a second test harness outside `storage::test_support`.
- It does not change Jaunder's ubiquitous language, so `CONTEXT.md` is
  unchanged.
