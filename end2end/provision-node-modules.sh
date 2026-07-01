#!/usr/bin/env bash
# Provision end2end/node_modules for the offline `tsc --noEmit` type-check gate.
#
# Why this exists: end2end/node_modules is gitignored, so it is absent from every
# fresh checkout and every git worktree. The type-dep closure tsc needs
# (@types/node, undici-types, typescript, playwright/-core, and @playwright/test)
# all come from the Nix e2ePackage / playwright-test store paths, which the
# devShell exports as env vars. This script symlinks them into
# ./end2end/node_modules, relative to $PWD (the repo/worktree root).
#
# It is invoked from two places, both of which run with the repo root as cwd:
#   * the devShell shellHook — interactive IDE support in the main checkout; and
#   * xtask's `tsc-deps` static-check step — so `cargo xtask check|validate`
#     self-heals in any worktree, where the shellHook never re-ran for that cwd.
#
# Idempotent: rm -rf before each ln so re-runs overwrite cleanly and the plain
# @playwright dir (below) can replace an earlier symlink of the same name.
set -euo pipefail

: "${E2E_TYPES_NODE_MODULES:?unset — run inside the Nix devShell (nix develop)}"
: "${E2E_PLAYWRIGHT_TEST:?unset — run inside the Nix devShell (nix develop)}"

# Symlink the type-dep closure from the e2ePackage, then re-pin @playwright/test
# to the nix-matched Playwright (browser-driver parity + IDE support) instead of
# the e2ePackage's own npm copy.
mkdir -p end2end/node_modules
for dep in "$E2E_TYPES_NODE_MODULES"/*; do
  target="end2end/node_modules/$(basename "$dep")"
  rm -rf "$target"
  ln -sfn "$dep" "$target"
done
rm -rf end2end/node_modules/@playwright
mkdir -p end2end/node_modules/@playwright
ln -sfn "$E2E_PLAYWRIGHT_TEST" end2end/node_modules/@playwright/test
