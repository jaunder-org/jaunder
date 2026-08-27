# Peripheral Process Configuration Implementation Outline

> Execute with `jaunder-iterate`; delegate individual tasks with
> `jaunder-dispatch`. This outline exists because the approved spec moves
> durable host/storage interfaces, process-global initialization, and
> asynchronous storage configuration ownership.

## Scope

In:

- Resolve runtime configuration at executable, command, or test-harness
  composition roots and thread narrow typed values through host, server,
  storage, backup, and test-provisioning interfaces.
- Preserve the existing environment contract and backend parity while deleting
  every in-process environment mutation path.
- Forbid unsafe Rust at every Cargo lint boundary and project the decision into
  the ADR log and architecture view.

Out:

- Renaming variables or changing flags, defaults, precedence, warnings, secret
  semantics, bootstrap credentials, or backend behavior.
- An omnibus process-config bundle, environment trait, reader closure threaded
  into a subsystem, global configuration, or compatibility shim.
- Redesigning Clap, tracing globals, panic hooks, storage `AppState`,
  compile-time environment inputs, or child-process provisioning.

## Task outline

- [x] Task 1: Move host telemetry and capture configuration to process roots
  - Contract: `host` owns narrow immutable telemetry and capture configuration
    types plus pure parsing/policy; it contains no runtime configuration calls
    to `std::env::var` / `var_os`. The `jaunder` and `test-support` roots read
    the named raw inputs once, then pass typed values through tracing,
    diagnostics, saturation metrics, mail, WebSub, and capture commands. One
    resolved OTLP value and one resolved capture directory are reused
    everywhere.
  - Verification: focused host, server observability/commands, and test-support
    tests prove existing fallback, malformed-input, global-initialization, and
    capture behavior without parent-process mutation.

- [x] Task 2: Thread one storage runtime snapshot through every connection path
  - Contract: storage owns a narrow immutable runtime configuration containing
    the shared SQL slow threshold and an optional redacted PostgreSQL
    application password. The command boundary preserves
    file-over-variable-over-URL precedence and typed errors, then passes the
    same value through SQLite and PostgreSQL opening, server
    auto-initialization, database-empty checks, backup/restore reconnects, and
    the scheduled-backup closure. Bootstrap connections remain outside the
    application-password override.
  - Verification: focused storage and server command tests prove SQLite/Postgres
    threshold parity, valid-empty passwords, file trimming, error sources,
    embedded-URL fallback, reconnect reuse, and scheduled backup ownership.

- [x] Task 3: Make PostgreSQL test provisioning own its configuration
  - Contract: an owned PostgreSQL test configuration is resolved at each harness
    setup boundary and passed through template creation, database provisioning,
    helpers, and teardown. `PostgresDbGuard` owns everything its asynchronous
    `Drop` path needs and never rereads ambient state.
  - Verification: focused storage test-support, migration, bootstrap, teardown,
    and dual-backend integration tests prove setup/teardown use one snapshot;
    backend-parity cases continue to use `#[apply(backends)]` where applicable.

- [x] Task 4: Replace ambient environment tests and delete the mutation seam
  - Contract: unit tests construct typed configuration directly. Representative
    Clap flag/environment/default precedence runs in a self-reexecuting server
    test-binary child: the parent uses `current_exe`, selects one sentinel test,
    and supplies child `Command` environment values; the child calls
    `Cli::try_parse_from` with controlled arguments and emits a stable
    projection of the parsed typed fields for the parent to assert. This
    bypasses the production binary's cheap-KDF startup guard without changing
    it. Every `with_env`/`Env` caller and export migrates before
    `common/src/test_support/env.rs` is deleted; no replacement mutation helper
    or wrapper remains.
  - Verification: the self-reexecution contract proves representative flag,
    environment, and default precedence; affected crate tests pass. Source
    inspection finds no `std::env::set_var`, `std::env::remove_var`, `with_env`,
    or exported test `Env`, and runtime configuration `var`/`var_os` calls are
    confined to the approved composition roots.

- [x] Task 5: Enforce and document exception-free unsafe Rust
  - Contract: Cargo lint configuration forbids unsafe code in the root, `xtask`,
    and `tools` workspaces; root members inherit except `web`, whose local lint
    table carries the equivalent rule, and every tools member inherits its
    workspace rule. No suppression exists. Because `linkme` expands to forbidden
    `link_section` code in `web`, server-fn integration registration uses the
    explicit registrar protected by its restored completeness gate. Ship the
    proposed peripheral-process ADR, ADR-0104's dated partial-supersession note,
    ADR-0066's dated registrar amendment, and the truthful architecture
    projection; `CONTEXT.md` remains unchanged.
  - Verification: temporary unsafe probes fail through an inherited root member,
    `web`, standalone `xtask`, and an inherited tools member, then are removed.
    The registrar gate and macro/wire tests, manifest/document checks, and the
    relevant focused Rust lanes pass.

- [x] Task 6 (late correction): Make capture-directory configuration valid-only
  and fail-fast
  - Contract: the applicable `serve` and `test-support` capture-command roots
    resolve `JAUNDER_CAPTURE_DIR` once. Absent or trim-blank input disables
    capture; explicitly configured non-Unicode or uncreatable directories
    return a typed error and abort the root. Construction prepares the directory
    once, after which the value is usable. Stream-path projection is pure and
    infallible, and downstream receives projected leaf paths rather than a
    capture-directory setting or deferred lookup capability.
  - Completion: this records the intentional user-facing correction for #848;
    invalid configured capture directories no longer defer failure to a later
    lookup or side effect.

## Ordering and contracts

- Tasks 1 and 2 establish the production typed interfaces before Task 4 removes
  the shared mutation helper. Task 3 may proceed independently once its owned
  test-config shape does not depend on Task 2's application connection config.
- Task 2's storage runtime configuration is application-only; Task 3's test
  provisioning configuration is a distinct type and must not acquire application
  password-override semantics.
- Task 5 lands after Task 4 removes the explicit unsafe seam; it also removes
  the unsafe `linkme` expansion before enabling the lint. Temporary probes
  verify every lint path and never enter a kept commit.
- Each completed task reaches `jaunder-commit` after its focused evidence. The
  commit hook owns the single precommit run; commits carry no `Co-Authored-By`
  trailer.

## Risk checks

- Raw environment values are read once at the applicable periphery before async
  work; reconnects, workers, and teardown reuse owned typed values.
- Secret configuration is redacted and never gains `Debug`, log, or error output
  containing password bytes.
- PostgreSQL password-file precedence, trailing-whitespace trimming, valid-empty
  behavior, and typed error chains remain exact.
- SQLite and PostgreSQL apply the same slow-query threshold; administrative
  bootstrap remains credential-isolated.
- Clap remains the authority for its declared environment precedence; only its
  integration tests become subprocess-based.
- Child `Command::env` use remains allowed; parent-process mutation and
  third-party wrappers around it do not.
- Direct runtime configuration `std::env::var` / `var_os` calls are limited to
  the named composition roots; unrelated `temp_dir`, `current_dir`, and
  `current_exe` queries remain untouched.
- Global tracing subscriber and panic-hook behavior stays serialized and is not
  made reentrant by this refactor.
- Run the full `cargo xtask validate` gate only after the clean final tree has
  passed focused iteration evidence.
