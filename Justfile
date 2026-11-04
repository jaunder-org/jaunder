set export

# Build the checkout's CSR bundle with its locked wasm toolchain and postprocessor.
build-csr profile="debug":
    #!/usr/bin/env bash
    set -euo pipefail
    unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_COMMON_DIR GIT_NAMESPACE
    case "$profile" in
      debug|release) ;;
      *) printf 'invalid CSR build profile: %s\n' "$profile" >&2; exit 2 ;;
    esac
    root="$(git rev-parse --show-toplevel)"
    cd "$root"
    cargo_args=(build --locked -p csr --target wasm32-unknown-unknown)
    if [[ "$profile" == release ]]; then cargo_args+=(--release); fi
    cargo "${cargo_args[@]}"
    bundle="$root/target/site"
    # Match try_exists/remove_dir_all: skip absent or dangling paths, unlink
    # final symlinks, and let inspection/removal errors stop bundling.
    python3 - "$bundle" <<'PY'
    import os
    import shutil
    import stat
    import sys

    path = sys.argv[1]
    try:
        os.stat(path)
    except FileNotFoundError:
        pass
    else:
        if stat.S_ISLNK(os.lstat(path).st_mode):
            os.unlink(path)
        else:
            shutil.rmtree(path)
    PY
    cargo run --locked --manifest-path tools/devtool/Cargo.toml -- csr-bundle --wasm "$root/target/wasm32-unknown-unknown/$profile/csr.wasm" --out "$bundle"
