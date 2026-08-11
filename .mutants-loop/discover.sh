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
LOOP_DIR="$ROOT/.mutants-loop"
OUT="$LOOP_DIR/out"
LOG="$LOOP_DIR/discover.log"

# TMPDIR, the nextest filter, and the mandatory flags all live in common.sh, so
# discovery and verify.sh run the tool identically. They must: if they differ,
# they answer different questions about the same mutant.
# shellcheck source=/dev/null
. "$ROOT/.mutants-loop/common.sh"

mkdir -p "$OUT"

# Cheapest and highest-signal first.
#
# Two packages are left out on purpose:
#
# `client` — WASM-only. All 42 of its mutants survived with nothing caught, so
#   no host test reaches any of it. Same reasoning as .cargo/mutants.toml's
#   exclusion of storage/src/postgres.
#
# `web` — dropped by the user's call after the first pass reported 361
#   survivors against only 157 caught. That ratio was never credible, and the
#   render.rs finding gives the likely reason: much of web is feature-gated or
#   component code the host unit run does not reach, so the "survivors" are
#   mostly the same measurement artifact. It is also the most expensive package
#   to scan. If web is ever wanted, run it on its own — do not fold it back in
#   here and make every future run pay for it.
PACKAGES="common storage macros host jaunder"

echo "=== discovery started $(date -Is) ===" >>"$LOG"

# Each package is scanned in SHARDS pieces, each with its own output dir and its
# own .done marker.
#
# Resumability is the whole point. A workspace-scoped pass is many hours, and a
# run WILL be interrupted — the harness killed one mid-package, and a machine
# can reboot. Without shards, cargo-mutants has no resume: the package restarts
# from mutant 1 and hours of work are thrown away. That already happened once at
# 154 of common's 580.
#
# The cost is one extra baseline build per shard (~30s). Cheap against losing an
# hour.
SHARDS="${MUTANTS_SHARDS:-8}"

for pkg in $PACKAGES; do
  if [ -f "$OUT/$pkg/.done" ]; then
    echo "[$pkg] already done, skipping" >>"$LOG"
    continue
  fi

  mkdir -p "$OUT/$pkg"
  pkg_ok=1

  # cargo-mutants shards are ZERO-indexed: --shard k/n requires k < n. Looping
  # 1..8 skipped shard 0 entirely and errored on 8/8, silently losing an eighth
  # of every package.
  i=0
  while [ "$i" -lt "$SHARDS" ]; do
    shard_dir="$OUT/$pkg/shard-$i"
    if [ -f "$shard_dir/.done" ]; then
      echo "[$pkg $i/$SHARDS] already done, skipping" >>"$LOG"
      i=$((i + 1))
      continue
    fi
    echo "[$pkg $i/$SHARDS] starting $(date -Is)" >>"$LOG"
    rm -rf "$shard_dir"
    mkdir -p "$shard_dir"
    # run_mutants carries the mandatory flags and the filter — see common.sh for
    # what each one is for and what went wrong without it.
    run_mutants --package "$pkg" --shard "$i/$SHARDS" --output "$shard_dir" \
      >>"$LOG" 2>&1
    status=$?
    echo "[$pkg $i/$SHARDS] finished status=$status $(date -Is)" >>"$LOG"
    # Mark done only on EVIDENCE that mutants were actually tested, never on the
    # exit code alone.
    #
    # The exit code cannot carry this. cargo-mutants uses 2 for "found surviving
    # mutants" — a normal result — and clap also exits 2 for an invalid argument.
    # That collision is how `--shard 8/8` ("shard k must be less than n") was
    # recorded as a finished shard: the directory held nothing but the .done
    # marker, and an eighth of every package went missing while the summary
    # looked complete.
    #
    # outcomes.json is written only by a run that actually tested something.
    if [ ! -f "$shard_dir/mutants.out/outcomes.json" ]; then
      echo "[$pkg $i/$SHARDS] NO OUTCOMES (status=$status) — nothing tested" >>"$LOG"
      pkg_ok=0
    elif [ "$status" -eq 4 ]; then
      echo "[$pkg $i/$SHARDS] BASELINE FAILED — nothing tested" >>"$LOG"
      pkg_ok=0
    else
      touch "$shard_dir/.done"
    fi
    i=$((i + 1))
  done

  # Merge the shards into the flat layout the queue and verify steps read.
  for f in missed caught unviable timeout; do
    cat "$OUT/$pkg"/shard-*/mutants.out/"$f".txt 2>/dev/null |
      sort -u >"$OUT/$pkg/$f.txt" || true
  done

  if [ "$pkg_ok" -eq 1 ]; then
    touch "$OUT/$pkg/.done"
  else
    echo "[$pkg] INCOMPLETE — a shard's baseline failed" >>"$LOG"
  fi
done

echo "=== discovery finished $(date -Is) ===" >>"$LOG"

# "Finished" is not "complete". Count the mutants before believing the summary —
# a run can end tidily having never examined a large slice of the work, and that
# has happened more than once here. reconcile.sh is the only thing that actually
# answers the question.
{
  echo "--- reconciliation ---"
  "$LOOP_DIR/reconcile.sh" $PACKAGES 2>&1
  echo "reconcile status=$?"
} >>"$LOG" 2>&1

echo "=== discovery complete $(date -Is) ===" >>"$LOG"
