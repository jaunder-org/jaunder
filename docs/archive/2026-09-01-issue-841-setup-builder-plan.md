# Issue #841 Setup Builder Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for independent
> migration slices. This outline exists because the change alters a cross-crate
> test-support API, durable fixture-state semantics, and transactional storage
> setup.

> Approved specification:
> `docs/archive/2026-09-01-issue-841-setup-builder-spec.md`

## Scope

In:

- A storage-owned awaitable setup builder, typed site-config write surfaces,
  atomic fixture seeding, and dual-backend contract tests.
- Complete migration of registration/base-URL ceremony and the named
  backup/media fixtures.
- Explicit pristine or selective-absence declarations at every affected
  absence/default test.
- The proposed ADR, architecture projection, and removal of the redundant
  base-URL helper.

Out:

- Production config defaults, keys, validation, schema, provisioning, or backend
  lifecycle changes.
- General cleanup of unrelated site-config tests or raw call sites.
- Raw or invalid values in the setup builder.

## Shared contracts

- `Backend::setup(self) -> SetupBuilder`;
  `SetupBuilder: IntoFuture<Output = TestEnv>` preserves bare `.await`.
- Consuming typed methods are `registration(RegistrationPolicy)`,
  `base_url(Option<BaseUrl>)`, `backup(BackupConfig)`,
  `media_limits(MaxFileSize, UserQuota)`, and exact `pristine()`, each returning
  `Self`.
- The default seed is Open registration plus canonical `https://example.com/`.
- Overrides replace only their corresponding defaults. Duplicate declarations
  and pristine/override combinations panic with a fixture-configuration message
  in either order.
- `inject_invalid_site_config(&TestEnv, SiteConfigKey, &str) -> Result<(), sqlx::Error>`
  is the test-only physical-row injector consumed by defensive-read tests; it is
  not a builder option or a raw storage accessor.
- Private `Backend::provision() -> TestEnv` performs only database provisioning.
  `seed_site_config(&TestEnv, SiteConfigSeed, SeedFailure) -> anyhow::Result<()>`
  owns the single confirmed write scope. `SeedFailure::AfterFirst` exists only
  under `cfg(test)`; the public builder calls the same function with
  `SeedFailure::Never` and returns `TestEnv` only after success. The rollback
  test provisions first, retains that private environment, invokes the failure
  variant, and inspects the rolled-back rows.
- Existing typed identity and backup behavior setters remain public behavior
  surfaces.

## Task outline

- [x] Task 1: Deliver the typed, atomic dual-backend setup contract.
  - Ownership: `storage/src/test_support/backend.rs`,
    `storage/src/test_support/mod.rs`, `storage/src/site_config.rs`, and their
    colocated tests. Later tasks do not edit these files.
  - Contract: Implement every shared setup, typed-setter, invalid-row, and
    rollback-seam signature above. Migrate the relevant site-config tests to
    typed setters or the invalid-row injector.
  - Verification: Dual-backend tests prove defaults, every override group,
    selective base-URL omission, exact pristine state, duplicate/exclusive
    failures, rollback after a forced later seed failure, and continued
    `set_identity`/`set_backup_config` round trips.

- [x] Task 2: Remove registration-policy ceremony while preserving policy and
      account behavior.
  - Depends on: Task 1 setup API.
  - Ownership: all edits in `server/tests/web/web_auth.rs` and
    `server/tests/web/web_account.rs`, including their base-URL setup.
  - Contract: Bare Open prerequisites disappear; InviteOnly/Closed behavior uses
    explicit typed setup or typed transitions; absence-safe-default cases use
    pristine setup. Account URL setup uses the builder's optional base-URL
    contract. Policy transition/getter tests retain behavior-oriented typed
    writes.
  - Verification: Focused dual-backend web-auth/account tests cover bare Open
    registration, InviteOnly/Closed behavior, absent-policy Closed behavior,
    generated invite URLs, and explicit missing-URL behavior.

- [x] Task 3: Make the common base URL implicit and delete its parallel
      convention.
  - Depends on: Tasks 1, 2, and 4 so no remaining caller lives in a concurrently
    owned web file.
  - Ownership: `server/tests/helpers/session.rs` and every
    `setup_with_base_url()` caller except `web_account.rs` and `web_media.rs`,
    which Tasks 2 and 4 own exclusively.
  - Contract: Delete `setup_with_base_url()` and migrate owned atompub/feed/web
    callers to bare setup; URL-absence cases use `base_url(None)` when they need
    the remaining baseline or pristine setup when they require no site config.
    Direct URL overrides remain typed and deliberate.
  - Verification: Focused AtomPub, feed, email, and password-reset tests cover
    generated absolute URLs and explicit missing-URL failures; source census
    finds no helper definition or caller.

- [x] Task 4: Migrate backup/media fixture preconditions without hiding config
      behavior.
  - Depends on: Task 1 setup API and invalid-row injector.
  - Ownership: all edits in `server/tests/web/web_backup.rs`,
    `server/tests/web/web_media.rs`, and affected media-manager tests; Task 3
    does not edit `web_media.rs`.
  - Contract: Aggregate backup/media builder options replace only precondition
    ceremony, and `web_media.rs` base-URL setup follows the new baseline. Valid
    setting, update, transition, and persistence subjects continue through typed
    setters. Invalid legacy-row fallback tests use physical test injection.
    Default/absence tests declare pristine state where the new baseline would
    otherwise matter.
  - Verification: Focused dual-backend backup and media tests prove aggregate
    overrides, accessor defaults, invalid-row defense, update persistence, URL
    behavior, and limit boundaries.

- [x] Task 5: Integrate the complete fixture contract and decision record.
  - Depends on: Tasks 1–4.
  - Contract: Reconcile all `Backend::setup()` callers with the new baseline;
    keep `TestEnv`, templates, SQLite/PostgreSQL lifecycle, and production
    Closed policy unchanged. Ensure the proposed ADR and `docs/ARCHITECTURE.md`
    describe the delivered API exactly.
  - Verification: Repository static checks, complete host-native product tests,
    and source census showing no web-only config setup ceremony, no
    `setup_with_base_url`, and no unintentional raw config primitive use
    introduced by this issue.

## Risk checks

- Existing `.setup().await` callers must not require imports, type annotations,
  or lifetime changes.
- The boxed awaitable future is test-only and `Send`; no manual future state
  machine or production allocation is introduced.
- Seed validation completes before writes where possible; any storage failure
  rolls back the single transaction.
- The public fixture never exposes a partially seeded `TestEnv`; only the
  private rollback test holds the provisioned environment while seeding fails.
- SQLite and PostgreSQL receive identical rows and preserve their existing
  per-test isolation and teardown order.
- `.pristine()` means zero site-config rows, not merely suppression of the two
  defaults.
- `base_url(None)` suppresses only the base-URL row and retains Open
  registration.
- Production absent/invalid registration remains Closed.
- Test-only invalid-row injection is physically explicit and cannot become a
  production or general fixture API.
- `CONTEXT.md` remains unchanged because no ubiquitous language changes.
