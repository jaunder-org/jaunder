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
provisioning is handled separately by `jaunder create-pg-db`, which is intended
for one-time administrative bootstrap before `jaunder init` runs migrations.
Because `create-pg-db` is meant to be run manually by an experienced
administrator, it takes its inputs explicitly via command-line flags rather than
via `JAUNDER_*` environment variables.
The command is intentionally simple: bootstrap URL, application database URL,
and a separate application-role password. It fails if the requested role or
database already exists.
Use `--bootstrap-db` for the elevated connection and `--app-db` for the
long-term application connection details.

### Git hooks

The repository includes git hooks in `.githooks/` that enforce code quality standards. Configure git to use them after cloning:

```
git config core.hooksPath .githooks
```

**`pre-commit`** runs on every commit:
- `cargo fmt --check` — formatting
- `cargo clippy -- -D warnings` — linting
- `cargo nextest run` — unit and integration tests

**`pre-push`** runs on every push:
- `cargo deny check` — dependency and advisory policy
- `scripts/check-coverage` — coverage gate
- `nix flake check` — end-to-end tests

To skip e2e tests on a WIP push:

```
SKIP_E2E=1 git push
```

## Testing

There are several testing layers in this repository. Use the smallest one that
matches the change first, then move up to the broader checks before pushing.

### Fast local checks

- `scripts/verify` runs the full local verification sequence in the preferred order: formatting, build, tests, lint, coverage, then `nix flake check`.
  - By default it prints only `--- verify: ... ---` progress markers and captures step output.
  - Set `VERIFY_PASSTHROUGH=1` to stream full tool output directly.
  - Set `VERIFY_SHOW_STEP_OUTPUT=1` to print captured output for successful steps.
  - Set `VERIFY_SHOW_FAILURE_LOG=0` to suppress failed-step logs, or `VERIFY_FAILURE_LOG_LINES=<n>` to change the failure tail length (default `200` lines).
- `cargo fmt --check` checks Rust formatting.
- `leptosfmt -x .direnv -x .git -x target --check '**/*.rs'` checks files that contain Leptos `view!` macros.
- `prettier --check end2end` checks Playwright and other frontend test assets.
- `cargo clippy -- -D warnings` checks the main workspace for lint errors.
- `cargo nextest run` runs the default Rust unit and integration test suite.

### Targeted Rust tests

When a change is confined to one area, run the relevant target directly.

- CLI and command behavior: `cargo test -p jaunder --test commands`
- Storage behavior: `cargo test -p jaunder --test storage`
- Web/server-function behavior:
  - `cargo test -p jaunder --test web_auth`
  - `cargo test -p jaunder --test web_account`
  - `cargo test -p jaunder --test web_email`
  - `cargo test -p jaunder --test web_password_reset`
- Library-only tests: `cargo test -p jaunder --lib`

`cargo nextest list -p jaunder --tests` shows the currently registered Rust
test targets if you need to confirm the target split.

### PostgreSQL-backed Rust tests

Some ignored tests require a live PostgreSQL instance.

For local development:

```bash
nix run .#postgres-testing-vm
export JAUNDER_PG_TEST_URL=postgres://jaunder@127.0.0.1:55432/jaunder
export JAUNDER_PG_BOOTSTRAP_TEST_URL=postgres://postgres@127.0.0.1:55432/postgres
```

With those set, you can run the PostgreSQL-only ignored tests:

```bash
cargo test -p jaunder --test commands -- --ignored --test-threads=1
cargo test -p jaunder --test storage -- --ignored --test-threads=1
```

Those tests share one database and reset schema state between cases, so run
them individually or with `--test-threads=1`.

### Coverage and dependency policy

- `scripts/check-coverage` enforces the coverage requirement used by `pre-push`.
- `cargo deny check` verifies dependency policy, advisories, and licensing.

### Nix VM checks

`nix flake check` runs the full Nix-backed validation matrix, including:

- `checks.x86_64-linux.nextest` — Rust nextest suite
- `checks.x86_64-linux.clippy` — clippy
- `checks.x86_64-linux.rustfmt` — rustfmt
- `checks.x86_64-linux.leptosfmt-check` — leptosfmt
- `checks.x86_64-linux.prettier-check` — prettier for `end2end/`
- `checks.x86_64-linux.deny` — cargo-deny
- `checks.x86_64-linux.e2e-sqlite` — Playwright end-to-end flow against SQLite
- `checks.x86_64-linux.e2e-postgres` — Playwright end-to-end flow against PostgreSQL
- `checks.x86_64-linux.postgres-commands` — `server/tests/commands.rs` against PostgreSQL, including ignored PostgreSQL-only cases
- `checks.x86_64-linux.postgres-storage` — `server/tests/storage.rs` against PostgreSQL, including ignored PostgreSQL-only cases
- `checks.x86_64-linux.postgres-web-account` — `server/tests/web_account.rs` against PostgreSQL
- `checks.x86_64-linux.postgres-web-auth` — `server/tests/web_auth.rs` against PostgreSQL
- `checks.x86_64-linux.postgres-web-email` — `server/tests/web_email.rs` against PostgreSQL
- `checks.x86_64-linux.postgres-web-password-reset` — `server/tests/web_password_reset.rs` against PostgreSQL

The PostgreSQL VM checks are split by Rust test binary rather than by
individual test case. That keeps the flake readable while still making slow
or failing PostgreSQL coverage easy to localize.

If you only need one of the VM-backed checks, you can run it directly:

```bash
nix build .#checks.x86_64-linux.e2e-sqlite
nix build .#checks.x86_64-linux.e2e-postgres
nix build .#checks.x86_64-linux.postgres-commands
nix build .#checks.x86_64-linux.postgres-storage
nix build .#checks.x86_64-linux.postgres-web-auth
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
