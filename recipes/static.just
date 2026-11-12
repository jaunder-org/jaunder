# Shared pinned static definitions; callers select a named leaf, never a catalog.
product_sandbox := "env CARGO_NET_OFFLINE=true CARGO_HOME=" + quote(env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", ""))
tools_sandbox := "env CARGO_NET_OFFLINE=true CARGO_HOME=" + quote(env_var_or_default("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME", ""))

elisp-fmt mode="verify":
    emacs --batch -Q -l elisp/scripts/format.el -f {{if mode == "fix" { "jaunder-fmt-fix" } else if mode == "verify" { "jaunder-fmt-check" } else { error("expected fix or verify") }}}

ast-grep-tests:
    ast-grep test --config sgconfig.yml

no-full-reload:
    ast-grep scan --config sgconfig.yml --filter '^no-full-reload$' web/src client/src

byte-compile:
    emacs --batch -Q -l elisp/scripts/byte-compile.el

# TSC has no autofix; both accepted modes retain the same compiler invocation.
tsc _mode="verify":
    tsc --noEmit -p end2end/tsconfig.json

# The retained Instant owner times this fixed request, including Just startup.
ert:
    emacs --batch -Q -l elisp/scripts/run-tests.el

# The adapter admits scoped UTF-8 inputs and promotes the pin's per-file errors.
leptosfmt mode="verify":
    scoped-leptosfmt {{quote(mode)}}

[private]
_leptosfmt-cli mode:
    leptosfmt -x .direnv -x .git -x target {{if mode == "fix" { "" } else if mode == "verify" { "--check" } else { error("expected fix or verify") }}} web/src csr/src

# Preserve the selected group order, admitting every required home before Fix.
rust-format mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", ""))) (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME", ""))) (fmt mode lane) (leptosfmt mode) (tools-fmt mode lane)

# Fixed compositions preserve the former groups and the independently ordered all.
# Host all callers provide the current checkout's devtool and scoped adapter.
code mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", ""))) (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME", ""))) (elisp-fmt mode) ast-grep-tests no-full-reload byte-compile (cargo-deny mode lane) (product-core-clippy mode lane) (web-server-clippy mode lane) (web-no-server-clippy mode lane) (client-host-clippy mode lane) (wasm-clippy mode lane) (tools-clippy mode lane)

all mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", ""))) (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME", ""))) (fmt mode lane) (leptosfmt mode) (elisp-fmt mode) (tools-fmt mode lane) ast-grep-tests no-full-reload byte-compile (tsc mode) (cargo-deny mode lane) (product-core-clippy mode lane) (web-server-clippy mode lane) (web-no-server-clippy mode lane) (client-host-clippy mode lane) (wasm-clippy mode lane) (tools-clippy mode lane) _ert-request

[private]
_ert-request:
    devtool check ert

# Admission is separate so a selected population can admit both homes before work.
[private]
_admit lane home:
    @case {{quote(lane)}} in host) ;; sandbox) test -n {{quote(home)}} || { echo 'sandbox Cargo home is missing' >&2; exit 1; } ;; *) echo 'expected host or sandbox' >&2; exit 2 ;; esac

fmt mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", "")))
    {{if lane == "sandbox" { product_sandbox } else { "" }}} cargo {{if lane == "sandbox" { "--offline" } else { "" }}} fmt {{if mode == "fix" { "" } else if mode == "verify" { "--check" } else { error("expected fix or verify") }}}

tools-fmt mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME", "")))
    {{if lane == "sandbox" { tools_sandbox } else { "" }}} cargo {{if lane == "sandbox" { "--offline" } else { "" }}} fmt --manifest-path tools/Cargo.toml --all {{if mode == "fix" { "" } else if mode == "verify" { "--check" } else { error("expected fix or verify") }}}

cargo-deny mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", "")))
    {{if lane == "sandbox" { product_sandbox } else { "" }}} cargo {{if lane == "sandbox" { "--offline" } else { "" }}} deny check {{if lane == "sandbox" { "bans licenses sources" } else { "" }}}

product-core-clippy mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", "")))
    {{if lane == "sandbox" { product_sandbox } else { "" }}} cargo {{if lane == "sandbox" { "--offline" } else { "" }}} clippy --workspace --exclude web --exclude client --exclude csr --all-targets -- -D warnings

web-server-clippy mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", "")))
    {{if lane == "sandbox" { product_sandbox } else { "" }}} cargo {{if lane == "sandbox" { "--offline" } else { "" }}} clippy -p web --features server --all-targets -- -D warnings

web-no-server-clippy mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", "")))
    {{if lane == "sandbox" { product_sandbox } else { "" }}} cargo {{if lane == "sandbox" { "--offline" } else { "" }}} clippy -p web --no-default-features --all-targets -- -D warnings

client-host-clippy mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", "")))
    {{if lane == "sandbox" { product_sandbox } else { "" }}} cargo {{if lane == "sandbox" { "--offline" } else { "" }}} clippy -p client --features csr --all-targets -- -D warnings

wasm-clippy mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", "")))
    {{if lane == "sandbox" { product_sandbox } else { "" }}} cargo {{if lane == "sandbox" { "--offline" } else { "" }}} clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings

tools-clippy mode="verify" lane="host": (_admit (lane) (env_var_or_default("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME", "")))
    {{if lane == "sandbox" { tools_sandbox } else { "" }}} cargo {{if lane == "sandbox" { "--offline" } else { "" }}} clippy --manifest-path tools/Cargo.toml --workspace --all-targets -- -D warnings
