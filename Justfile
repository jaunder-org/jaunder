# Public hermetic workflows refuse source/report drift before producer work.
coverage: _clean-tree coverage-workflow

doctests: _clean-tree doctests-workflow

[private]
_clean-tree:
    #!/usr/bin/env bash
    set -euo pipefail
    status=$(git status --porcelain --untracked-files=all)
    if [[ -n "$status" ]]; then
      printf 'coverage/doctests require a clean checkout\n%s\n' "$status" >&2
      exit 1
    fi

# Internal transition entrypoints also serve existing dirty-capable Rust callers.
[private]
coverage-workflow:
    mkdir -p .xtask/gcroots
    nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.coverage --out-link .xtask/gcroots/coverage
    nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.coverage-gate --no-link
    cargo xtask coverage-policy --out .xtask/gcroots/coverage

[private]
doctests-workflow:
    mkdir -p .xtask/gcroots
    nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.doctests --out-link .xtask/gcroots/doctests
    nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.doctests-gate --no-link

# Build the checkout's CSR bundle with its locked wasm toolchain and Vite.
[default]
build-csr $profile="debug":
    #!/usr/bin/env bash
    set -euo pipefail
    checkout="$PWD"
    case "$profile" in
      debug) cargo_profile=debug; set -- ;;
      release) cargo_profile=release; set -- --release ;;
      *) printf 'invalid CSR build profile: %s\n' "$profile" >&2; exit 2 ;;
    esac
    vite_tools="$(nix build .#csr-vite-node-modules --no-link --print-out-paths)"
    cargo build --locked -p csr --target wasm32-unknown-unknown "$@"
    rm -rf -- target/site
    work="$(mktemp -d)"
    trap 'rm -rf -- "$work"' EXIT
    project="$work/project"
    input="$work/input"
    dist="$work/dist"
    system_artifacts="$work/system-artifacts"
    mkdir -p "$project" "$input" "$system_artifacts"
    cp "$checkout/package.json" "$checkout/package-lock.json" "$checkout/vite.config.js" "$project/"
    mkdir -p "$project/csr"
    cp "$checkout/csr/index.html" "$checkout/csr/entry.js" "$project/csr/"
    ln -s "$vite_tools/node_modules" "$project/node_modules"
    cargo run --locked --manifest-path "$checkout/tools/Cargo.toml" -p devtool -- system-artifacts --out "$system_artifacts"
    CSR_VITE_PUBLIC_DIR="$checkout/public" bash "$checkout/scripts/build-csr-assets.sh" \
      "target/wasm32-unknown-unknown/$cargo_profile/csr.wasm" "$project" "$input" "$system_artifacts" "$dist"
    mkdir -p "$checkout/target/site"
    cp -a "$dist/." "$checkout/target/site/"
