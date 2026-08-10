#!/usr/bin/env bash
# Start discovery DETACHED, so it survives the agent session that launched it.
#
#   .mutants-loop/start-discovery.sh          # start (no-op if already running)
#   .mutants-loop/start-discovery.sh --status # is it alive, and how far along
#   .mutants-loop/start-discovery.sh --stop   # stop it
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
    echo "stopping discovery (pid $pid)"
    kill "$pid"
    exit 0
  fi
  echo "discovery not running"
  exit 0
  ;;
--start) ;;
*)
  echo "usage: start-discovery.sh [--start|--status|--stop]" >&2
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
