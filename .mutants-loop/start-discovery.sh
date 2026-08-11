#!/usr/bin/env bash
# Start discovery DETACHED, so it survives the agent session that launched it.
#
#   .mutants-loop/start-discovery.sh           # start (no-op if already running)
#   .mutants-loop/start-discovery.sh --in 1800 # start after a delay, detached
#   .mutants-loop/start-discovery.sh --status  # is it alive, and how far along
#   .mutants-loop/start-discovery.sh --stop    # stop it (and its children)
#
# Why detached: a full workspace-scoped pass is many hours. Run as a child of
# the agent's shell it is a tracked background task, and the harness kills those
# — one pass died mid-package that way. setsid puts it in its own session, so it
# keeps going across agent turns, compaction, and session exit.
#
# It is still killable: the PID is in .mutants-loop/discovery.pid, and --stop
# uses it. Never pattern-kill cargo-mutants; other work may be running.

set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOOP="$ROOT/.mutants-loop"
PIDFILE="$LOOP/discovery.pid"
LOG="$LOOP/discover.log"

running_pid() {
  [ -f "$PIDFILE" ] || return 1
  pid="$(cat "$PIDFILE")"
  [ -n "$pid" ] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  echo "$pid"
}

case "${1:---start}" in
--status)
  if pid="$(running_pid)"; then
    echo "discovery RUNNING (pid $pid)"
  else
    echo "discovery not running"
  fi
  echo
  echo "per-package progress:"
  for d in "$LOOP"/out/*/; do
    [ -d "$d" ] || continue
    pkg="$(basename "$d")"
    # Counts are summed live across shards, so this works mid-run — the merged
    # top-level files only appear once every shard of a package has finished.
    count() {
      cat "$d"/shard-*/mutants.out/"$1".txt 2>/dev/null | sort -u | grep -c . || true
    }
    shards_done=$(find "$d" -maxdepth 2 -name .done -path '*/shard-*' 2>/dev/null | wc -l)
    if [ -f "$d/.done" ]; then state="done"; else state="$shards_done shard(s) done"; fi
    echo "  $pkg: $state — caught $(count caught), missed $(count missed), unviable $(count unviable), timeout $(count timeout)"
  done
  exit 0
  ;;
--stop)
  if pid="$(running_pid)"; then
    # Signal the whole process GROUP, not just the launcher.
    #
    # setsid makes discover.sh a session and group leader, so its pgid equals
    # its pid. `kill $pid` reaps only the shell loop and leaves the cargo-mutants
    # child orphaned and still building — that happened once, and the run kept
    # chewing CPU after --stop reported success. `kill -- -$pgid` takes the
    # group down with it.
    pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')"
    echo "stopping discovery (pid $pid, pgid ${pgid:-unknown})"
    if [ -n "$pgid" ]; then
      kill -TERM -- "-$pgid" 2>/dev/null || kill "$pid" 2>/dev/null || true
    else
      kill "$pid" 2>/dev/null || true
    fi

    # Confirm. A --stop that reports success while the run continues is worse
    # than one that fails loudly.
    n=0
    while [ "$n" -lt 20 ]; do
      kill -0 "$pid" 2>/dev/null || break
      sleep 1
      n=$((n + 1))
    done
    if kill -0 "$pid" 2>/dev/null; then
      echo "WARNING: pid $pid still alive after 20s; sending KILL" >&2
      [ -n "$pgid" ] && kill -KILL -- "-$pgid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
    fi
    rm -f "$PIDFILE"
    echo "stopped"
    exit 0
  fi
  echo "discovery not running"
  exit 0
  ;;
--in)
  # Delayed start, detached. The sleeper is in its own session too, so it
  # survives the session that scheduled it — the whole point of asking for a
  # delay is that nobody has to be around when it fires.
  delay="${2:-}"
  case "$delay" in
  '' | *[!0-9]*)
    echo "usage: start-discovery.sh --in <seconds>" >&2
    exit 64
    ;;
  esac
  if pid="$(running_pid)"; then
    echo "discovery already running (pid $pid) — not scheduling" >&2
    exit 0
  fi
  setsid nohup sh -c "sleep $delay; exec '$0' --start" >>"$LOOP/scheduled.log" 2>&1 &
  echo "discovery scheduled in ${delay}s (starter pid $!)"
  echo "  it will write its own pid to $PIDFILE when it fires"
  echo "  cancel before it fires: kill $!"
  exit 0
  ;;
--start) ;;
*)
  echo "usage: start-discovery.sh [--start|--in <seconds>|--status|--stop]" >&2
  exit 64
  ;;
esac

if pid="$(running_pid)"; then
  echo "discovery already running (pid $pid) — not starting a second one"
  exit 0
fi

setsid nohup "$LOOP/discover.sh" >/dev/null 2>&1 &
echo $! >"$PIDFILE"
echo "discovery started detached (pid $(cat "$PIDFILE"))"
echo "  log:    $LOG"
echo "  status: .mutants-loop/start-discovery.sh --status"
