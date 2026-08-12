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
rm -rf "$out"
mkdir -p "$out"

echo "verifying $pkg :: $file"
echo "  output: $out"

run_mutants --package "$pkg" --file "$file" --output "$out"
status=$?

echo
echo "--- $file ---"
for f in missed caught unviable timeout; do
  path="$out/mutants.out/$f.txt"
  if [ -s "$path" ]; then
    echo "$f: $(wc -l <"$path")"
  else
    echo "$f: 0"
  fi
done

if [ -s "$out/mutants.out/missed.txt" ]; then
  echo
  echo "STILL ALIVE:"
  cat "$out/mutants.out/missed.txt"
fi

# cargo-mutants exits non-zero when mutants survive; that is a result, not a
# tool failure. Exit 4 (baseline failed) is a real failure and must not be
# mistaken for "nothing survived".
if [ "$status" -eq 4 ]; then
  echo
  echo "ERROR: unmutated baseline FAILED — nothing was tested." >&2
  echo "Do not read the zeros above as success. See PROTOCOL.md." >&2
  exit 4
fi

exit 0
