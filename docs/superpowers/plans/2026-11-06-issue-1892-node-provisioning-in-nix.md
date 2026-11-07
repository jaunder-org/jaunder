# Move Node dependency provisioning from devtool — implementation outline

> Execute with dev-cycle-iterate. Outline needed because this changes the
> durable provisioning owner across host shell, Just and a Nix derivation, with
> a shared link-safe setup interface.

## Execution summary

First prove that one Nix-provided setup executable can be consumed by devShell
startup, the Just TSC workflow, `static-typescript` and the census derivation
without adding more maintained complexity than the removed Rust provisioner.
Then connect those callers and remove the Rust interface. Preserve the approved
spec's package, symlink and census contracts.

## Task outline

- [x] Task 1: Provide and prove one shared Nix setup executable
  - Contract: Nix supplies `${e2ePackage}/node_modules` and
    `${pkgs.playwright-test}/lib/node_modules/@playwright/test`; setup acts only
    on the caller's current `end2end/node_modules` destination and safely
    handles absent, ordinary-directory, repeated and stale-symlink states.
  - Verification: focused setup regression checks both pinned package sources,
    dot-entry omission, preserved unrelated sentinel, unlinks a stale
    destination symlink before writes, and confirms its external referent is
    unchanged. Compare the complete proposed Rust/Just/script/Nix/test/doc
    surface with removals and additions; proceed only if one setup
    implementation reduces total maintained complexity without duplicate
    caller-specific provisioning. If not, return the mechanism choice to spec
    approval before wiring callers.
- [ ] Task 2: Route the host and census consumers through the shared setup
  - Contract: devShell supports IDE use; Just TSC works after moving into a
    fresh worktree in the already-running shell; census setup mutates only its
    writable copied source. Each of the devShell, Just TSC workflow,
    `static-typescript` and `static-e2e-census` receives both
    `${e2ePackage}/node_modules` and
    `${pkgs.playwright-test}/lib/node_modules/@playwright/test` directly.
    `devtool check tsc` retains check-definition ownership; census
    population/verdict stay unchanged.
  - Verification: inspect all four call sites for the same two Nix-provided
    paths and no caller-specific fallback. First confirm the fresh worktree's
    ignored `end2end/node_modules` is absent; invoke the Just TSC workflow, then
    confirm setup created the expected links and TSC completed. Run the actual
    `static-e2e-census` and `static-typescript` derivations. Repository-wide
    search covers the Rust CLI name, module/test references, and all former call
    sites.
- [x] Task 3: Retire the Rust provisioner surface and confirm net simplification
  - Contract: remove `tools/devtool/src/provision.rs`, its unit tests,
    `tools/devtool/tests/provision_cli.rs`, provision CLI arguments/dispatch,
    and obsolete Just/Nix/shell call sites; retain unrelated devtool checks and
    census logic.
  - Verification: run focused remaining devtool tests; verify no
    provision-specific references remain across Rust, Just, scripts, Nix, tests
    and documentation. Review the complete maintained
    Rust/Just/script/Nix/test/doc diff against removed implementation and tests
    to confirm no duplicate helper layer and less total maintained complexity.

## Risk checks

- A symlink at `end2end/node_modules` must be unlinked as a link before any
  directory creation or writes; its target must remain unchanged.
- The Nix helper must be available in all three caller environments without
  introducing duplicated implementations or a larger maintained wrapper layer.
- ADR-0028's in-sandbox `devtool` role and ADR-0052/0146's shared static-check
  definition ownership remain unchanged; provisioning moves, check definitions
  do not. ADR-0225 is precedent only, not expanded authority.
- Do not alter package versions/pairing, census membership/verdict, unrelated
  `devtool` responsibilities, or accepted ADR ownership; if implementation
  requires changing those decisions, return to spec approval.
