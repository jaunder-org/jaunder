# ADR-0144: Process Configuration and CLI Contract

- Status: accepted
- Date: 2026-08-14
- Issue: [#938](https://github.com/jaunder-org/jaunder/issues/938)

## Context

Jaunder is a single binary ([ADR-0008](0008-deployment-model.md)), so the CLI is
also the operator surface for initialization, serving, user administration,
PostgreSQL provisioning, backup, and maintenance. The code already exposes a
stable-looking process configuration layer through clap flags and `JAUNDER_*`
environment variables, but no ADR recorded which parts were contract and which
were local parsing details.

This surface is not the stored configuration registry.
[ADR-0102](0102-config-key-closed-registry.md) governs validated `site_config`
database keys such as runtime site settings. The process surface is applied
before or while opening the process: bind address, database URL, data path,
logging verbosity, runtime-info path, and environment mode. Conflating the two
would make startup behavior depend on a database that may not exist yet and
would blur secret handling.

PostgreSQL needs one special case. Operators should not have to place a password
in the database URL just because `JAUNDER_DB` carries the rest of the connection
string. The storage layer already reads `JAUNDER_DB_PASSWORD_FILE` and
`JAUNDER_DB_PASSWORD`, applies those as overrides, and returns typed
configuration errors when a configured source cannot be read.

## Decision

The `jaunder` CLI is the public operator contract. For every applicable
argument, precedence is:

1. an explicit CLI flag;
2. the matching `JAUNDER_*` environment variable;
3. the documented default.

Applicability is part of the contract, not every variable for every subcommand:

- `JAUNDER_VERBOSE` / `--verbose` is global and defaults to false.
- `JAUNDER_STORAGE_PATH` / `--storage-path` applies to storage-using commands
  and defaults to `./data`.
- `JAUNDER_DB` / `--db` applies to storage-using commands and defaults to
  `sqlite:./data/jaunder.db`.
- `JAUNDER_BIND` / `--bind` applies to `serve` and defaults to `127.0.0.1:3000`.
- `JAUNDER_ENV` / `--environment` applies to `serve` and defaults to `dev`.
- `JAUNDER_RUNTIME_FILE` / `--runtime-file` applies to `serve`; when absent,
  runtime info is written to `<storage-path>/runtime.json`.

For PostgreSQL, `JAUNDER_DB_PASSWORD_FILE` and `JAUNDER_DB_PASSWORD` are the
process secret channels. `JAUNDER_DB_PASSWORD_FILE` takes precedence over
`JAUNDER_DB_PASSWORD`; file content has trailing whitespace trimmed. Either
channel overrides a password embedded in `--db` or `JAUNDER_DB`. A configured
password source that is unreadable or not Unicode is a configuration error.
Operators should not put the secret in the database URL when a service manager
can inject the dedicated password variable or file.

`JAUNDER_ENV=prod` is load-bearing: it enables secure cookies and declines
initialization of a missing database. Existing databases still open and migrate
normally. Development mode may initialize only a missing SQLite database; it
does not provision PostgreSQL.

This process configuration surface remains distinct from ADR-0102's stored
`site_config` registry. Neither surface is an alias or fallback for the other.

## Consequences

Changing a flag name, environment variable, default, precedence rule, password
source order, or command applicability requires compatibility review. These are
operator contracts even when implemented with clap attributes.

Local defaults remain optimized for development. Production operators should set
process configuration explicitly, including `JAUNDER_ENV=prod`, rather than rely
on defaults that bind to loopback and use a relative SQLite URL.

Secrets stay outside persisted URLs. The tradeoff is a two-part PostgreSQL
configuration story — URL plus secret channel — but it avoids baking credentials
into command lines, systemd unit text, or copied documentation.

Rejected alternatives:

- Treating clap `env` annotations as private implementation detail. That would
  let ordinary refactors break deployments without an architectural change.
- Storing process configuration in `site_config`. Startup needs these values
  before stored configuration can be read, and process secrets do not belong in
  the database key registry.
- Silently ignoring malformed secret inputs. A configured secret channel that
  cannot be read is an operator error, not a cue to try an unauthenticated
  connection.
