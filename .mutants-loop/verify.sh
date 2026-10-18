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

# One native consumer owns all evidence policy and reporting. Propagate its
# outcome, including launcher failures, rather than interpreting partial output.
cargo xtask mutants verify-evidence --package "$pkg" --file "$file" \
  --output "$out" --producer-status "$status"
exit $?
