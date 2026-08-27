# ADR-0158: Process environment is resolved at the periphery

- Status: accepted
- Date: 2026-08-27
- Issue: [#848](https://github.com/jaunder-org/jaunder/issues/848)

## Context

Rust 2024 makes process-environment mutation unsafe on platforms where readers
and writers cannot be synchronized.
[ADR-0104](0104-edition-2024-unsafe-env-and-precise-capturing.md) centralized
test mutation behind one lock, but that lock cannot constrain reads performed by
runtime threads, libc, or dependencies. The safety obligation is therefore
auditable but not structurally discharged.

The prohibition also applies to code generated inside a Jaunder crate. The
`linkme` distributed slice selected by
[ADR-0066](0066-server-fn-test-registrar-guard.md) expands to an unsafe
`link_section` declaration in `web`; centralizing unsafe inside a dependency
does not make the repository exception-free.

The same migration exposed a broader design problem. Host telemetry and capture,
storage opening, PostgreSQL test provisioning, and Clap tests obtain
configuration from ambient process state at different depths. Tests mutate that
state to reach policy branches, while reconnect and teardown paths may reread it
later. This is difficult to reason about even apart from unsafe mutation.

[ADR-0016](0016-dependency-injection-and-appstate.md) already concentrates
breadth at composition roots and requires subsystems to declare narrow
constructor dependencies rather than receive heterogeneous bundles or service
locators. Process configuration should follow the same rule.

## Decision

Treat the inherited process environment as immutable startup input.

Executable, command, or test-harness composition roots read runtime
configuration through `std::env::var` / `var_os` and resolve raw values into
narrow typed configuration owned by the relevant subsystem. Each subsystem
receives only those typed values through its ordinary interface. Library modules
do not read ambient configuration, receive a general environment reader, or
access a global configuration object.

Resolution happens before asynchronous work begins. Backend-specific values may
be resolved at a command boundary after backend selection. Test harnesses pass
owned configuration through provisioning and teardown. Reconnects, background
workers, and asynchronous teardown own or borrow the same resolved snapshot;
they never reread ambient state.

Clap continues to resolve its declared flag/environment/default inputs at the
executable boundary. Unit tests pass typed values directly. Tests of actual
environment wiring run a child with `std::process::Command` environment methods,
which configure inherited state before the child starts without mutating the
parent.

Delete the repository's in-process environment-mutation seam and forbid unsafe
Rust without exceptions at every Cargo lint boundary in the root, `xtask`, and
`tools` workspaces. This decision supersedes ADR-0104's process-environment
mutation decision; its Rust edition, precise-capturing, and formatter decisions
remain current.

## Consequences

- Process configuration has one directional flow: inherited input, typed
  resolution at the periphery, then explicit injection.
- Environment-sensitive policy becomes ordinary deterministic code, and unit
  tests no longer coordinate through global state.
- Configuration is a stable snapshot for one process or command. Runtime
  mutation and later ambient rereads are unsupported.
- Existing variable names, defaults, precedence, diagnostics, backend parity,
  and secret-handling behavior remain part of the interface.
- Storage open, backup/restore, scheduled work, telemetry, capture, server
  startup, and test provisioning interfaces gain explicit typed inputs.
- Capture follows the same boundary: its owning root constructs an optional,
  valid-only `CaptureDirectory` once. Absent or trim-blank input disables
  capture; configured non-Unicode or uncreatable directories fail fast before
  work begins. A constructed directory is usable, and downstream code receives
  only an infallibly projected leaf path, never an ambient capture setting or a
  directory lookup capability.
- Child-process tests remain necessary for the small integration surface where
  the inherited environment itself is the behavior under test.
- `common::test_support::{with_env, Env}` and its unsafe operations disappear.
  Third-party wrappers around the same operating-system mutation are ruled out.
- The repository-wide unsafe prohibition becomes structural rather than a grep
  convention, and no lint suppression is needed.
- Server-fn integration tests return to ADR-0066's explicit registrar plus
  completeness gate. This trades generated unsafe code for a mechanical list
  whose drift fails the ordinary static gate.
- Compile-time environment access and process-manager or child-command
  provisioning are outside this decision because they do not mutate a running
  process's environment.
