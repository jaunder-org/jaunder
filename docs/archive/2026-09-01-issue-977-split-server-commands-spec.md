# Split server commands by concern

## Outcome

The server command module keeps one stable dispatch interface while its
implementations and tests are organized by independently changing concerns. All
public command paths, CLI behavior, startup behavior, and operator-visible
output remain unchanged.

## Load-bearing decisions

- Keep `jaunder::commands` as the module interface. A wiring-only
  `commands/mod.rs` explicitly re-exports every currently public function and
  type used by production and integration-test callers.
- Keep dispatch thin and stable in a private `dispatch` module:
  `Commands::execute`, `SiteConfigAction::execute`, and `CommandOutput` continue
  routing the same variants, arguments, and results.
- Split implementations into private `storage_bootstrap`, `account`, `backup`,
  `lifecycle`, and `site_config` modules, plus a narrow private `support` module
  for runtime-configuration resolution and confirmed-mutation normalization
  shared across concerns.
- `storage_bootstrap` owns explicit `init` and PostgreSQL provisioning commands.
  `account` owns user, app-password, invitation, and SMTP diagnostic commands.
  `backup` owns backup/restore and target validation. `site_config` owns stored
  site-configuration CRUD and formatting.
- `lifecycle` owns database startup policy, server preparation, workers,
  shutdown, and serving. `prepare_server` remains ADR-0016's composition root
  and retains its dependency construction, injection, startup ordering, cleanup,
  and error behavior. The storage `Backend` remains root-only and is never
  injected into a subsystem; no heterogeneous dependency holder or services
  bundle passes beyond the root; services and workers remain root-constructed
  and injected per consumer.
- Preserve ADR-0144 and ADR-0158 process-configuration precedence,
  applicability, immutable snapshot, and production/development startup rules.
  Keep process configuration distinct from ADR-0102 stored site configuration.
- Preserve ADR-0064, ADR-0115, ADR-0054, and ADR-0136 backup/restore target
  selection, emptiness, ordering, failure, interop, and retained-data contracts.
- Preserve the single telemetry guard at the executable dispatch seam from
  ADR-0011; command implementations do not acquire telemetry lifecycle
  ownership.
- Move each unit test beside its owning implementation in a concern-local
  `tests` module. Use private shared test support only for fixtures used by
  multiple concerns. Internal test paths may gain concern segments; test
  function names and behavior remain unchanged, and hardcoded subprocess filters
  are updated to the new path.

## Acceptance

- Existing production and integration-test imports through
  `jaunder::commands::…` compile unchanged, including all public command
  functions, `ServeCapturePaths`, `PreparedServer`, and public field types.
- Every CLI variant reaches the same command implementation with the same
  resolved arguments and `CommandOutput` behavior.
- `prepare_server` remains the composition root and constructs/injects storage,
  maintenance, workers, WebSub, mailer, saturation metrics, router, listener,
  and runtime metadata in the same order and ownership relationships. No
  extracted interface accepts the root-only `Backend`, a heterogeneous
  dependency holder, or a services bundle.
- Every existing unit test exists exactly once with its original function name
  and assertions under its owning concern; existing integration tests remain
  unchanged.
- Each implementation leaf has one named responsibility; `commands/mod.rs`
  contains assembly only; shared support contains only cross-concern policy or
  fixtures.
- Focused command tests and the repository gate pass.

## Boundaries

- No CLI grammar, flags, environment variables, defaults, output, exit behavior,
  command applicability, or dispatch semantics change.
- No storage schema, backup format, restore policy, site-configuration registry,
  server dependency graph, worker behavior, telemetry setup, or production
  endpoint behavior change.
- No new abstraction, dependency-injection container, command trait, second
  dispatch layer, compatibility alias, or duplicated public path.
- No ADR, architecture projection, glossary, or generated ADR table change;
  existing decisions fully govern this structural refactor.
