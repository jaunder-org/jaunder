#!/usr/bin/env bash
# pg-run [--] <command> [args...]: run a command against a throwaway PostgreSQL
# cluster reachable only over a Unix socket, and tear the cluster down however
# the command ends (normal exit, failure, SIGINT or SIGTERM).
#
# The command receives JAUNDER_PG_TEST_URL (role/database `jaunder`) and
# JAUNDER_PG_BOOTSTRAP_TEST_URL (superuser `postgres`). Each cluster owns its
# socket directory, so there is no TCP port to choose or race for. Durability is
# off because the cluster is discarded after the run.
set -euo pipefail

if [[ ${1-} == -- ]]; then
  shift
fi
if (($# == 0)); then
  echo 'usage: pg-run [--] <command> [args...]' >&2
  exit 2
fi

pgdata=
sockdir=
started=
caught=

# Cleanup problems are reported but never replace the command's result, and
# never print the data directory path.
warn() {
  echo "pg-run: warning: ignored failure to $1 the ephemeral PostgreSQL cluster" >&2
}

# Idempotent. Every step is guarded: errexit also applies inside the EXIT trap,
# and an unguarded failure would skip the remaining cleanup and change the exit
# status.
teardown() {
  if [[ -n $started ]]; then
    started=
    if ! pg_ctl -D "$pgdata" -m immediate stop >/dev/null 2>&1; then
      warn stop
    fi
  fi
  if [[ -n $pgdata || -n $sockdir ]]; then
    # rm's own message would name the path.
    if ! rm -rf -- ${pgdata:+"$pgdata"} ${sockdir:+"$sockdir"} 2>/dev/null; then
      warn remove
    fi
    pgdata=
    sockdir=
  fi
}

# Every exit, including an errexit during setup, tears down; after a signal,
# pg-run then dies by that signal, as the caller expects.
finish() {
  teardown
  if [[ -n $caught ]]; then
    trap - "$caught" EXIT
    kill -"$caught" "$$"
  fi
}

# A signal is only recorded here; the main flow and `finish` act on it.
trap finish EXIT
trap 'caught=INT' INT
trap 'caught=TERM' TERM

pgdata=$(mktemp -d "${TMPDIR:-/tmp}/jaunder-pg.XXXXXX")
# A socket path is limited to 107 bytes, which a checkout-local TMPDIR inside a
# branch-named worktree can exceed, so the socket gets its own short directory.
sockdir=$(mktemp -d "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/jaunder-pg.XXXXXX")

initdb -D "$pgdata" -U postgres -A trust --no-sync >/dev/null
cat >>"$pgdata/postgresql.conf" <<EOF
listen_addresses = ''
unix_socket_directories = '$sockdir'
max_connections = 200
fsync = off
full_page_writes = off
synchronous_commit = off
EOF

# The server log stays out of the command's output, but is shown if the server
# does not come up, because teardown deletes it. A failed start may still have
# left a postmaster behind (a `-w` timeout), so it is stopped without a warning.
started=1
if ! pg_ctl -D "$pgdata" -l "$pgdata/server.log" -w start >/dev/null; then
  cat -- "$pgdata/server.log" >&2 || :
  pg_ctl -D "$pgdata" -m immediate stop >/dev/null 2>&1 || :
  started=
  exit 1
fi

psql -X -q -h "$sockdir" -U postgres -d postgres -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
CREATE ROLE jaunder LOGIN CREATEDB;
CREATE DATABASE jaunder OWNER jaunder;
SQL

export JAUNDER_PG_TEST_URL="postgres://jaunder@localhost/jaunder?host=$sockdir"
export JAUNDER_PG_BOOTSTRAP_TEST_URL="postgres://postgres@localhost/postgres?host=$sockdir"

status=0
if [[ -z $caught ]]; then
  # The command runs in the background because `wait`, unlike a foreground
  # command, returns as soon as a trapped signal arrives. Without the explicit
  # redirect, a non-interactive shell would give it /dev/null as stdin.
  "$@" <&0 &
  child=$!
  if [[ -z $caught ]]; then
    wait "$child" || status=$?
  fi
  if [[ -n $caught ]]; then
    # Background commands ignore SIGINT, so both signals are forwarded as TERM.
    # Each further signal interrupts `wait` again, so wait until it is gone.
    kill -TERM "$child" 2>/dev/null || :
    while kill -0 "$child" 2>/dev/null; do
      wait "$child" 2>/dev/null || :
    done
  fi
fi
finish
exit "$status"
