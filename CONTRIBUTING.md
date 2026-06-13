# Contributing to jaunder

This is the definitive working guide for both human contributors and coding agents. Agent-specific entry points such as `AGENTS.md` should point here instead of duplicating project policy.

## Project guides

- `docs/DESIGN.md`: project goals
- `docs/ARCHITECTURE.md`: project architecture
- `docs/ROADMAP.md`: project roadmap

## Repository layout

- `flake.nix`: development environment, comprehensive test environment, and PostgreSQL testing
- `common/`: code shared between other packages
- `end2end/`: Playwright end-to-end tests
- `hydrate/`: frontend driver
- `storage/`: storage traits, records, migrations, and backend-specific storage support
- `server/`: backend, CLI, server runner, and integration tests
- `web/`: Leptos server functions in `web/src/*.rs` and page components in `web/src/pages/`

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

With that environment set, `jaunder init`, `jaunder serve`, and targeted storage tests will use the PostgreSQL test VM instead of SQLite.

If a PostgreSQL URL omits the password, `jaunder` also supports:

```bash
export JAUNDER_DB_PASSWORD=secret
# or
export JAUNDER_DB_PASSWORD_FILE=/run/secrets/jaunder-db-password
```

Those inputs are for steady-state connections. PostgreSQL database and role provisioning is handled separately by `jaunder create-pg-db`, which is intended for one-time administrative bootstrap before `jaunder init` runs migrations.  Because `create-pg-db` is meant to be run manually by an experienced administrator, it takes its inputs explicitly via command-line flags rather than via `JAUNDER_*` environment variables.  The command is intentionally simple: bootstrap URL, application database URL, and a separate application-role password. It fails if the requested role or database already exists.  Use `--bootstrap-db` for the elevated connection and `--app-db` for the long-term application connection details.

### Git hooks

The repository includes git hooks in `.githooks/` that enforce code quality standards. Configure git to use them after cloning:

```
git config core.hooksPath .githooks
```

**`pre-commit`** runs on every commit — fast formatting, lint, and the SQLite test suite:

- `leptosfmt --check`, `cargo fmt --check`, `prettier --check end2end` — formatting
- `cargo clippy -- -D warnings` — linting
- `cargo nextest run` — unit and integration tests (SQLite)

**`pre-push`** runs `scripts/verify` (the commit gate): the static checks plus `scripts/check-coverage --check` (the SQLite + host-PostgreSQL suites under instrumentation, gating both test failures and coverage regressions) and the host e2e suite. The hermetic Nix VM checks are not run here — they run in CI, or locally via `scripts/verify --full`.

To skip the pre-push gate on a WIP push:

```
SKIP_PRE_PUSH=1 git push
```

## Development workflow

- Track development work with `beads` (`bd`) rather than ad-hoc markdown TODO lists. Use beads for durable memory items when possible.
- Prefer focused, atomic changes. The system should remain in a working state at each commit.
- Write and commit preparatory refactors before the behavior changes that use them.
- Every module should have comprehensive internal documentation.
- Avoid `cat >>` or other append-only shell edits when modifying files; use structured editing tools.
- Address all `clippy` lints.
- Unless explicitly instructed otherwise, request review before committing.

## Testing

There are several testing layers in this repository. Use the smallest one that matches the change first, then move up to the broader checks before pushing.  Never remove functionality to pass tests, and never bypass or suppress testing, coverage, or linting without explicit approval.

Every HTTP endpoint must have both an integration test and an end-to-end test.  Unit tests belong in the same file as the code being tested. End-to-end tests belong in `end2end/` and use Playwright.

For tests requiring a database, use `sqlite::memory:` and run migrations with `sqlx::migrate!("./migrations").run(&pool).await?` before creating the `AppState`.

### Fast local checks

The verify ladder has three tiers:

- `scripts/verify --fast` — static checks (fmt, leptosfmt, prettier, cargo-deny) and clippy only. Quick inner-loop feedback; does not certify a change.
- `scripts/verify` — the commit (pre-push) gate: the `--fast` checks plus `scripts/check-coverage --check` and the host end-to-end suite (`scripts/e2e-local.sh`). `check-coverage` runs the SQLite + host-PostgreSQL suites under instrumentation, so it doubles as the test pass/fail check **and** gates coverage regressions — so test failures, coverage regressions, and e2e breakage are all caught locally, not deferred to CI. `--check` is compare-only (it never rewrites the committed baseline). **No VM.**
- `scripts/verify --full` — the commit gate plus the hermetic Nix VM checks (`nix-only-checks`: e2e + the consolidated PostgreSQL integration VM). The VM e2e re-runs the end-to-end suite more thoroughly (both browsers, both backends), so the host e2e suite is **skipped** under `--full` rather than run twice. The VM checks also run in CI, so running `--full` locally is optional.
  - By default it prints only `--- verify: ... ---` progress markers and captures step output.
  - Set `VERIFY_PASSTHROUGH=1` to stream full tool output directly.
  - Set `VERIFY_SHOW_STEP_OUTPUT=1` to print captured output for successful steps.
  - Set `VERIFY_SHOW_FAILURE_LOG=0` to suppress failed-step logs, or `VERIFY_FAILURE_LOG_LINES=<n>` to change the failure tail length (default `200` lines).
- `cargo fmt --check` checks Rust formatting.
- `leptosfmt -x .direnv -x .git -x target --check '**/*.rs'` checks files that contain Leptos `view!` macros.
- `prettier --check end2end` checks Playwright and other frontend test assets.
- `cargo clippy -- -D warnings` checks the main workspace for lint errors.
- `cargo nextest run` runs the default Rust unit and integration test suite.
- For e2e perf diagnostics, set `JAUNDER_E2E_WARMUP=1` before `playwright test` to warm each test page context before test instrumentation; tune with `JAUNDER_E2E_WARMUP_URL` and `JAUNDER_E2E_WARMUP_TIMEOUT_MS`.
- In hydration-heavy e2e tests, use `hydrationHeavyTimeoutMs(testInfo, chromiumBudgetMs)` for whole-test budgets and `hydrationHeavyFirstNavigationTimeoutMs(testInfo, chromiumBudgetMs)` for first navigation waits (see [ADR-0012](docs/decisions/0012-environment-aware-timeouts.md)).

### Observability and Performance Analysis

Jaunder uses OpenTelemetry for deep performance analysis (see [ADR-0011](docs/decisions/0011-unified-observability.md)).

- **Trace Analysis**: Use `scripts/analyze-otel-traces` to process trace artifacts (JSONL) from VM runs or local tests.
  ```bash
  scripts/analyze-otel-traces \
    /path/to/otel-traces-sqlite.jsonl/otel-traces.jsonl \
    /path/to/otel-traces-postgres.jsonl/otel-traces.jsonl
  ```
  The analyzer reports on:
  - Slowest spans overall and per `e2e.test`.
  - Top navigation and action hotspots.
  - `commit -> hydration` splits by `cacheWarmth`.
  - Hydration component hotspots (`wasm_init`, `leptos_hydrate`, etc.).

  **Optional Filters**:
  - `--top N`: Limit the number of rows in each section.
  - `--trace TRACE_ID`: Analyze a specific trace.
  - `--project NAME`: Filter by browser (e.g., `firefox`, `webkit`).

- **WASM Audit**: Use `scripts/audit-wasm-bundle` to measure the size of the frontend WASM and JS bundles from the deterministic Nix build.
  ```bash
  scripts/audit-wasm-bundle
  ```
  Useful options: `--json` for machine-readable output, or `--site-path` to reuse a build.

- **Run & Analyze**: Use `scripts/run-e2e-trace-analysis` to run the full VM e2e suite and immediately analyze the results.
  - Use `--cold` to run against cold caches instead of the default warmup checks.

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

`cargo nextest list -p jaunder --tests` shows the currently registered Rust test targets if you need to confirm the target split.

### PostgreSQL-backed Rust tests

The integration suite is backend-parametric: it runs against SQLite by default, and against PostgreSQL when `JAUNDER_PG_TEST_URL` is set. Each test creates its own database — a clone of a once-migrated template (see `server/tests/helpers/mod.rs`) — so the PostgreSQL tests run **in parallel** just like the SQLite ones. No `--test-threads=1` is needed.

The simplest way to run them against a throwaway PostgreSQL is the wrapper, which starts an ephemeral cluster, exports the connection env, runs the command, and tears everything down:

```bash
scripts/with-ephemeral-postgres cargo nextest run -p jaunder --run-ignored all
```

To use a persistent instance instead (e.g. the dev VM), set the env yourself:

```bash
nix run .#postgres-testing-vm
export JAUNDER_PG_TEST_URL=postgres://jaunder@127.0.0.1:55432/jaunder
export JAUNDER_PG_BOOTSTRAP_TEST_URL=postgres://postgres@127.0.0.1:55432/postgres
cargo nextest run -p jaunder --run-ignored all
```

Per-test databases are not dropped after each run, so a persistent instance accumulates `jaunder_test_*` databases over time; the ephemeral wrapper avoids this by discarding the whole cluster.

### Coverage and dependency policy

- `scripts/check-coverage` enforces the coverage requirement. The default `scripts/verify` gate runs it as `--check` (compare-only — gates regressions without rewriting the committed baseline), and CI runs it too, so coverage regressions are caught before push rather than at merge. The baseline is regenerated deliberately from the Nix sandbox (the CI-reproducible environment), not by local runs.
- `scripts/check-coverage --check` compares against the baseline and fails on a regression without updating it (used by the verify gate); plain `scripts/check-coverage` also updates the baseline on success.
- `scripts/check-coverage --investigate` provides detailed information about missing coverage.
- `cargo deny check` verifies dependency policy, advisories, and licensing.

Coverage is measured by `scripts/check-coverage`, which counts source lines with at least one execution hit across all test binaries. This deduplicates generic-function instantiations across compile units, unlike the inflated `cargo llvm-cov --json` summary percentages for code exercised by multiple test binaries.

Coverage runs in two passes that accumulate into one merged report: the whole workspace against SQLite, then the `jaunder` integration tests against a throwaway host PostgreSQL (via `scripts/with-ephemeral-postgres`), so `storage/src/postgres/*` gets real instrumented coverage. Because the Nix `coverage` build sandbox has no network, a few network-sensitive files (`common/src/websub/http.rs`, `server/src/commands.rs`) report slightly lower there than on a networked host. **The committed baseline must therefore be generated from the Nix `coverage` check (the CI environment), not from a local host run** — a host run would raise the baseline above what CI can reproduce and break the gate. To regenerate it, build the `coverage` check with `--update` semantics and copy out its manifests (see `jaunder-uox1`).

The baseline is stored in `.coverage-manifest.json`. Never lower or update it without user approval; approved changes to the baseline must be committed in the same commit as the file whose coverage changed. Coverage improvements are always allowed.

Some areas have inherent host-side coverage gaps and should not be force-fitted with artificial tests:

- **WASM entry point** (`hydrate/src/lib.rs`, 0%): runs only in the browser WASM context.
- **Leptos page components** (`web/src/pages/*.rs`, varied): `#[component]` functions render view trees; correctness is validated by e2e tests in the Nix VM.
- **A few PostgreSQL storage paths** (`storage/src/postgres/media.rs` ~5%, plus some error branches in `sessions`/`users`/`invites`): the integration suite drives the common paths but not every branch. The bulk of `storage/src/postgres/*` and `storage/src/backup/postgres.rs` is now measured by the host-PostgreSQL coverage pass (formerly a blanket 3–15% gap).
- **Asset serving** (`server/src/assets.rs`, 0%): compile-time embedded assets are not practical to unit test.

### Nix VM checks

`nix flake check` runs the full Nix-backed validation matrix, including:

- `checks.x86_64-linux.nextest` — Rust nextest suite
- `checks.x86_64-linux.clippy` — clippy
- `checks.x86_64-linux.rustfmt` — rustfmt
- `checks.x86_64-linux.leptosfmt-check` — leptosfmt
- `checks.x86_64-linux.prettier-check` — prettier for `end2end/`
- `checks.x86_64-linux.deny` — cargo-deny
- `checks.x86_64-linux.e2e-sqlite` — Playwright end-to-end flow against SQLite with `JAUNDER_E2E_WARMUP=1` (default)
- `checks.x86_64-linux.e2e-postgres` — Playwright end-to-end flow against PostgreSQL with `JAUNDER_E2E_WARMUP=1` (default)
- `checks.x86_64-linux.postgres-integration` — every `server/tests/*.rs` integration binary against PostgreSQL (including the ignored PostgreSQL-only cases), all in one VM

Additional Nix-backed checks available as packages (not run by default):

- `packages.x86_64-linux.e2e-sqlite-cold` — Playwright end-to-end flow against SQLite without warmup
- `packages.x86_64-linux.e2e-postgres-cold` — Playwright end-to-end flow against PostgreSQL without warmup

All PostgreSQL integration binaries run in a **single** VM (`postgres-integration`): per-test databases isolate the tests, so they run with libtest's normal in-process parallelism rather than one VM per binary. This is much faster and far lighter on memory than the former per-binary matrix.

If you only need one of the VM-backed checks, you can run it directly:

```bash
nix build .#checks.x86_64-linux.e2e-sqlite
nix build .#checks.x86_64-linux.e2e-postgres
nix build .#packages.x86_64-linux.e2e-sqlite-cold
nix build .#packages.x86_64-linux.e2e-postgres-cold
nix build .#checks.x86_64-linux.postgres-integration
```

## Code conventions

- Use Rust, except end-to-end tests, which use Playwright and TypeScript.
- All Rust code is formatted with `cargo fmt`.
- Files containing Leptos `view!` macros are additionally formatted with `leptosfmt` (run it first, then `cargo fmt`).
- Commits reference the milestone item they address, e.g. `M0.1.1: Rename app/ to web/`.
- Every commit must include appropriate tests unless the user explicitly waives this requirement.
- Never use `.unwrap()`.
- Never use `.expect()` in production code.
- Use Rust's type system to make invalid states impossible with infallible types.
- At boundaries (`#[server]` functions, DB calls), parse data into infallible types, reject invalid data, and handle `Result`/`Option` conversion explicitly.
- Keep data transformations pure where possible so they are easy to test and reason about.

## Storage and web conventions

- Use `sqlx` for database access.
- Support SQLite and PostgreSQL, dynamically selected at runtime.
- Store SQL migrations in `storage/migrations` using the `000x_description.sql` numbering convention.
- Define storage traits such as `UserStorage` and `SessionStorage`, plus their record types, in `storage/src/*.rs`. This lets `web` and `server` use them without circular dependencies.
- Keep concrete SQLite/PostgreSQL implementations in the `server` crate, for example `server/src/storage/sqlite.rs`, and re-export them from `server/src/storage/mod.rs` for the CLI and server runner.
- Use specialized storage error enums in `common::storage`, such as `UserAuthError` and `CreateUserError`, with `thiserror`.
- Use `sqlx` unique violation checks (`is_unique_violation()`) to handle "already exists" errors gracefully.
- Use the `AppState` struct from `common::storage` to bundle storage handles. In web server functions, retrieve it with `expect_context::<Arc<AppState>>()`.
- **Dependency injection / composition-root invariant (see [ADR-0016](docs/decisions/0016-dependency-injection-and-appstate.md)):** No type may be both (a) a heterogeneous dependency holder and (b) passed beyond the composition root. Declare a component's dependencies as constructor parameters on the component that uses them — do not add a field to a shared bundle to make a dependency reachable. A storage `Backend` factory may mint storage handles, but only the composition root may hold it; it is never injected into a subsystem (that would be a service locator). Services (mailer, WebSub client, background workers) are constructed at the root and injected per-consumer; there is no "services bundle."
- The web framework is Leptos with SSR via `cargo-leptos`.
- Leptos components should only render data; business logic belongs in server functions or pure transformation functions.
- API methods are automatically prefixed with `/api`.
- Define `#[server]` functions in the relevant `web/src/*.rs` module, such as `web/src/auth.rs`. Use `#[cfg(feature = "ssr")]` for server-only imports and logic in those files.
- Convert storage errors to `leptos::prelude::ServerFnError` using `.map_err(|e| ServerFnError::new(e.to_string()))`.
- Use `require_auth().await?` at the start of any server function that requires a logged-in user. It returns an `AuthUser` containing `user_id`, `username`, and `token_hash`.
- Use `set_session_cookie(raw_token)` and `clear_session_cookie()` from `web/src/auth.rs` to manage the `session` cookie in server functions.
- Enforce lowercase usernames at the boundary before passing them to storage methods.
- Production deployment is expected to run behind a reverse proxy providing HTTPS.

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
- The PostgreSQL-backed tests each create their own database (template clones), so they run in parallel without interference; see "PostgreSQL-backed Rust tests" above. For a self-contained run, prefer `scripts/with-ephemeral-postgres`.
