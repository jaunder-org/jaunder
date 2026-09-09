# Issue #1422: Refresh nix-direnv for imported Nix modules

## Outcome

A checkout using the repository's nix-direnv environment automatically
reevaluates its development shell when any imported Nix module changes. Pulling
a new native build dependency therefore makes that dependency available before
`cargo xtask sandbox` builds host artifacts.

## Load-bearing decisions

- The active Nix development shell remains the dependency boundary for
  host-native xtask workflows, including UX sandboxes.
- `.envrc` watches every repository-owned Nix module imported by `flake.nix`, in
  addition to the files nix-direnv already watches itself.
- The watches are established before `use flake`, so nix-direnv cannot select a
  cached shell before learning the complete invalidation set.
- The fix covers the repository's current flat `nix/*.nix` module set without
  enumerating individual filenames; adding another module at that level inherits
  the same behavior.
- xtask does not duplicate native dependency knowledge or invoke Nix. `dav1d`
  remains owned by the existing Nix package and development-shell definitions.
- This is cache invalidation behavior, not a new architecture or domain
  decision; no ADR or `CONTEXT.md` change is required.

## Acceptance

- A direnv environment loaded from the checkout records all current `nix/*.nix`
  modules as watched files.
- In a disposable checkout, adding and importing another flat `nix/*.nix` module
  makes direnv watch that module without adding its filename to `.envrc`.
- Changing an imported Nix module invalidates the loaded environment and causes
  the next direnv entry to reevaluate the flake instead of reusing the prior
  cached development shell.
- The reevaluated shell's target pkg-config path contains the declared dav1d
  development package.
- From that reevaluated shell, `cargo xtask sandbox --profile demo` completes
  host artifact compilation and reaches sandbox readiness rather than failing in
  `dav1d-sys` discovery.

## Boundaries

- Bare host shells outside the repository's Nix development environment remain
  unsupported.
- The change does not add sandbox-specific dependency preflights, dependency
  installation, or nested `nix develop` execution.
- AVIF support and its native dav1d dependency remain unchanged.
- Sandbox profile, fixture, persistence, locking, and process-lifecycle
  semantics remain unchanged.
