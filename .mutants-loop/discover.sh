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

# TMPDIR, the nextest filter, and the mandatory flags all live in common.sh, so
# discovery and verify.sh run the tool identically. They must: if they differ,
# they answer different questions about the same mutant.
# shellcheck source=/dev/null
. "$ROOT/.mutants-loop/common.sh"

mkdir -p "$OUT"

# Order: pure-logic crates first (best signal), UI crates last (most noise).
#
# `client` is left out on purpose. Its 42 mutants all survived with nothing
# caught: the crate is WASM-only, so no host test reaches it. Every mutant there
# is noise, for the same reason .cargo/mutants.toml excludes storage/src/postgres.
PACKAGES="common storage macros host jaunder web"

echo "=== discovery started $(date -Is) ===" >>"$LOG"

for pkg in $PACKAGES; do
  if [ -f "$OUT/$pkg/.done" ]; then
    echo "[$pkg] already done, skipping" >>"$LOG"
    continue
  fi
  echo "[$pkg] starting $(date -Is)" >>"$LOG"
  rm -rf "$OUT/$pkg"
  mkdir -p "$OUT/$pkg"
  # run_mutants carries the mandatory flags and the filter — see common.sh for
  # what each one is for and what went wrong without it.
  echo "[$pkg] filter: $MUTANTS_FILTER" >>"$LOG"
  run_mutants --package "$pkg" --output "$OUT/$pkg" >>"$LOG" 2>&1
  status=$?
  echo "[$pkg] finished status=$status $(date -Is)" >>"$LOG"
  # cargo-mutants exits non-zero when mutants survive — a result, not a failure.
  # Exit 4 is different: the unmutated baseline failed, so NOTHING was tested
  # and the empty result means nothing. Do not mark such a package done.
  if [ "$status" -eq 4 ]; then
    echo "[$pkg] BASELINE FAILED — no mutants tested, not marking done" >>"$LOG"
    continue
  fi
  touch "$OUT/$pkg/.done"
done

echo "=== discovery complete $(date -Is) ===" >>"$LOG"
