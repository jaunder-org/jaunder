# Contributing to jaunder

## Development setup

### Prerequisites

This project uses [Nix](https://nixos.org/) to manage the development environment. All required tools are provided by the Nix flake.

Enter the development shell:

```
nix develop
```

### Git hooks

The repository includes git hooks in `.githooks/` that enforce code quality standards. Configure git to use them after cloning:

```
git config core.hooksPath .githooks
```

**`pre-commit`** runs on every commit:
- `cargo fmt --check` — formatting
- `cargo clippy -- -D warnings` — linting
- `cargo nextest run` — unit and integration tests
- `cargo llvm-cov nextest` — 100% line coverage check

**`pre-push`** runs on every push:
- `nix flake check` — end-to-end tests

To skip e2e tests on a WIP push:

```
SKIP_E2E=1 git push
```

## Code conventions

- All Rust code is formatted with `cargo fmt`.
- Files containing Leptos `view!` macros are additionally formatted with `leptosfmt` (run it first, then `cargo fmt`).
- Commits reference the milestone item they address, e.g. `M0.1.1: Rename app/ to web/`.
- Every commit must include appropriate tests; coverage must remain at 100%.
