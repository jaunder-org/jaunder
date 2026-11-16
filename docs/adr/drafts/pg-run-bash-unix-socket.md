# ADR-DRAFT: Throwaway test PostgreSQL is a bash script over a Unix socket

- Status: proposed
- Date: 2026-11-15
- Issue: [#1976](https://github.com/jaunder-org/jaunder/issues/1976)

## Context

`pg-run` runs one command against a throwaway PostgreSQL cluster. Its process
tree is fixed: `initdb`, `pg_ctl start`, a `psql` bootstrap, the command, then
`pg_ctl stop` and removal of the data directory. It began as a bash script and
was ported to Rust when the scripts were moved uniformly, without re-checking
each one ([ADR-0028](../0028-devtool-vs-xtask-boundary.md)).
[ADR-0246](../0246-retire-xtask-devtool-endpoints.md) then placed it in
`tools/coverage` as a nondefault `ephemeral-postgres` library module, shared by
the in-sandbox coverage producer and a host-proof `pg-run` binary.
[ADR-0247](../0247-host-tool-json-report-contract.md) listed that binary among
host-proof's executables.

The Rust module was about 390 non-test lines. Much of that existed only because
it was Rust: role-tagged URL types, an `Arc`/`Weak` guard and a signal thread to
emulate a shell `trap`, and injectable closures for testing cleanup warnings.
Each host consumer had to compile it before first use. #1976 re-checked it as a
pilot for a wider language review.

It also listened on loopback TCP with a random port and an eight-attempt retry
on bind races. #1811 had turned off its Unix socket defensively, because a
socket path is limited to 107 bytes and the checkout-local `TMPDIR` can exceed
that. No failure had actually been observed.

## Decision

`pg-run` is one bash script, `scripts/pg-run.sh`. Nix packages it with
`writeShellApplication` (shellcheck at build time, PostgreSQL 18 as its only
runtime input). It is on PATH in the shared devShells and in the coverage
derivation. It is the only throwaway-PostgreSQL implementation for host
workflows (`just pg-run`, `test-local`, mutants CI) and for the coverage
producer. The coverage derivation runs `pg-run devtool coverage emit`, and
devtool contains no PostgreSQL code.

The cluster listens only on a Unix socket, with no TCP. The socket lives in its
own short directory under `XDG_RUNTIME_DIR` (or `TMPDIR` when that is unset).
PGDATA stays under the checkout-local `TMPDIR`. The exported URLs are
`postgres://<role>@localhost/<db>?host=<socket dir>`. Because each cluster owns
its socket directory, the port is a constant, and port selection and retry no
longer exist.

Teardown runs on normal exit, failure, SIGINT and SIGTERM. On a signal, the
command receives SIGTERM, the cluster is removed, and `pg-run` dies by the
signal it received. Cleanup failures warn without changing the exit status.

Shell was chosen over Python and Rust because the job is a fixed command
sequence with trap-based cleanup. Shell is a native fit for that, and the
coverage sandbox already has bash but not python3. Rejected alternatives:

- `postgresqlTestHook` runs only in a Nix check phase.
- `ephemeralpg`'s `pg_tmp` tears down on a timer from a detached process, not
  when the command exits.

## Consequences

- This supersedes ADR-0246's placement of PostgreSQL custody in the
  `tools/coverage` library and the host-proof `pg-run` binary, and removes
  `pg-run` from ADR-0247's host-proof executables. It also closes ADR-0028's
  older placement of `with-ephemeral-postgres` under devtool, which ADR-0246 had
  already moved. The "exactly one PG implementation" rule still holds: the one
  implementation is this script.
- `CONTRIBUTING.md`'s language rule names shell for small process-orchestration
  wrappers of this shape. This is not a general licence for shell
  infrastructure.
- Editing the script needs a devShell reload, and there is no first-use compile.
- The script has no dedicated test suite. `test-local`, the coverage producer,
  mutants CI and shellcheck exercise it. Its signal and failure behavior was
  measured once on #1976.
- If the socket path still exceeds 107 bytes, startup fails loudly with
  PostgreSQL's own error. A `'` in the path breaks the server configuration, and
  `+`, `&` or `%` break the URL for clients. There is no TCP fallback.
- If PostgreSQL fails to start in the coverage derivation, the build fails
  before the producer runs, rather than recording a red `status.json`.
- Tests that rebuild a connection URL must keep the bootstrap URL's query.
  `PostgresTestConfig::bootstrap_url_as` does this.
