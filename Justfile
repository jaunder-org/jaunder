# Host Cargo compilation shares rustc artifacts across canonical worktree roots.
# Git discovery failure retains the current root, as in xtask's cache policy.
export RUSTC_WRAPPER := "sccache"
export CARGO_INCREMENTAL := "0"
export SCCACHE_BASEDIRS := ```
    unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_COMMON_DIR GIT_NAMESPACE
    {
      git worktree list --porcelain | while IFS= read -r line; do
        case "$line" in
          'worktree '*) root=${line#worktree }; [ ! -d "$root" ] || (cd "$root" && pwd -P) ;;
        esac
      done
      pwd -P
    } | LC_ALL=C sort -u | paste -sd ':'
    ```

devtool := "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p devtool --bin devtool --"
prettier := "just --justfile recipes/prettier.just --working-directory ."

# Setup follows the current checkout even after cd in an existing devShell.
tsc mode="verify":
    setup-e2e-node-modules "$E2E_TYPES_NODE_MODULES" "$E2E_PLAYWRIGHT_TEST"
    {{devtool}} check tsc {{if mode == "fix" { "--fix" } else { "" }}}

# Select this checkout's pinned host package, never an ambient executable.
# Controller operations require the trusted workflow context; tests use fake adapters.
adr-promoter:
    nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- controller

# Verify workflows refuse dirty or unavailable Git state before work.
coverage: _clean-tree coverage-workflow

doctests: _clean-tree doctests-workflow

[private]
_clean-tree:
    #!/usr/bin/env bash
    set -euo pipefail
    unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_COMMON_DIR GIT_NAMESPACE
    status=$(git status --porcelain --untracked-files=all)
    if [[ -n "$status" ]]; then
      printf 'verify workflows require a clean checkout\n%s\n' "$status" >&2
      exit 1
    fi

# Qualification's public catalog owns ordinary Git preflight; the host leaf
# binds this HEAD to its clean SourcePin before selecting any Nix harness.
production-baseline-discover revision: (_production-baseline revision "")

# Only discovery may use the shared helper's empty-target sentinel.
production-baseline-accept source target: (_production-baseline (source) (if target == "" { error("production-baseline accept requires a nonempty target") } else { target }))

production-baseline-qualify-styling: _clean-tree
    #!/usr/bin/env bash
    set -euo pipefail
    unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_COMMON_DIR GIT_NAMESPACE
    harness=$(git rev-parse --verify 'HEAD^{commit}')
    cargo xtask qualification-leaf --expected-harness "$harness"

# Mutable local lifecycle diagnostic; deliberately no qualification/evidence.
production-baseline-smoke:
    cargo xtask qualification-leaf

[private]
_production-baseline source target: _clean-tree
    #!/usr/bin/env bash
    set -euo pipefail
    unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_COMMON_DIR GIT_NAMESPACE
    harness=$(git rev-parse --verify 'HEAD^{commit}')
    source=$(git rev-parse --verify --end-of-options {{quote(source + "^{commit}")}})
    target=""
    if [[ -n {{quote(target)}} ]]; then
      target=$(git rev-parse --verify --end-of-options {{quote(target + "^{commit}")}})
      if [[ "$source" == "$target" ]]; then
        printf 'production-baseline accept requires distinct resolved source and target commits\n' >&2
        exit 1
      fi
    fi
    origin=$(git remote get-url origin)
    case "$origin" in
      https://github.com/jaunder-org/jaunder.git|https://github.com/jaunder-org/jaunder|git@github.com:jaunder-org/jaunder.git|ssh://git@github.com/jaunder-org/jaunder.git) ;;
      *) printf 'production-baseline requires origin to be jaunder-org/jaunder\n' >&2; exit 1 ;;
    esac
    commits=("$harness")
    [[ "$source" == "$harness" ]] || commits+=("$source")
    if [[ -n "$target" && "$target" != "$harness" && "$target" != "$source" ]]; then
      commits+=("$target")
    fi
    # Re-request every SHA: ordinary fetch can accept locally complete objects
    # without asking the server whether it has them.
    git fetch --quiet --no-write-fetch-head --refetch origin "${commits[@]}"
    args=(--expected-harness "$harness" --source "$source")
    [[ -z "$target" ]] || args+=(--target "$target")
    cargo xtask qualification-leaf "${args[@]}"

# Internal implementations; Fix-mode check also uses doctests-workflow without
# imposing the public doctests clean-tree admission.
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

# Local workflows fail fast. Producer commands own their evidence and cleanup.
# Explicit variants avoid treating Just's own options as forwarded workflow flags.

check-no-test: (_host "fix" "host") _host-tests

check: check-no-test _product-tests _wasm-tests doctests-workflow

precommit:
    cargo xtask precommit-safety

# Called only by the focused Git/index safety owner, never by public precommit.
precommit-markdown-host: (_markdown "fix") _docs-shape

precommit-broad-host: check-no-test

prepush: _clean-tree (_host "verify" "prepush") _host-tests _product-tests _workspace-doctests

validate-no-e2e: _clean-tree (_host "verify" "host") _platform _static _wasm-budget _host-tests _wasm-tests coverage doctests _elisp-coverage

# The full E2E workflow owns producer settlement, reconciliation and authoritative
# SQLite/Chromium flow verification; static inventory remains in the host surface.
validate: validate-no-e2e
    just e2e

ci-validate-host: _clean-tree (_host "verify" "host") _platform _host-tests

ci-validate-hermetic: _clean-tree _static _wasm-budget

ci-validate-test-checks: _clean-tree _wasm-tests doctests _elisp-coverage

ci-validate-coverage: coverage

[private]
_markdown mode:
    {{prettier}} prettier-markdown {{mode}}
    {{prettier}} prettier-end2end {{mode}}

[private]
_docs-shape:
    cargo xtask check-leaf sequence-check
    cargo xtask check-leaf adr-format
    cargo xtask check-leaf adr-readme-parity
    cargo xtask check-leaf adr-view-parity
    cargo xtask check-leaf doc-links
    cargo xtask check-leaf flow-docs

[private]
_host mode docs:
    {{devtool}} check fmt {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check leptosfmt {{if mode == "fix" { "--fix" } else { "" }}}
    {{if docs + "-" + os() + "-" + arch() == "prepush-linux-x86_64" { "cargo xtask check-leaf prepush-docs" } else { prettier + " prettier-markdown " + mode }}}
    {{prettier}} prettier-end2end {{mode}}
    {{prettier}} prettier-css {{mode}}
    {{devtool}} check elisp-fmt {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check tools-fmt {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check ast-grep-tests {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check no-full-reload {{if mode == "fix" { "--fix" } else { "" }}}
    cargo fmt --manifest-path xtask/Cargo.toml {{if mode == "fix" { "" } else { "--check" }}}
    just _docs-shape
    cargo xtask check-leaf test-patterns
    cargo xtask check-leaf server-fn-registrar
    cargo xtask check-leaf server-fn-tracing
    cargo xtask check-leaf server-fn-coverage
    cargo xtask check-leaf server-fn-wire-arg-error
    cargo xtask check-leaf traced-context
    cargo xtask check-leaf e2e-telemetry-boundary
    cargo xtask check-leaf e2e-collector-config
    cargo xtask check-leaf proffered-secret
    cargo xtask check-leaf e2e-goto-wrapper
    cargo xtask check-leaf e2e-server-fn-endpoints
    cargo xtask check-leaf target-arch-placement
    cargo xtask check-leaf write-transaction-contract
    cargo xtask check-leaf common-host-target-closure
    cargo xtask check-leaf lint-suppression
    cargo xtask check-leaf thin-components
    cargo xtask check-leaf sqlx-newtype-bind
    cargo xtask check-leaf sqlx-newtype-decode
    cargo xtask check-leaf doctest-fences
    cargo xtask check-leaf raw-html-door
    cargo xtask check-leaf html-sink
    cargo xtask check-leaf e2e-scaffold
    cargo xtask check-leaf xlang-literal
    {{devtool}} check byte-compile {{if mode == "fix" { "--fix" } else { "" }}}
    just tsc {{mode}}
    {{devtool}} check cargo-deny {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check product-core-clippy {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check web-server-clippy {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check web-no-server-clippy {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check client-host-clippy {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check wasm-clippy {{if mode == "fix" { "--fix" } else { "" }}}
    {{devtool}} check tools-clippy {{if mode == "fix" { "--fix" } else { "" }}}
    cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings
    {{devtool}} check ert {{if mode == "fix" { "--fix" } else { "" }}}

[private]
_host-tests:
    cargo test --manifest-path xtask/Cargo.toml --lib --bins --tests
    cargo test --manifest-path tools/Cargo.toml --lib --bins --tests

[private]
_product-tests:
    cargo xtask test-local

[private]
_workspace-doctests:
    cargo xtask check-leaf workspace-doctests

[private]
_platform:
    nix eval --impure --expr 'import ./nix/platform-contract.nix { flake = builtins.getFlake (toString ./.); }'

[private]
_static: (_nix "static-docs") (_nix "static-typescript") (_nix "rust-format") (_nix "static-code") (_nix "static-elisp") (_nix "static-e2e-census") (_nix "static-ert") (_nix "xtask-format") (_nix "xtask-clippy")

[private]
_wasm-budget:
    cargo xtask check-leaf wasm-budget

[private]
_wasm-tests: (_nix "wasm-tests")

[private]
_elisp-coverage: (_nix "elisp-coverage-producer")
    cargo xtask check-leaf elisp-coverage-consume

[private]
_nix check:
    mkdir -p .xtask/gcroots
    nix build -L --accept-flake-config --out-link .xtask/gcroots/{{check}} .#checks.x86_64-linux.{{check}}

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

# Nix selects the producer set; this ordered host loop retains each output before
# the next measurement can run and validates status before continuing.
perf profile="medium" backends="sqlite,postgres" browsers="" workload="all" posts="" authors="" revisions="":
    #!/usr/bin/env bash
    set -euo pipefail
    nonce=$(cargo run --locked --quiet --manifest-path tools/Cargo.toml -p performance --bin perf-data -- nonce)
    run_root=".xtask/performance/$nonce"
    mkdir -p "$run_root/producers"
    parameters=$(jq -cn \
      --arg freshnessNonce "$nonce" \
      --arg profile {{ quote(profile) }} \
      --arg backends {{ quote(backends) }} \
      --arg browsers {{ quote(browsers) }} \
      --arg workload {{ quote(workload) }} \
      --arg posts {{ quote(posts) }} \
      --arg authors {{ quote(authors) }} \
      --arg revisions {{ quote(revisions) }} \
      '{freshnessNonce: $freshnessNonce, profile: $profile, backends: $backends, browsers: $browsers, workload: $workload, posts: $posts, authors: $authors, revisions: $revisions}')
    JAUNDER_PERF_PARAMETERS="$parameters" nix eval --impure --json \
      --expr 'let flake = builtins.getFlake (toString ./.); parameters = builtins.fromJSON (builtins.getEnv "JAUNDER_PERF_PARAMETERS"); in flake.performanceLib.${builtins.currentSystem}.selectPerformanceProducers parameters' \
      > "$run_root/selection.json"
    cargo run --locked --quiet --manifest-path tools/Cargo.toml -p performance --bin perf-data -- validate-selection --selection "$run_root/selection.json"
    jq -c '.producers[]' "$run_root/selection.json" > "$run_root/selected.ndjson"

    while IFS= read -r selected; do
      name=$(jq -er '.name' <<<"$selected")
      installable=$(jq -er '.installable' <<<"$selected")
      selected_identity=$(jq -er '.identity' <<<"$selected")
      kind=$(jq -er '.producer' <<<"$selected")
      backend=$(jq -er '.backend' <<<"$selected")
      browser=$(jq -r '.browser // empty' <<<"$selected")
      output=$(nix build --no-link --print-out-paths "$installable")
      [[ "$output" == "$selected_identity" ]] || { printf 'Nix output identity mismatches selection for %s: %s != %s\n' "$name" "$output" "$selected_identity" >&2; exit 1; }
      [[ -d "$output/performance" ]] || { printf 'Nix output lacks performance evidence for %s: %s\n' "$name" "$output" >&2; exit 1; }

      producer_root="$run_root/producers/$name"
      mkdir -p "$producer_root"
      cp -a -- "$output/performance" "$producer_root/performance"
      validator=(validate-producer --producer "$kind" --backend "$backend")
      if [[ -n "$browser" ]]; then
        validator+=(--browser "$browser")
      fi
      validator+=(--performance-root "$producer_root/performance")
      cargo run --locked --quiet --manifest-path tools/Cargo.toml -p performance --bin perf-data -- "${validator[@]}"
      jq -cn \
        --arg name "$name" \
        --arg identity "$output" \
        --arg producer "$kind" \
        --arg backend "$backend" \
        --arg browser "$browser" \
        --arg retained_root "$producer_root/performance" \
        '{name: $name, identity: $identity, producer: $producer, backend: $backend, browser: (if $browser == "" then null else $browser end), retained_root: $retained_root}' \
        > "$producer_root/metadata.json"
    done < "$run_root/selected.ndjson"

    jq -n \
      --arg profile {{ quote(profile) }} \
      --arg nonce "$nonce" \
      --argjson selection "$(jq -c '.selection' "$run_root/selection.json")" \
      --argjson producers "$(jq -cs '[.[]]' "$run_root"/producers/*/metadata.json)" \
      '{profile: $profile, freshness_nonce: $nonce, selection: $selection, producers: $producers}' \
      > "$run_root/invocation.json"
    cargo run --locked --quiet --manifest-path tools/Cargo.toml -p performance --bin perf-data -- assemble --invocation "$run_root/invocation.json"

# Comparison is advisory; only explicit authenticated import may replace the baseline.
perf-compare result:
    cargo run --locked --quiet --manifest-path tools/Cargo.toml -p performance --bin perf-data -- compare --result {{quote(result)}}

perf-import-baseline run_id:
    cargo run --locked --quiet --manifest-path tools/Cargo.toml -p performance --bin perf-data -- import-baseline {{quote(run_id)}}

# Full hermetic E2E workflow: continue all producers, then reconcile both backends.
e2e jobs="":
    bash scripts/e2e-workflow all {{quote(jobs)}}

# CI uses the same isolated producer and artifact-only reconciliation boundaries.
e2e-producer lane:
    bash scripts/e2e-workflow producer {{quote(lane)}}

# Intentional snapshot update; ordinary producers and the full workflow only verify.
e2e-regenerate-coverage:
    bash scripts/e2e-workflow producer sqlite-chromium-unsplit regenerate

e2e-reconcile backend artifacts=".xtask/diagnostics":
    cargo xtask e2e-reconcile {{quote(backend)}} --catalog end2end/e2e-lanes.json --diagnostics-root {{quote(artifacts)}}

# Exhaustive read-only milestone candidates; never claims or dispatches work.
issue-discover milestone:
    bash scripts/issue-discover.sh {{quote(milestone)}}

# Explicit issue decisions; native gh owns mutations, shared glue verifies readback.
[positional-arguments]
issue-create title issue_type milestone priority labels body_file *blockers:
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/issue-metadata.sh
    prepare_issue "$1" "$2" "$3" "$4" "$5" "$6"
    shift 6
    validate_blockers "$@"
    gh api --method POST "repos/$repo/issues" --input - < "$work/input.json" > "$work/mutated.json"
    finish_issue "$@"

[positional-arguments]
issue-edit number title issue_type milestone priority labels body_file *blockers:
    #!/usr/bin/env bash
    set -euo pipefail
    source scripts/issue-metadata.sh
    number=$1
    [[ "$number" =~ ^[1-9][0-9]*$ ]] || { echo 'Expected issue number' >&2; exit 1; }
    shift
    prepare_issue "$1" "$2" "$3" "$4" "$5" "$6"
    shift 6
    validate_blockers "$@"
    gh api --method PATCH "repos/$repo/issues/$number" --input - < "$work/input.json" > "$work/mutated.json"
    jq -e --argjson expected "$number" '.number == $expected' "$work/mutated.json" > /dev/null
    finish_issue "$@"

# Prior recorded exact PR/head human approval is mandatory, not inferred here.
# --reenqueue additionally requires explicit one-shot recovery authorization.
pr-land number expected_head mode="ordinary":
    bash scripts/pr-land.sh {{quote(number)}} {{quote(expected_head)}} {{quote(mode)}}

# Caller must positively verify unmanaged ownership; never invoke in managed work.
pr-cleanup number expected_head ownership:
    bash scripts/pr-cleanup.sh {{quote(number)}} {{quote(expected_head)}} {{quote(ownership)}}

# Read-only caller-actionable PR observation; only this focused tool polls GitHub.
pr-watch *args:
    cargo run --locked --quiet --manifest-path tools/Cargo.toml -p pr-observer -- watch {{args}}
