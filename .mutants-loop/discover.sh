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

# cargo-mutants copies the whole source tree per job and builds it there. On the
# default TMPDIR that is /tmp, a 16 GB tmpfs — four jobs at ~2.4 GB of build
# artifacts each, running beside a Nix build, exhausted it and made the repo's
# own gate fail a test that passes fine on its own. A red gate the loop cannot
# explain is worse than a slow run: the rules tell it to revert and skip, so it
# would throw away good work. Put the scratch trees on the big disk instead.
export TMPDIR="${TMPDIR_MUTANTS:-$HOME/.cache/cargo-mutants-tmp}"
mkdir -p "$TMPDIR"

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
  # --test-tool nextest is required, not a preference. Under plain `cargo test`
  # the host crate's metrics tests share one process and a global recorder, so
  # the unmutated baseline fails and the whole package is skipped. nextest gives
  # each test its own process, which is what the repo's own gate uses.
  #
  # -E 'not test(postgres)' drops the case_2_postgres variants, which need a
  # live PostgreSQL that is not running here. Their case_1_sqlite twins cover
  # the same code, so nothing is lost. Without this, storage and jaunder fail
  # their baseline and contribute nothing.
  cargo mutants \
    --package "$pkg" \
    --jobs 2 \
    --no-shuffle \
    --test-tool nextest \
    --output "$OUT/$pkg" \
    -- -E 'not test(postgres)' \
    >>"$LOG" 2>&1
  status=$?
  echo "[$pkg] finished status=$status $(date -Is)" >>"$LOG"
  # cargo-mutants exits non-zero when mutants survive. That is the expected
  # outcome here, not a failure, so record completion either way.
  touch "$OUT/$pkg/.done"
done

echo "=== discovery complete $(date -Is) ===" >>"$LOG"
