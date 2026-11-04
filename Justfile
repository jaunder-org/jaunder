set shell := ["bash", "-cu"]
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
    # Preserve try_exists semantics: inspection errors, including ELOOP, must not
    # be mistaken for an absent output or followed by cleanup/bundling.
    python3 - "$bundle" <<'PY'
    import os
    import shutil
    import stat
    import sys

    path = sys.argv[1]
    try:
        info = os.lstat(path)
    except FileNotFoundError:
        sys.exit(0)
    except OSError as error:
        print(f"inspecting previous CSR bundle root {path}: {error}", file=sys.stderr)
        sys.exit(1)

    if stat.S_ISLNK(info.st_mode):
        try:
            os.stat(path)
        except FileNotFoundError:
            sys.exit(0)
        except OSError as error:
            print(f"inspecting previous CSR bundle root {path}: {error}", file=sys.stderr)
            sys.exit(1)
        try:
            os.unlink(path)
        except OSError as error:
            print(f"removing previous CSR bundle root {path}: {error}", file=sys.stderr)
            sys.exit(1)
    elif stat.S_ISDIR(info.st_mode):
        try:
            shutil.rmtree(path)
        except OSError as error:
            print(f"removing previous CSR bundle root {path}: {error}", file=sys.stderr)
            sys.exit(1)
    else:
        print(f"CSR bundle path is not a directory: {path}", file=sys.stderr)
        sys.exit(1)
    PY
    cargo run --locked --manifest-path tools/devtool/Cargo.toml -- csr-bundle --wasm "$root/target/wasm32-unknown-unknown/$profile/csr.wasm" --out "$bundle"
