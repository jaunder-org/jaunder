# Issue #29 — Migrate `scripts/with-ephemeral-postgres` into `devtool`

* Status: approved
* Date: 2026-06-27
* Issue: jaunder-org/jaunder#29 (Milestone 3 — Devtool migration)
* Governing ADR: [ADR-0028 — devtool/xtask boundary](../../adr/0028-devtool-vs-xtask-boundary.md)
  (explicitly places `with-ephemeral-postgres` #29 in `devtool` — in-sandbox)

## Problem

The coverage-pipeline Rust migration introduced `tools/devtool`, an in-sandbox bin
crate, and moved most of the coverage bash into it — but left `devtool coverage
emit` shelling out to `bash scripts/with-ephemeral-postgres` as a thin shim. This
issue removes that shim: the throwaway-PostgreSQL lifecycle (initdb / pg_ctl /
role+db creation / cleanup / env) moves into `devtool`, in Rust, and the bash
script is deleted.

### Scope correction (verified against the tree)

- The script's **only** programmatic caller is `tools/devtool/src/coverage/emit.rs:71`.
  The issue body says "used by the coverage `emit` pass and the e2e checks," but the
  Nix e2e checks (`mkE2ePostgresCheck`, `flake.nix:632`) stand up their **own**
  `services.postgresql` systemd service — they do **not** invoke this script. So #29
  touches `emit.rs`, the script file, and docs/comments only; no e2e wiring changes.
- `flake.nix:894` is a code **comment** referencing the script, not an invocation.
- `CONTRIBUTING.md:167` and `:306` show the script as a manual dev command.

### Latent gate gap discovered (in scope)

The `tools/` workspace tests **execute in no gate today**:
- `xtask` `host_tests.rs` runs only `cargo test --manifest-path xtask/Cargo.toml`.
- The `coverage` Nix check **excludes** `/tools/` from its source (`flake.nix:874`),
  and `devtoolBin` is built with `doCheck = false` (`flake.nix:342`).
- `static_checks.rs` runs `tools-clippy --all-targets` — which **compiles** the test
  code but never **runs** it.
- There is no separate `nextest` Nix check (the comment at `flake.nix:298` is stale;
  the real suite runs inside the `coverage` derivation, app workspace only).

Consequence: `emit.rs`'s existing four `#[cfg(test)]` tests never actually run.
Without fixing this, any test added for the new `pg` module would be equally dead.
Per the user's decision this fix is folded into #29 (not filed separately).

## Decisions (from brainstorming)

1. **emit ↔ pg boundary:** in-process API (`pg::with_ephemeral`) called directly by
   `emit::run`, plus a thin `devtool pg run -- <cmd>` CLI subcommand over the same
   module. No self-exec; one lifecycle implementation shared by both paths.
2. **Cleanup parity:** RAII `Drop` guard (normal exit + panic) **plus** a
   SIGINT/SIGTERM handler running the same teardown — full parity with the bash
   `trap cleanup EXIT INT TERM`, so a Ctrl-C during a manual `pg run` cannot orphan a
   cluster on the fixed port.
3. **Tests:** pure unit tests only for `pg` (no per-run cluster boot); the real
   end-to-end lifecycle stays exercised every run by the `coverage` `emit` pass (a
   broken `devtool pg` makes the `coverage` / `coverage-gate` check go red). A new
   `tools-test` xtask step gives those unit tests (and the dead `emit.rs` ones) a home.
4. **No new ADR:** ADR-0028 already governs the placement; cross-reference only.

## Design

### New module: `tools/devtool/src/pg.rs`

Sibling of `coverage/`. Owns the ephemeral PG 16 lifecycle. Public surface:

```rust
/// Connection endpoints handed to the wrapped command.
pub struct PgEnv {
    pub test_url: String,       // postgres://jaunder@127.0.0.1:<port>/jaunder
    pub bootstrap_url: String,  // postgres://postgres@127.0.0.1:<port>/postgres
}

/// Boot a throwaway PG 16 cluster, run `body` with its endpoints, tear down on exit.
pub fn with_ephemeral<T>(body: impl FnOnce(&PgEnv) -> anyhow::Result<T>) -> anyhow::Result<T>;
```

Lifecycle inside `with_ephemeral` (parity with the bash):
1. Create a unique temp `PGDATA` (`tempfile` crate, prefix `jaunder-pg.`).
2. `initdb -D <PGDATA> -U postgres -A trust --no-sync`.
3. `pg_ctl -D <PGDATA> -w start` with durability disabled and parity settings:
   `listen_addresses=127.0.0.1`, `port=<port>`, `unix_socket_directories=<PGDATA>`,
   `max_connections=200`, `fsync=off`, `full_page_writes=off`,
   `synchronous_commit=off`.
4. `psql` bootstrap (ON_ERROR_STOP): `CREATE ROLE jaunder LOGIN CREATEDB;` +
   `CREATE DATABASE jaunder OWNER jaunder;`.
5. Build `PgEnv` and call `body(&env)`.
6. Teardown (see below).

Port resolution: `JAUNDER_PG_TEST_PORT` env var, default `54329` (parity).

The env endpoints are **passed to the wrapped command**, never exported into
`devtool`'s own process environment.

### Cleanup / signals

- A `Cluster` guard holds `PGDATA` + port. Teardown is one centralized function:
  `pg_ctl -D <PGDATA> -m immediate stop` (best-effort) then remove the temp dir.
- `Cluster::drop` calls teardown → covers normal return and panic unwinding.
- A `signal-hook` handler thread for `SIGINT`/`SIGTERM` calls the same teardown, then
  restores the default disposition and re-raises so the process exits with the right
  status.
- An `AtomicBool` makes teardown idempotent so the signal path and `Drop` cannot
  double-fire.

New deps on `tools/devtool/Cargo.toml` (small, on the cache-eligible crate):
`tempfile`, `signal-hook`.

### CLI: `devtool pg run -- <cmd>…`

`main.rs` gains a `Pg` subcommand with a `Run` variant taking the trailing command
(`trailing_var_arg`). It calls `pg::with_ephemeral(|env| …)`, runs the child with the
two env vars set (inheriting stdio), and propagates the child's exit status.

### `emit::run` change

Replace the `bash scripts/with-ephemeral-postgres cargo llvm-cov … nextest` block
with an in-process call wrapping the `cargo llvm-cov … nextest` command:

```rust
let nextest = pg::with_ephemeral(|env| {
    run_capture(Command::new("cargo")
        .args(["llvm-cov", "--no-report", "nextest", "--show-progress", "none"])
        .env("JAUNDER_PG_TEST_URL", &env.test_url)
        .env("JAUNDER_PG_BOOTSTRAP_TEST_URL", &env.bootstrap_url))
})?;
```

Behavior is otherwise unchanged: a non-zero child exit is still non-fatal (recorded
via `status.json`); the captured combined output still feeds `classify_nextest_output`
and `diagnostics/nextest.log`.

### Deletions & doc/comment updates

- **Delete** `scripts/with-ephemeral-postgres`.
- `flake.nix` comment (~893–894): "via `scripts/with-ephemeral-postgres`" →
  "via `devtool pg`". No derivation logic changes; `emit` still calls the same binary
  and `postgresql_16` stays in `nativeBuildInputs`.
- `CONTRIBUTING.md:167` and `:306`: `scripts/with-ephemeral-postgres cargo …` →
  `devtool pg run -- cargo …`, with a one-line note on running `devtool`
  (`cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- …`).

### Tests + gate step

- **Pure unit tests** in `pg.rs` (no cluster): app/bootstrap URL builders, `initdb`
  argv, `pg_ctl` start argv + server-settings list, bootstrap SQL, port resolution
  (env override vs default).
- **New xtask step `tools-test`** in `xtask/src/steps/host_tests.rs`:
  `cargo test --manifest-path tools/Cargo.toml`, run alongside `xtask-tests` in every
  mode. This is the execution home for the `tools/` workspace tests (the new `pg`
  ones and the previously-dead `emit.rs` ones; the `tools/coverage` lib's tests come
  along for free).

## Out of scope

- No change to the e2e Nix checks (they use `services.postgresql`, not the script).
- No change to the `coverage` derivation logic (only a comment reword).
- No real-cluster integration test in the per-run suite (coverage check covers it).

## Verification

- `cargo xtask validate` (full: static + coverage + e2e). The `coverage` check
  exercises the real `devtool pg` lifecycle; `tools-test` runs the new unit tests;
  the deleted script must leave no dangling reference (`rg with-ephemeral-postgres`
  returns only archived docs).
