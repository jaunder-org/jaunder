# Issue #878: Role-typed devtool PostgreSQL URLs

## Outcome

The devtool ephemeral-PostgreSQL harness carries its test-database and
bootstrap-database connection URLs as distinct types. Swapping either role in
`PgEnv` construction or environment emission fails at compile time, while
emitted URL values and process behavior remain unchanged.

## Load-bearing decisions

- `tools/devtool/src/pg.rs` owns a local generic `PgUrl<Role>` carrier with a
  zero-sized role marker.
- The concrete roles are exposed inside the crate through `TestPgUrl` and
  `BootstrapPgUrl` aliases; concrete use sites spell the aliases.
- `PgEnv` keeps the established `test_url` and `bootstrap_url` field names,
  makes both fields private, and assigns the corresponding distinct aliases.
- `PgUrl<Role>` stores trusted static host and port components; each role marker
  supplies the fixed database user and database name used when rendering the
  process-environment value.
- The carrier is structured endpoint data, not a string-backed domain newtype,
  so ADR-0063's `StrNewtype` trailer does not apply.
- The trusted `app_url(host, port)` and `bootstrap_url(host, port)` composition
  helpers are the only minting doors.
- `PgEnv::configure_command` is the sole environment-emission interface. It
  delegates to a private helper whose `&TestPgUrl` then `&BootstrapPgUrl`
  parameters bind each role to its environment key.
- Both command-building callsites use that interface; the exact URL strings are
  rendered only inside the helper at `Command::env`, the process boundary.
- ADR-0112 supplies the generic role-tagged carrier pattern; ADR-0063 supplies
  the transposition-safety criterion without imposing its string-backed
  interface on structured endpoint data.
- Existing server CLI `InvalidPgUrl`, storage parsing, and environment consumers
  remain unchanged; they own different validation and dependency seams.
- No parser, validation error, shared dependency, Nix source change, role
  conversion, neutral alias, or compile-fail harness is introduced.

## Acceptance

- `PgEnv.test_url` is a `TestPgUrl`; `PgEnv.bootstrap_url` is a
  `BootstrapPgUrl`.
- `app_url` returns `TestPgUrl`; `bootstrap_url` returns `BootstrapPgUrl`.
- Each role renders its exact established URL from the held host and port.
- `run_command` and `coverage::emit::run` configure their commands only through
  `PgEnv::configure_command`.
- A focused `Command::get_envs` test proves both exact environment key/value
  mappings through that interface.
- Existing URL-format tests continue to prove the exact generated strings.
- A temporary negative compiler probe that transposes the private helper's typed
  URL arguments produces Rust `E0308`; after restoration, the tools workspace
  checks pass.
- `JAUNDER_PG_TEST_URL` and `JAUNDER_PG_BOOTSTRAP_TEST_URL` receive the same
  values as before.

## Boundaries

- No storage, server, common, SQLx option, or environment-consumer type is
  changed.
- The issue does not rename existing fields or environment variables.
- The issue does not add role conversion or a neutral PostgreSQL URL alias that
  would bypass role safety.
- No domain glossary or new ADR is required: ADR-0063 governs the
  transposition-safety criterion, while ADR-0112 already records the accepted
  role-tagged carrier structure.
