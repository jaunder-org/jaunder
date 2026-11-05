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

# Build the checkout's CSR bundle with its locked wasm toolchain and postprocessor.
[default]
build-csr $profile="debug":
    #!/usr/bin/env bash
    set -euo pipefail
    case "$profile" in
      debug) set -- ;;
      release) set -- --release ;;
      *) printf 'invalid CSR build profile: %s\n' "$profile" >&2; exit 2 ;;
    esac
    cargo build --locked -p csr --target wasm32-unknown-unknown "$@"
    rm -rf -- target/site
    cargo run --locked --manifest-path tools/devtool/Cargo.toml -- csr-bundle --wasm "target/wasm32-unknown-unknown/$profile/csr.wasm" --out target/site
