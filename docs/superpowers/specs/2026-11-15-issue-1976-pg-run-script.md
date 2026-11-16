# #1976 — pg-run as a bash script (language pilot)

## Outcome

`pg-run [--] <command> [args…]` runs a command against a throwaway PostgreSQL
cluster reached over a Unix socket. It is one bash script, packaged by Nix and
on PATH in the devShell and the coverage sandbox. The 18-line Rust binary and
the ~390-line `coverage::ephemeral_postgres` library are deleted. The issue
records a before/after comparison and a keep-or-revert verdict for the language
review.

## Approach and tradeoffs

**Script.** `scripts/pg-run.sh` does the following, in order:

1. Create PGDATA with `mktemp -d` under `TMPDIR` (checkout-local, per #1811).
   Create a separate short socket directory with
   `mktemp -d "${XDG_RUNTIME_DIR:-$TMPDIR}/jaunder-pg.XXXXXX"`.
2. Run `initdb` (trust auth, `--no-sync`, superuser `postgres`).
3. Append the server settings to `postgresql.conf`. Writing them to the config
   file avoids `pg_ctl -o` shell quoting:
   - `listen_addresses=''` (no TCP)
   - `unix_socket_directories='<sockdir>'`
   - `max_connections=200`
   - `fsync`, `full_page_writes` and `synchronous_commit` off
4. Start with `pg_ctl -w -l "$PGDATA/server.log" start`. The server log stays
   off the command's stdout and stderr.
5. Create the `jaunder` role and database with `psql`. Setup steps send their
   stdout to `/dev/null`.
6. Export
   `JAUNDER_PG_TEST_URL=postgres://jaunder@localhost/jaunder?host=<sockdir>` and
   `JAUNDER_PG_BOOTSTRAP_TEST_URL=postgres://postgres@localhost/postgres?host=<sockdir>`.
7. Run the command.

Interface: one leading `--` is accepted and dropped, so the issue's
`just pg-run -- true` works. With no command, pg-run prints usage and exits 2.

The script is plain sequencing except for these points:

- **Running the command:** it runs as `"$@" <&0 &` followed by `wait`. A
  backgrounded command lets pg-run's traps fire promptly. The explicit `<&0`
  keeps stdin, which non-interactive bash otherwise points at `/dev/null` for
  background jobs. The exit code comes from re-waiting until the child has
  really exited, because a trapped signal makes `wait` return early.
- **Signals:** on SIGINT or SIGTERM, pg-run sends SIGTERM to the command
  (background children ignore SIGINT) and waits for it. It then tears down,
  removes its traps and re-raises the original signal on itself, so it dies by
  that signal.
- **Teardown:** `pg_ctl -m immediate stop`, then `rm -rf` PGDATA and the socket
  directory. It runs from the EXIT trap and the signal path. Each step is
  guarded with `|| warn`, because `writeShellApplication` turns on errexit. A
  failed step prints a warning that names the step but not the path, and never
  changes the exit status.

**Why Unix sockets, not TCP.** Each cluster has its own socket directory, so the
port number is a constant and there is nothing to choose, race for or retry.
That removes port selection and the 8-attempt retry entirely.

- **Path limit.** Socket paths are limited to 107 bytes. Worktrunk worktrees sit
  at `~/src/jaunder/<branch>`, and branch names alone can overflow that if the
  socket lives under the checkout. So the socket goes in its own short directory
  under `XDG_RUNTIME_DIR` (here `/run/user/1000`). Where that is unset it falls
  back to `TMPDIR`: the Nix sandbox's `/build` and the CI checkout are both
  short.
- **Exception to #1811.** One socket directory, holding the socket and its lock
  file, sits outside the checkout. It is deleted at teardown.
- **Accepted failure modes.** Startup fails loudly, with PostgreSQL's own error,
  if the socket path is still too long. It also fails if the directory path
  contains `+`, `&` or `%`, which the URL query would misread.
- **Precedent.** Production already uses this URL form
  (`?host=/run/postgresql`).

This deliberately overrides three items in the issue's "Behavior to preserve":
the loopback TCP port, the bind-race retry and "no Unix socket directory". All
of them exist only to support TCP.

**Why bash.** The job is a fixed process sequence plus "always clean up", which
is what `trap` does. The coverage sandbox already has bash. Rejected:

- Python adds python3 to the sandbox and rebuilds signal handling by hand.
- `postgresqlTestHook` runs only in a Nix check phase, so host callers would
  need a second implementation.
- `ephemeralpg`'s `pg_tmp` deletes clusters on a timer from a detached process,
  not when the command exits, and leaves pre-warmed clusters in `/tmp`.

**Delivery.** `nix/packages.nix` builds the script with `writeShellApplication`
(shellcheck runs at build time), with `runtimeInputs = [ pkgs.postgresql_18 ]`
only, matching the devShell. It is added to the shared devShell inputs (the
default, CI and mutants shells) and to the coverage derivation's
`nativeBuildInputs`, replacing that derivation's explicit `postgresql_18`.
Editing the script needs a devShell reload, the same as
`setup-e2e-node-modules`.

| Consumer          | Today                                      | After                                       |
| ----------------- | ------------------------------------------ | ------------------------------------------- |
| `just pg-run`     | `cargo run … --bin pg-run --`              | `pg-run "$@"`                               |
| `test-local`      | `cargo run … pg-run -- cargo nextest run`  | `pg-run cargo nextest run …`                |
| mutants CI        | `nix build .#host-proof` → `bin/pg-run --` | `pg-run` inside `nix develop .#mutants`     |
| coverage producer | in-process `with_ephemeral` in `emit.rs`   | buildPhase `pg-run devtool coverage emit …` |

**Coverage consequence (accepted).** Today, a Postgres start failure inside emit
still yields a red `status.json` and diagnostics in `$out`. Afterwards it fails
the Nix build, and the cause appears only in the build log. Test failures still
produce a red `status.json` as today.

**Removed:**

- `ephemeral_postgres.rs` and coverage's `ephemeral-postgres` feature, with its
  now-unused `signal-hook` dependency. `tempfile` stays, because the `host`
  feature still uses it.
- devtool's use of that feature.
- From host-proof: the `pg-run` bin, its integration test and the whole
  `coverage` dependency.
- `pg-run` from the `.#host-proof` package bins.
- The `nix/checks.nix` source assertions about the deleted file.

The host-proof orchestration tests switch from a fake `cargo` to a fake
`pg-run`. Stale comments and docs naming "host-proof `pg-run`" are updated:
`.mutants-loop/{ci-shard,common}.sh`, `mutants.yml`, `nix/packages.nix`,
host-proof's `lib.rs`, coverage's `Cargo.toml`, `CONTRIBUTING.md` (four places)
and `docs/ARCHITECTURE.md`.

**Test helpers.** `server/tests/misc/postgres/commands.rs` (two tests) rebuilds
an app URL from the bootstrap URL's `host:port` and drops the query, which loses
`?host=`. It must keep the query string. No product code changes.

**Recorded decision.** A `jaunder-adr` draft records bash plus Unix sockets for
pg-run, and amends ADR-0246's "one PG implementation" clause and ADR-0247's
mention of pg-run. Numbered ADR bodies are not edited. CONTRIBUTING's language
rule gains: shell for small process-orchestration wrappers whose job is a fixed
command sequence with trap-based cleanup.

## Acceptance

Proof to execute:

1. Smoke test under `just pg-run`:
   - both URLs connect as `jaunder`/`postgres` over the socket;
   - `SHOW listen_addresses` is empty;
   - the setup steps add nothing to the command's stdout;
   - interactive stdin works
     (`echo 'select 1' | pg-run psql "$JAUNDER_PG_TEST_URL"`);
   - with no arguments it exits 2.
2. `just test-local` passes, Postgres-backed tests included.
3. The coverage Nix check and `coverage-gate` pass.
4. The mutants Postgres path runs: `pg_isready` succeeds under `pg-run` in
   `nix develop .#mutants`, shown on one small local shard or by CI on the PR.
5. `just validate-no-e2e` is green.

Pilot report, posted as an issue comment (not committed). The "before"
measurements are taken on `origin/main` before any code change:

- non-test and test LOC, before vs after;
- time from a fresh worktree with an empty `target/` to the first
  `just pg-run -- true`, and warm startup time;
- SIGINT, and SIGTERM sent to pg-run alone: cleanup, exit status, orphaned
  processes, leftover PGDATA or socket directory;
- a failing command (`pg-run false`, exit-code propagation);
- a forced teardown failure (stop the server from inside the command): expect a
  warning without the path, an unchanged exit status, and both directories
  removed;
- the bind race: before, by forcing a port collision; after, not applicable by
  construction;
- anything the script handles worse, and a keep-or-revert verdict.

Deliverables to commit: the script and its Nix package, consumer rewiring, the
removals above, the test-helper fix, the ADR draft and the doc updates.

## Boundaries

- No dedicated test suite for the script (user decision). It is exercised by
  test-local, the coverage producer and mutants, with shellcheck at build. The
  signal and failure behaviors are shown once in the pilot report.
- No TCP fallback, no port override, no change to the e2e/VM Postgres setups,
  and no change to the coverage derivation's phase structure.
- The wider language review is out of scope; this only feeds it.
