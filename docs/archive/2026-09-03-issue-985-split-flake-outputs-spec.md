# Issue #985: split flake outputs by concern

## Outcome

`flake.nix` becomes a readable assembly surface over four concern-owned Nix
files. The refactor preserves every existing flake output, derivation contract,
VM/check behavior, development shell, and lazy fixed-point relationship.

## Load-bearing decisions

- Add four explicit argument-set functions under `nix/`:
  - `nixos.nix` owns the Jaunder NixOS service module, interactive and
    PostgreSQL testing VM configurations, their shared capture configuration,
    and the two testing-VM application launchers.
  - `packages.nix` owns toolchain selection, source filtering, vendoring,
    package derivations, and package inputs.
  - `checks.nix` owns e2e infrastructure and constructors plus static, wasm,
    Elisp, coverage, and doctest checks.
  - `dev-shells.nix` owns the `ci`, `mutants`, and default development shells.
- `flake.nix` owns only flake inputs/configuration, imports, explicit layer
  wiring, and assembly of the final output attribute sets. It contains no
  derivation implementation, NixOS module body, VM body, check script, source
  filter, or shell hook.
- Each imported file receives named dependencies through an explicit argument
  set and returns a named record. No shared catch-all environment hides a
  concern's dependencies.
- Output fragments are producer-owned. `nixos.nix` returns the testing-VM `apps`
  fragment, and `checks.nix` returns the e2e-backed `packages` fragment;
  `flake.nix` merges each with any sibling fragment without changing public
  attribute names.
- Preserve the existing lazy `self` edges explicitly in the layers that already
  depend on `self.packages`, `self.checks`, or `self.nixosModules`. This issue
  does not redesign the flake fixed point.
- Move shell snippets, generated test scripts, source filters, output ordering,
  constants, and comments without semantic edits. Visibility through the flake
  output schema remains identical.
- Preserve ADR-0028, ADR-0034, ADR-0052, and ADR-0118. This decomposition
  creates no new architectural decision and changes no domain vocabulary.

## Acceptance

- Each new file has exactly its named responsibility, and `flake.nix` is
  assembly-only as defined above.
- The public `nixosModules`, `nixosConfigurations`, and every-system `packages`,
  `apps`, `checks`, and `devShells` attribute names are unchanged.
- The four `{sqlite,postgres}×{chromium,firefox}` e2e derivations, aggregate e2e
  package/check outputs, static checks, coverage producer/gate, Elisp
  producer/consumer, wasm checks, and doctest producer/gate retain their current
  wiring and fail/pass behavior.
- A normalized before/after projection of `services.jaunder` options and both
  testing VM configurations proves their package lookup, users, services, ports,
  capture paths, and application launch commands are unchanged.
- Source inclusion/exclusion, offline Cargo homes, patched dependencies,
  leptosfmt/wasm-bindgen vendoring adapter, and dev-shell tool/environment sets
  remain unchanged.
- Before/after targeted Nix evaluation compares the complete output-name
  inventory for every `flake-utils.lib.eachDefaultSystem` system that evaluates
  on the baseline. A baseline-unsupported system must retain the same evaluation
  failure rather than being silently dropped or newly treated as supported.
- The repository's host/static gate passes; PR CI proves the hermetic Nix checks
  and distributed e2e matrix.
- Existing xtask contracts continue to resolve the same installable paths; no
  replacement test framework or duplicate output census is introduced.

## Boundaries

- No behavior work from #802, #828, #893, or #276.
- No dependency upgrades, hash refreshes, source-filter policy changes, new
  checks, renamed outputs, VM changes, shell-tool changes, or CI workflow edits.
- Do not split beyond the four agreed concern files merely to reduce line count.
- Do not remove or replace `self` recursion, and do not introduce a generic
  layer framework or compatibility aliases.
