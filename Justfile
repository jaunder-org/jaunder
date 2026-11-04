# Build the checkout's CSR bundle with its locked wasm toolchain and postprocessor.
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
