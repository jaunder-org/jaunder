#!/usr/bin/env bash
# Run one shard of one package's mutants, for CI.
#
#   .mutants-loop/ci-shard.sh <package> <k> <n> <output-dir>
#   .mutants-loop/ci-shard.sh common 0 4 .mutants-ci/common-0
#
# Differences from discover.sh, all of them because this runs unattended on a
# hosted runner rather than on a workstation:
#
#   --iterate     Skips mutants that a PREVIOUS run recorded as caught or
#                 unviable, reading them from <output-dir>/mutants.out (which
#                 cargo-mutants rotates to mutants.out.old). That directory is
#                 the CI cache. It is what makes a scheduled run cheap in the
#                 steady state: after a burn-down, almost every mutant is
#                 already caught, so a run tests only genuinely new ones.
#
#                 **This is unsound against regressions.** If the test that
#                 caught a mutant is later deleted or weakened, --iterate skips
#                 that mutant forever and the run stays green. The cache
#                 remembers "was caught", not "is still caught". A periodic full
#                 run WITHOUT --iterate is the only thing that catches it — see
#                 the `full` input on the workflow, and schedule it.
#
#   exit 0 always The caller decides pass/fail by reading missed.txt. cargo-mutants
#                 exits 2 both for "found surviving mutants" (the result we want
#                 to report) and for an invalid argument, so its exit code cannot
#                 carry the answer — that ambiguity already cost an eighth of
#                 every package once. Judge on artifacts, never on $?.

set -u

if [ "$#" -ne 4 ]; then
  echo "usage: ci-shard.sh <package> <k> <n> <output-dir>" >&2
  exit 64
fi

pkg="$1"
k="$2"
n="$3"
out="$4"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
. "$ROOT/.mutants-loop/common.sh"

mkdir -p "$out"

iterate=(--iterate)
if [ "${MUTANTS_FULL:-0}" = "1" ]; then
  # A full audit: re-test everything, including mutants a previous run caught.
  # Slower by design — this is the run that can notice a test having rotted away.
  iterate=()
  rm -rf "$out/mutants.out" "$out/mutants.out.old"
  echo "MUTANTS_FULL=1 — ignoring cached outcomes, re-testing every mutant"
fi

echo "package=$pkg shard=$k/$n jobs=${MUTANTS_JOBS:-2} full=${MUTANTS_FULL:-0}"
echo "filter=$MUTANTS_FILTER"

run_mutants \
  --package "$pkg" \
  --shard "$k/$n" \
  "${iterate[@]}" \
  --output "$out"
status=$?
echo "cargo-mutants exit=$status (informational only)"

mo="$out/mutants.out"

# A run that tested nothing must not look like a clean one. outcomes.json is
# written only by a run that actually got as far as testing mutants; its absence
# means a failed baseline, a bad argument, or a crash.
if [ ! -f "$mo/outcomes.json" ]; then
  echo "::error::no outcomes.json — nothing was tested (exit $status). Treating as failure."
  exit 1
fi

count() { [ -s "$mo/$1.txt" ] && wc -l <"$mo/$1.txt" || echo 0; }

caught=$(count caught)
missed=$(count missed)
unviable=$(count unviable)
timeout=$(count timeout)

echo "caught=$caught missed=$missed unviable=$unviable timeout=$timeout"

# A timeout is not a result, it is a hole: neither caught nor missed, so the
# mutant went unexamined while the totals still look tidy. Surface it.
if [ "$timeout" -gt 0 ]; then
  echo "::warning::$pkg shard $k/$n: $timeout mutant(s) timed out and were never examined"
fi

exit 0
