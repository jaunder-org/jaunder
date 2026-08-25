# Issue #787 — scan trait-default SQLx decode sites

## Outcome

`sqlx-newtype-decode` includes calls in trait default-method bodies in its
structurally enumerated decode population. The widened gate fails closed on
unapproved targets while distinguishing site-configuration reads from SQLx row
decodes through exact, counted allowlist entries.

## Load-bearing decisions

- Trait default-method bodies join the same structural population as free
  functions and implementation methods. Required trait methods without a body
  contribute no decode sites.
- Decode target precedence remains nearest-declared type first: call-level
  turbofish, then an ascribed `let`, then the enclosing function or trait-method
  return type.
- The scanner reuses its existing function-name, return-type, and `let`
  ascription state while visiting a trait default body; it does not introduce a
  trait-specific inference path.
- Newly visible `SiteConfigStorage::get` calls that return aggregate
  site-configuration values are structural over-bites, not SQLx row decodes.
  Each entry has `count: 1`, category `NotADecodeTarget`, and reason
  `SiteConfigStorage::get reads typed site-configuration text, not a SQLx row decode`.
  The exact inventory is:
  - `get_backup_config` targeting `sqlx::Result<BackupConfig>` for
    `BackupDestinationPath`, `BackupSchedule`, `BackupRetentionCount`, and
    `BackupMode`;
  - `get_registration_policy` targeting `sqlx::Result<RegistrationPolicy>` for
    `SiteRegistrationPolicy`; and
  - `get_identity` targeting `sqlx::Result<SiteIdentity>` for `SiteTitle` and
    `SiteBaseUrl`. Receiver-name guessing is forbidden.
- Existing trait defaults whose return leaves are already approved remain
  approved through the normal type model and receive no allowlist entry.
- Scanner population documentation states explicitly that enclosing
  trait-default-method return types participate. `SiteConfigStorage`
  documentation drops its stale claim that trait default bodies are invisible
  while retaining the independent requirement that `get_smtp_config` perform
  typed SQLx bridge decoding at the query boundary.

## Acceptance

- A synthetic `.get` call in a trait default body is recorded under the exact
  trait-method return target `Result<i64, E>`; removing trait-item visitation
  makes that assertion fail.
- Synthetic tests assert exact target vectors or failure text while proving
  call-level turbofish and typed-`let` precedence over the trait method return,
  and proving a required method without a default body adds no site.
- `devtool run -- cargo test --manifest-path xtask/Cargo.toml sqlx_newtype_decode_check`
  passes with the synthetic trait-default coverage.
- `cargo xtask check --no-test` passes with exactly the seven new counted
  site-configuration over-bite entries and no unallowlisted trait-default site.
- `cargo xtask validate --no-e2e` passes with the widened population.

## Boundaries

- No change to the `.bind` direction, gate-module layout, or site-configuration
  value typing.
- No receiver-name or SQL-text heuristic and no broad function/file exemption.
- No architecture decision record or implementation outline: this routine
  static-gate bug changes no architecture, schema, public API, security,
  concurrency, or storage-correctness contract.
