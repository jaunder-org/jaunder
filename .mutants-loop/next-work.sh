#!/usr/bin/env bash
# What is left to grind, ranked, from the last scheduled mutation run.
#
#   .mutants-loop/next-work.sh            # every file with survivors, worst first
#   .mutants-loop/next-work.sh --package storage
#   .mutants-loop/next-work.sh --run 31603190839
#
# Output is one line per file, tab-separated:
#
#   <package>\t<file>\t<surviving>
#
# This reads CI, not the working tree, and keeps no state. That is the point.
# The previous design kept a hand-maintained queue.md, which recorded what one
# discovery run found and then had no way to notice it had stopped being true:
# it still listed host/src/metrics.rs as 21 survivors months after commit
# ee9a34d5 killed all of them. Anything derived from a fixed point in history
# rots the same way. This re-derives from the newest scan every time it runs,
# and callers are expected to confirm each file with verify.sh before working
# it — a file fixed since the scan comes back missed=0 and drops out by itself.
set -euo pipefail

REPO="jaunder-org/jaunder"
WORKFLOW="mutants.yml"
package=""
run_id=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --package)
      package="${2:?--package needs a value}"
      shift 2
      ;;
    --run)
      run_id="${2:?--run needs a value}"
      shift 2
      ;;
    -h | --help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 64
      ;;
  esac
done

# Prefer the newest completed SCHEDULED run, and fall back to any completed run
# only if no cron has fired yet.
#
# Not simply "the newest run": a workflow_dispatch can scan a single package, and
# picking that one would report an empty work list for the other four — the same
# class of quiet wrongness as the queue this replaces, arrived at from the other
# direction. Only the schedules scan everything, so only they can answer "what is
# left" for the whole workspace.
#
# Whatever its conclusion, note. Filtering on success would find nothing: the
# report job exits non-zero whenever mutants survive, which is the normal state
# of a run that has anything to tell us. Green means there is no work, not that
# the results are unusable.
if [ -z "$run_id" ]; then
  run_id=$(gh run list --repo "$REPO" --workflow "$WORKFLOW" \
    --event schedule --status completed --limit 1 \
    --json databaseId --jq '.[0].databaseId // empty')
fi
if [ -z "$run_id" ]; then
  run_id=$(gh run list --repo "$REPO" --workflow "$WORKFLOW" \
    --status completed --limit 1 --json databaseId --jq '.[0].databaseId // empty')
  echo "note: no scheduled run yet — falling back to run $run_id, which may" >&2
  echo "      have scanned only one package. Check its scope before trusting" >&2
  echo "      an empty result for any other." >&2
fi
if [ -z "$run_id" ] || [ "$run_id" = "null" ]; then
  echo "no completed $WORKFLOW run found in $REPO" >&2
  exit 1
fi

# Say which scan this is. A work list is only as current as the run behind it,
# and the caller cannot judge that from the file names alone.
gh run view "$run_id" --repo "$REPO" --json event,createdAt,conclusion \
  --jq '"source: run '"$run_id"' (\(.event), \(.createdAt), \(.conclusion))"' >&2

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

# One directory per artifact: shards must not overwrite each other's missed.txt.
if ! gh run download "$run_id" --repo "$REPO" --pattern 'mutants-*' \
  --dir "$scratch" 2>/dev/null; then
  echo "run $run_id has no mutants-* artifacts (expired, or it never uploaded)" >&2
  exit 1
fi

# Depth-agnostic on purpose. upload-artifact roots an artifact at the
# non-wildcard prefix of its search pattern, not at the least common ancestor of
# the files, so the nesting is deeper than it looks and has changed once already.
mapfile -t missed_files < <(find "$scratch" -name missed.txt)
if [ "${#missed_files[@]}" -eq 0 ]; then
  echo "run $run_id reported no missed.txt at all — suspect the run, not the code" >&2
  exit 1
fi

# A mutant line is `path/to/file.rs:LINE:COL: description`. The package is the
# first path segment, except `server/`, whose crate is named `jaunder` — the one
# place the directory and the cargo package disagree, and the reason a naive
# `cut -d/ -f1` would hand cargo-mutants a package that does not exist.
cat "${missed_files[@]}" \
  | grep -v '^[[:space:]]*$' \
  | cut -d: -f1 \
  | sort \
  | uniq -c \
  | sort -rn \
  | while read -r count file; do
    dir="${file%%/*}"
    case "$dir" in
      server) pkg=jaunder ;;
      *) pkg="$dir" ;;
    esac
    if [ -n "$package" ] && [ "$pkg" != "$package" ]; then
      continue
    fi
    printf '%s\t%s\t%s\n' "$pkg" "$file" "$count"
  done
