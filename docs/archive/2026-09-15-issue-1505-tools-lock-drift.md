# Issue 1505: Prevent tools lock drift from blocking commits

## Outcome

A stale `tools/Cargo.lock` fails before a tools-workspace command can rewrite
the contributor's tree. With the committed lock repaired, ordinary pre-commit
checks launch cleanly and leave unrelated staged work untouched.

## Load-bearing decisions

- Repair the existing `host -> rustix` dependency edge in `tools/Cargo.lock`.
- Every xtask launch of `tools/devtool` through `cargo run` passes Cargo's
  `--locked` option: static checks, local tests, CSR bundling, and local e2e
  seeding.
- Lock enforcement belongs at each existing xtask-to-devtool launch seam,
  matching ADR-0029's locked outer xtask launch.
- A stale tools lock is a command failure, not a mutation to reconcile or
  auto-stage.
- Existing precommit staging safety remains unchanged.
- Exact command-shape unit tests protect the locked launch contract.
- No broader lockfile-consistency subsystem is introduced.

## Acceptance

- The tools lock records `rustix` in the `host` package dependency list.
- The minimized `prettier-markdown` devtool command succeeds from a clean
  checkout and leaves `git status --short` empty.
- Removing the repaired lock edge makes the same launch fail under `--locked`
  without rewriting the lockfile.
- Exact command-shape tests prove `--locked` is present for static-check Check
  and Fix modes, local tests, CSR bundling, and local e2e seeding.
- `cargo xtask precommit` no longer fails staging reconciliation solely because
  it launched a tools-workspace command.

## Boundaries

- Do not weaken, bypass, or broaden precommit staging reconciliation.
- Do not auto-stage lockfiles or any unrelated path.
- Do not change devtool's owned check definitions or the Nix execution contract.
- Do not add a repository-wide lockfile scanner or alter other Cargo workspace
  boundaries.
