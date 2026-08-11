#!/usr/bin/env bash
# Did discovery actually examine every mutant it should have?
#
#   .mutants-loop/reconcile.sh            # all scanned packages
#   .mutants-loop/reconcile.sh common     # one package
#
# Compares the mutants cargo-mutants GENERATES for a package against the ones
# that appear in the results (caught + missed + unviable + timeout). Anything in
# the first set and not the second was never examined.
#
# This exists because that gap has happened repeatedly and always looked like a
# clean result:
#   - a failed baseline reports zeroes and exits 4
#   - --shard 8/8 is invalid, so an eighth of every package silently vanished,
#     and the invalid-argument exit code (2) is the same one cargo-mutants uses
#     for the perfectly normal "found surviving mutants"
#   - a too-tight timeout files mutants as neither caught nor missed
#
# In every case the summary looked plausible. Counting the mutants is the only
# thing that actually answers the question, so do not trust a discovery run
# until this passes.

set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOOP="$ROOT/.mutants-loop"
OUT="$LOOP/out"
# shellcheck source=/dev/null
. "$LOOP/common.sh"

PACKAGES="${*:-common storage macros host jaunder}"

work="$TMPDIR/reconcile.$$"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT

overall=0

for pkg in $PACKAGES; do
  if [ ! -d "$OUT/$pkg" ]; then
    echo "$pkg: NO RESULTS"
    overall=1
    continue
  fi

  # The mutants that should exist. --list only parses; it runs nothing.
  cargo mutants --package "$pkg" --list 2>/dev/null | sort -u >"$work/expected"

  # The mutants that have an outcome, from every shard.
  cat "$OUT/$pkg"/shard-*/mutants.out/caught.txt \
    "$OUT/$pkg"/shard-*/mutants.out/missed.txt \
    "$OUT/$pkg"/shard-*/mutants.out/unviable.txt \
    "$OUT/$pkg"/shard-*/mutants.out/timeout.txt \
    2>/dev/null | sort -u >"$work/actual"

  exp=$(grep -c . <"$work/expected" || true)
  act=$(grep -c . <"$work/actual" || true)
  comm -23 "$work/expected" "$work/actual" >"$work/gap"
  gap=$(grep -c . <"$work/gap" || true)

  if [ "$gap" -eq 0 ]; then
    echo "$pkg: OK — $act/$exp mutants examined"
  else
    echo "$pkg: INCOMPLETE — $act/$exp examined, $gap NEVER EXAMINED"
    echo "  first few:"
    head_n=0
    while IFS= read -r line && [ "$head_n" -lt 5 ]; do
      echo "    $line"
      head_n=$((head_n + 1))
    done <"$work/gap"
    overall=1
  fi
done

if [ "$overall" -eq 0 ]; then
  echo
  echo "All packages fully examined."
else
  echo
  echo "Discovery is INCOMPLETE. Do not build a work queue from these results." >&2
fi

exit "$overall"
