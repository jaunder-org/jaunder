# Remove Unused cfg-if Dependency

Issue: #1287 Status: Approved

## Outcome

The `web` crate no longer declares `cfg-if` as a direct dependency, and the
workspace dependency registry no longer advertises an unused shared dependency.

## Changes

- Delete `cfg-if = "1.0.0"` from the root `[workspace.dependencies]` table.
- Delete `cfg-if.workspace = true` from `web/Cargo.toml`.
- Retain any `Cargo.lock` entry still required transitively; otherwise accept
  Cargo's normal lockfile removal.

## Acceptance

- Cargo metadata does not report `cfg-if` as a direct dependency of `web` or any
  other workspace member.
- `cargo xtask check` passes, including the web host and wasm compilation
  surfaces.
- No conditional-compilation source, feature declaration, target-specific
  module, registration path, test, or tool changes.

## Boundaries

- No replacement dependency or abstraction.
- No dependency version updates unrelated to removing this direct edge.
- No ADR is required because this deletes an unused manifest declaration without
  changing an architectural contract.
