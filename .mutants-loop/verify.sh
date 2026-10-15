#!/usr/bin/env bash
# Verify that the surviving mutants in one file are now dead.
#
#   .mutants-loop/verify.sh <package> <file>
#   .mutants-loop/verify.sh common common/src/render.rs
#
# Runs cargo-mutants exactly as the scheduled CI scan runs it (both source
# common.sh) against one file, into a scratch output dir, and prints the counts.
# Anything left in `missed` is still alive.
#
# This is the authority for one file, and CI is the authority for what is left
# overall — .mutants-loop/next-work.sh ranks that. A file CI listed may already be
# fixed; running this is how you find out, and why no work list is kept on disk.
#
# Use this rather than hand-typing a cargo mutants command. The flags are not
# optional and getting one wrong produces a plausible wrong answer instead of an
# error.

set -u

if [ "$#" -ne 2 ]; then
  echo "usage: verify.sh <package> <file>" >&2
  echo "   eg: verify.sh common common/src/render.rs" >&2
  exit 64
fi

pkg="$1"
file="$2"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
. "$ROOT/.mutants-loop/common.sh"

slug="$(echo "$file" | tr '/.' '--')"
out="$TMPDIR/verify-$slug"
if ! { rm -rf "$out" && mkdir -p "$out"; }; then
  echo "ERROR: verification failed: cannot prepare fresh output: $out" >&2
  exit 1
fi

echo "verifying $pkg :: $file"
echo "  output: $out"

run_mutants --package "$pkg" --file "$file" --output "$out"
status=$?

# Survivors are findings, but tool errors cannot establish verification.
case "$status" in
  0|2) ;;
  *)
    echo "ERROR: verification failed: cargo-mutants exited $status; see $out" >&2
    exit "$status"
    ;;
esac

# Validate before publishing counts or survivor conclusions. Missing category
# files are I/O errors too, not evidence of a zero population.
if ! counts=$(jq -ers --arg package "$pkg" --arg file "$file" \
  --argjson status "$status" \
  --rawfile missed "$out/mutants.out/missed.txt" \
  --rawfile caught "$out/mutants.out/caught.txt" \
  --rawfile unviable "$out/mutants.out/unviable.txt" \
  --rawfile timeout "$out/mutants.out/timeout.txt" \
  -f "$ROOT/.mutants-loop/verify-outcomes.jq" "$out/mutants.out/outcomes.json"); then
  echo "ERROR: verification failed: invalid or incomplete evidence; see $out" >&2
  exit 1
fi

echo
echo "--- $file ---"
printf '%s\n' "$counts"

if [ -s "$out/mutants.out/missed.txt" ]; then
  echo
  echo "STILL ALIVE:"
  cat "$out/mutants.out/missed.txt" || exit 1
fi

exit 0
