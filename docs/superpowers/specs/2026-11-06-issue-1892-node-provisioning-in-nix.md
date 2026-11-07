# Move Node dependency provisioning from devtool to Nix

## Outcome

The pinned Node dependency tree needed by the E2E census and TypeScript check is
available from Nix in a fresh checkout, without host `devtool` provisioning.
Census and TSC remain single-command workflows, and both continue using the
intended pinned packages.

## Approach and tradeoffs

Move ownership of `end2end/node_modules` setup out of `tools/devtool`: its Rust
provisioner, `provision-node-modules` CLI, TSC pre-step, devShell hook, and
census derivation caller are removed. Nix/package setup supplies the pinned
`e2ePackage` type dependencies and the separately pinned `@playwright/test`
where each consumer needs them. Keep the ordinary TSC and census entrypoints
reliable as single commands; Just may expose or orchestrate common setup, but
users must not need a manual setup sequence. Prefer native Nix/COTS setup over a
new maintained framework or redundant wrapper. Compare the complete maintained
Rust, Just, script, Nix, test and documentation surface for net simplification.

Use one small Nix-provided setup executable, with the pinned package paths
supplied by Nix, from all applicable entrypoints: devShell entry (for IDE use in
the starting checkout), the Just TSC workflow (so TSC also works after `cd` to a
fresh worktree in the already-running shell), and the census derivation (against
its own writable copied source under `$TMPDIR`, never the user's checkout). This
keeps one setup implementation instead of parallel shell and derivation scripts;
retain the `devtool check tsc` check definition but remove its provisioning
pre-step. The fresh-worktree TSC proof must use a worktree whose ignored
destination is absent, establish that absence before invoking the Just TSC
workflow, and prove the workflow provisions it.

Before creating/populating `end2end/node_modules`, the setup executable must
inspect the destination itself without following it and unlink a destination
symlink itself (never traverse or mutate its referent). For an ordinary existing
directory, preserve unrelated entries and refresh the current visible package
entries; skip dot entries, and replace `@playwright/test` with the separately
pinned Nix package as today. Do not promise cleanup of unrelated or obsolete
package names that are absent from the current closure. Repeated setup must be
safe. A focused setup regression must exercise absent destination, ordinary
directory with an unrelated sentinel, repeat invocation, and a stale destination
symlink to an external sentinel tree; verify expected current package links and
that the sentinels/referents remain unchanged. If the chosen Nix packaging
cannot expose the same setup executable to all three entrypoints without
introducing greater maintained complexity, return that mechanism choice to spec
approval rather than duplicating provisioning logic.

ADR-0028 defines `devtool` as an in-sandbox producer where Nix/xtask are
unavailable; it does not require devtool to own host shell or copied-source
dependency setup. ADR-0052/0146 continue to govern shared static-check command
definitions and remain unchanged: this issue removes provisioning, not check
definitions. Thus the transfer does not contradict those decisions and needs no
ADR amendment or architecture projection. ADR-0225 is precedent for Nix-owned
host tooling/provenance, not blanket authority; direct Nix dependencies are
appropriate here because the two consumers already receive these exact store
paths and setup has no Rust check semantics. Keep census population, census
verdict and their existing ownership unchanged; #1885 is closed, and its census
contract is not reopened by this provisioning change.

## Acceptance

- DevShell startup no longer calls `devtool provision-node-modules`; TSC and
  census callers no longer invoke the Rust provisioner.
- `tools/devtool/src/provision.rs`, its unit tests, provision CLI
  arguments/dispatch, and `tools/devtool/tests/provision_cli.rs` are removed,
  with no remaining provision-specific symbols or call sites.
- The same Nix-provided setup executable is used by shell entry, the Just TSC
  workflow, and `static-e2e-census`; each receives `${e2ePackage}/node_modules`
  and `${pkgs.playwright-test}/lib/node_modules/@playwright/test` directly. The
  helper is not duplicated in caller-specific shell snippets. The devtool TSC
  check remains the command-definition owner but no longer provisions
  dependencies.
- A focused setup test exercises absent destination, existing directory with
  preserved unrelated sentinel, repeat invocation, and a stale destination
  symlink to an external sentinel tree; it confirms the stale link is unlinked
  and the referent is unchanged. A fresh-worktree TSC invocation starts with
  absent `end2end/node_modules` and proves the Just workflow provisions it; the
  `static-e2e-census` derivation proves the copied-source consumer. Census
  population and verdict behavior remain unchanged.
- A repository-wide search finds no obsolete provisioning interface/caller. The
  maintained implementation/test/doc surface is simpler overall, with no new
  framework or duplicate census implementation.
- No ADR amendment is required: provisioning host/copy-tree dependencies is
  distinct from devtool's accepted in-sandbox and shared-check-definition
  responsibilities. If implementation evidence reveals a broader ownership
  change, stop and return it to spec approval rather than expanding scope.

## Boundaries

This issue changes only common pinned Node dependency provisioning and the three
identified call paths: the devShell hook, the E2E census derivation, and the TSC
check path. It does not redesign census membership, expected/executed verdicts,
E2E execution, package versions, Playwright/browser pinning, or the devtool's
unrelated in-sandbox responsibilities. It does not authorize broader xtask/Just
migration or implementation beyond this issue's accepted scope.
