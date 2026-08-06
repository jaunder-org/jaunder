#!/usr/bin/env bash
# Mutation-testing discovery run.
#
# Runs cargo-mutants one package at a time, cheapest and highest-signal first,
# and parks each package's results under .mutants-loop/out/<pkg>/.
# The agent loop reads out/<pkg>/missed.txt to fill the work queue.
#
# This script only DISCOVERS surviving mutants. It never edits code.
# It is safe to kill and restart: finished packages are skipped.

set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/.mutants-loop/out"
LOG="$ROOT/.mutants-loop/discover.log"

mkdir -p "$OUT"

# Order: pure-logic crates first (best signal), UI crates last (most noise).
PACKAGES="common storage macros host jaunder client web"

echo "=== discovery started $(date -Is) ===" >>"$LOG"

for pkg in $PACKAGES; do
  if [ -f "$OUT/$pkg/.done" ]; then
    echo "[$pkg] already done, skipping" >>"$LOG"
    continue
  fi
  echo "[$pkg] starting $(date -Is)" >>"$LOG"
  rm -rf "$OUT/$pkg"
  mkdir -p "$OUT/$pkg"
  cargo mutants \
    --package "$pkg" \
    --jobs 4 \
    --no-shuffle \
    --output "$OUT/$pkg" \
    >>"$LOG" 2>&1
  status=$?
  echo "[$pkg] finished status=$status $(date -Is)" >>"$LOG"
  # cargo-mutants exits non-zero when mutants survive. That is the expected
  # outcome here, not a failure, so record completion either way.
  touch "$OUT/$pkg/.done"
done

echo "=== discovery complete $(date -Is) ===" >>"$LOG"
