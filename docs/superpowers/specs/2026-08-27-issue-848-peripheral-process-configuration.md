# Issue #848: Peripheral process configuration

## Outcome

Jaunder treats the process environment as immutable startup input. Executable or
command composition roots resolve it once into typed values and inject only each
subsystem's explicit needs; production and test code never mutate it in process.
The repository's Rust code forbids unsafe Rust without exceptions.

## Load-bearing decisions

- Runtime configuration reads through `std::env::var` / `var_os` belong only at
  executable, command, or test-harness composition roots. Library modules
  consume typed values and do not accept a general environment reader.
- Configuration is split into narrow typed records for the owning subsystem.
  There is no omnibus process-config bundle, global config, service locator, or
  `Environment` trait passed through the application.
- The `jaunder` and `test-support` executable roots resolve their inherited
  environment before starting asynchronous work. A command may defer
  backend-specific resolution until it knows which backend it will use, but
  resolution still happens at that command's composition boundary. PostgreSQL
  test callers construct an owned test configuration at their harness setup
  boundary and pass it through provisioning and teardown.
- Clap remains the owner of its existing flag/environment/default precedence.
  Its already-typed fields continue through command dispatch unchanged.
- Host telemetry and capture configuration are immutable typed inputs. One
  resolved OTLP value governs both exporter initialization and saturation-metric
  activation; capture paths for diagnostics, mail, and WebSub derive from one
  resolved capture directory.
- Storage opening receives an immutable runtime configuration containing the
  resolved slow-query threshold and, for application PostgreSQL connections, an
  optional redacted password override. SQLite and PostgreSQL use the same
  threshold.
- PostgreSQL credential precedence remains password file, then password
  variable, then the URL's embedded password. File contents retain trailing
  whitespace trimming, including the existing valid-empty-value behavior;
  malformed variables and unreadable files preserve their typed error sources.
  Administrative bootstrap connections remain separate and do not acquire the
  application-password override.
- Backup, restore, scheduled backup, server auto-initialization, and every
  reconnect reuse the command's resolved storage configuration rather than
  rereading ambient state.
- PostgreSQL test provisioning resolves its test and bootstrap URLs once. Owned
  test configuration follows setup through asynchronous teardown, including
  `Drop`; teardown cannot observe a different environment snapshot.
- Unit tests pass ordinary typed configuration values. Subprocess tests are
  reserved for observable environment-to-configuration wiring, including Clap
  precedence and process-global initialization.
- Setting a child environment through `std::process::Command` remains valid: it
  defines inherited state before the child starts and does not mutate the
  parent.
- `common::test_support::{with_env, Env}` is deleted. No replacement in-process
  mutation helper or safe wrapper around unsafe OS mutation is introduced.
- Cargo lint configuration forbids unsafe code across the root, `xtask`, and
  `tools` Rust workspaces. Root members inherit the lint except `web`, whose
  existing local lint table carries the equivalent rule; every `tools` member
  likewise inherits its workspace rule. There is no suppression.
- The peripheral-configuration ADR supersedes only ADR-0104's audited
  environment-mutation decision. ADR-0104's edition, precise-capturing, and
  formatter decisions remain current.

## Acceptance

- Runtime configuration calls to `std::env::var` / `var_os` occur only in
  executable, command, or test-harness composition-root code; host, storage, and
  server subsystems receive typed configuration values.
- Existing variables, defaults, precedence, malformed-input behavior, warning
  behavior, secret redaction, and SQLite/PostgreSQL parity remain observable.
- No Rust source calls `std::env::set_var` or `std::env::remove_var`; child
  process configuration uses `Command` methods only.
- `common/src/test_support/env.rs` and all `with_env`/`Env` exports and callers
  are removed with no compatibility path.
- Telemetry, capture, storage opening, backup/restore, scheduled backup, server
  startup, and PostgreSQL test teardown all reuse one resolved snapshot.
- Focused unit tests cover typed parsing and subsystem policy without ambient
  mutation. Focused subprocess tests prove representative Clap and process
  environment wiring.
- Temporary unsafe probes fail compilation through every distinct Cargo lint
  path: an inheriting root member, `web`'s local lint table, the standalone
  `xtask` package, and an inheriting `tools` member. The probes are removed
  afterward.
- The new ADR draft and `docs/ARCHITECTURE.md` describe peripheral process
  configuration and the exception-free unsafe-code prohibition; the obsolete
  current-state projection of ADR-0104's environment seam is removed, and
  ADR-0104 carries a dated note that only its environment-mutation decision was
  superseded.
- The repository's full `cargo xtask validate` gate passes.

## Boundaries

- No environment variable is renamed or removed, and no user-facing
  configuration precedence changes.
- Compile-time environment access, build-script inputs, shell/service-manager
  provisioning, and non-configuration process queries such as `temp_dir`,
  `current_dir`, and `current_exe` are unchanged.
- Child-process environment setup in devtool and test harnesses is retained.
- This work does not redesign global tracing subscribers, panic hooks, storage
  `AppState`, database bootstrap credentials, or Clap's configuration model.
- `CONTEXT.md` is unchanged: this is an implementation and architecture
  decision, not new domain language.
