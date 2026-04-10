# Contributing to jaunder

## Development setup

### Prerequisites

This project uses [Nix](https://nixos.org/) to manage the development environment. All required tools are provided by the Nix flake.

Enter the development shell:

```
nix develop
```

The default local backend remains SQLite. PostgreSQL development is also supported:

```bash
nix run .#postgres-testing-vm
export JAUNDER_DB=postgres://jaunder@127.0.0.1:55432/jaunder
```

With that environment set, `jaunder init`, `jaunder serve`, and targeted storage tests will use the
PostgreSQL test VM instead of SQLite.

If a PostgreSQL URL omits the password, `jaunder` also supports:

```bash
export JAUNDER_DB_PASSWORD=secret
# or
export JAUNDER_DB_PASSWORD_FILE=/run/secrets/jaunder-db-password
```

Those inputs are for steady-state connections. PostgreSQL database and role
provisioning is expected to be handled separately by an experienced
administrator; `jaunder init` still assumes the target database already exists.

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

## Backend parity rules

- Any change that adds persisted state must include both a SQLite migration and a PostgreSQL migration in the same change.
- Any change that alters a storage trait or persisted behavior must implement the change on both backends before merge.
- New storage-backed tests must either cover both backends or state explicitly why one backend is intentionally deferred.
- Backend-specific optimizations are allowed, but user-visible behavior differences must be documented explicitly up front.

## NixOS integration

- The shared NixOS module is `nixosModules.jaunder`.
- Production imports should enable the service with `services.jaunder.enable = true;` and set `services.jaunder.bind` as needed.
- Set `services.jaunder.db` to choose the backend for a NixOS deployment. The default remains `sqlite:./data/jaunder.db`.
- Do not set `JAUNDER_MAIL_CAPTURE_FILE` in production. That is test-only and should stay in the interactive VM or e2e test node config.
- The `jaunder` CLI is installed for the `jaunder` user via `users.users.jaunder.packages`.

## Interactive testing VM

- Start it with `nix run .#interactive-testing-vm`.
- It auto-logs in as `jaunder` on the console.
- The VM user password is `jaunder`.
- `sudo` is passwordless for `wheel` in the VM only.
- The VM does not use SSH; it is intended for local console interaction and app testing.

## PostgreSQL test VM

- Start it with `nix run .#postgres-testing-vm`.
- The forwarded connection string is `postgres://jaunder@127.0.0.1:55432/jaunder`.
- Point the app at it with `export JAUNDER_DB=postgres://jaunder@127.0.0.1:55432/jaunder`.
- PostgreSQL storage tests in `server/tests/storage.rs` reset a shared schema. Run those tests individually, or with `-- --test-threads=1`, to avoid cross-test interference.
