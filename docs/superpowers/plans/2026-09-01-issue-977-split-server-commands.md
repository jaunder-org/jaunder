# Split Server Commands Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded work through
> `jaunder-dispatch`. This outline exists because moving the command module
> interface and ADR-0016 composition-root implementation carries
> architecture-sensitive seam and ownership risk.

## Scope

In:

- Preserve `jaunder::commands` while extracting private concern modules and
  concern-local tests.
- Preserve command dispatch, process/stored configuration policy, backup/restore
  behavior, and server composition/lifecycle behavior.
- Keep every existing public symbol available at its current path through
  explicit facade re-exports.

Out:

- CLI, protocol, storage, configuration, dependency-graph, telemetry, or worker
  behavior changes.
- New abstractions, compatibility aliases, ADRs, glossary terms, or architecture
  prose.

## Task outline

- [x] Task 1: Extract shared command policy and storage bootstrap.
  - Contract: private `support` owns runtime-configuration resolution and
    confirmed-mutation normalization; private `storage_bootstrap` owns explicit
    initialization and PostgreSQL provisioning. Create private test support
    containing only the cross-concern `sqlite_storage_args` and
    `assert_command_source` fixtures; all other test helpers stay with their
    sole consumer. Existing public bootstrap functions remain explicitly
    re-exported at `jaunder::commands::…`.
  - Verification: configuration/bootstrap unit tests pass at their new concern
    paths, including the updated subprocess filter; existing command
    integration-test files remain content-identical and compile against
    unchanged imports.
- [x] Task 2: Extract account/operator commands and tests.
  - Contract: private `account` owns user, app-password, invitation, and SMTP
    diagnostic handlers. It consumes production `support` and only the shared
    test fixtures named by Task 1; SMTP-only fixtures remain local. Existing
    public account/operator functions retain `jaunder::commands::…` paths.
  - Verification: focused account, invitation, SMTP, and source-chain tests
    pass; `server/tests/misc/commands.rs` remains content-identical.
- [ ] Task 3: Extract backup/restore commands and tests.
  - Contract: private `backup` owns backup/restore, validation reporting, target
    derivation, emptiness checks, and backup-local helpers. Existing public
    backup functions retain `jaunder::commands::…` paths.
  - Verification: focused backup unit tests pass;
    `server/tests/misc/backup_interop.rs` and backup coverage in
    `server/tests/misc/commands.rs` remain content-identical and pass.
- [ ] Task 4: Extract stored site-configuration commands and tests.
  - Contract: private `site_config` owns stored-key set/get/list/unset handlers,
    formatting, and site-config-only fixtures; it consumes production `support`
    without acquiring process-configuration ownership. Existing public
    site-configuration functions retain `jaunder::commands::…` paths.
  - Verification: focused dual-backend site-configuration tests pass; command
    integration-test files remain content-identical.
- [ ] Task 5: Extract dispatch and lifecycle, then finalize the assembly-only
      facade.
  - Contract: private `dispatch` retains `Commands::execute`,
    `SiteConfigAction::execute`, and `CommandOutput`; private `lifecycle` owns
    startup database policy, `prepare_server`, workers, serving, shutdown, and
    lifecycle-local fixtures. `commands/mod.rs` contains declarations and
    explicit re-exports only. All current public functions/types, including
    `ServeCapturePaths`, `PreparedServer`, and public field types, retain their
    paths. Remove the old monolith, obsolete imports/helpers, and duplicates
    only after every owner and caller is migrated.
  - Verification: lifecycle/dispatch unit tests and the broad command test lane
    pass; all three existing command integration-test files remain
    content-identical and pass; every original unit test appears exactly once;
    the public symbol/path inventory matches the approved spec. Compare pre/post
    `prepare_server` source wiring for storage, maintenance, workers, WebSub,
    mailer, saturation metrics, router, listener, and runtime metadata,
    explicitly checking ownership, construction/start order, rollback cleanup,
    and error propagation.

## Risk checks

- `prepare_server` alone owns the root-only storage `Backend`; no extracted
  interface accepts it, a heterogeneous dependency holder, or a services bundle.
- Root construction and injection of storage, maintenance, workers, WebSub,
  mailer, saturation metrics, router, listener, and runtime metadata retain
  their exact ownership and order; rollback cleanup and error propagation remain
  unchanged; services/workers remain injected per consumer.
- Dispatch match arms, resolved arguments, `CommandOutput`, telemetry-guard
  ownership, and public re-export paths do not change.
- Process configuration remains immutable startup input and distinct from stored
  site configuration.
- Backup/restore target selection, emptiness, clear-then-load ordering, failure
  atomicity, and retained-data behavior do not change.
- Unit-test function names/assertions remain unchanged; only concern-module path
  segments and the matching subprocess filter change.
- No `mod.rs` contains implementation items and no lint suppression is
  introduced.
