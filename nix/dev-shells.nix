{
  system,
  pkgs,
  packageInternals,
}:
let
  inherit (packageInternals)
    visualFontConfig
    toolchain
    cargo-crap
    devtoolBin
    emacsForCi
    leptosfmt
    wasm-bindgen-cli
    e2ePackage
    ;

  # Everything `cargo xtask validate` needs on the host (toolchain + the
  # static-check tools) plus what the Nix checks pull anyway — so the CI
  # shell shares those store paths rather than adding cost.
  ciInputs =
    [
      toolchain
      pkgs.ast-grep
      pkgs.cachix
      cargo-crap
      pkgs.cargo-deny
      pkgs.cargo-llvm-cov
      pkgs.cargo-nextest
      pkgs.curl
      # The shell exports Nix OpenSSL through `LD_LIBRARY_PATH` for
      # host-built Rust binaries. Use the matching Nix git too: a
      # distro git would load that OpenSSL into its older host glibc
      # process, which fails before Nix can fetch git inputs (#815).
      pkgs.git
      # `devtool run -- <cmd>` for humans/agents, and the `shellHook`'s
      # `devtool provision-node-modules` (#229) — so it must be on PATH in the
      # CI shell too, not just the interactive one. Already built for the
      # coverage and static-checks derivations, so this adds no new build.
      devtoolBin
      emacsForCi
      pkgs.jq
      leptosfmt
      pkgs.nodejs
      pkgs.openssl
      pkgs.pkg-config
      pkgs.playwright-test
      pkgs.postgresql_16
      # `cargo xtask e2e-local` supervises this pinned collector for its
      # shared VM/host JSONL trace pipeline.
      pkgs.opentelemetry-collector-contrib
      pkgs.prettier
      pkgs.sqlite
      pkgs.typescript
      # Host xtask steps opt Rust-compiling cargo invocations into
      # `RUSTC_WRAPPER=sccache`; xtask maintains the multi-checkout
      # `SCCACHE_BASEDIRS` registry at runtime.
      pkgs.sccache
      # `wasm-opt`, run by `devtool csr-bundle` after `wasm-bindgen` (#836).
      # In `ciInputs` rather than `devOnly` because `cargo xtask build-csr`
      # invokes it on the host, so the CI shell needs it too.
      pkgs.binaryen
      wasm-bindgen-cli
    ]
    ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
      pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
    ];

  # Interactive-only tools that `cargo xtask validate` never invokes and no
  # Nix check pulls (the language servers are the bulk). Kept out of
  # `devShells.ci` so CI does not download/build them.
  devOnly = [
    pkgs.typescript-language-server
    pkgs.vscode-langservers-extracted
    pkgs.cargo-generate
    pkgs.cargo-mutants
    pkgs.sqlx-cli
    # `cargo xtask pr watch`/`pr land` shell out to `gh` (#729). Host-only
    # manual commands — no Nix check or CI job runs them — so this stays
    # out of `ciInputs`, like the other interactive tooling here.
    pkgs.gh
  ];

  shellEnv = {
    RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
    PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
    PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";
    FONTCONFIG_FILE = "${visualFontConfig}";
    # The host `ert` step (run via `nix develop .#ci -c cargo xtask …`)
    # computes timezone->UTC from IANA zone names, which need a zone
    # database for `encode-time` to resolve. A clean CI runner has none
    # in this shell, so provide it deterministically rather than relying
    # on the host system's own TZDIR (which masked this locally). Mirrors
    # the ert-check derivation's TZDIR (#160).
    TZDIR = "${pkgs.tzdata}/share/zoneinfo";
    # Store paths for `devtool provision-node-modules`. Exported as env
    # vars (rather than baked into the shellHook) so they survive `cd`
    # into a worktree — that is what lets `devtool check tsc` re-run the
    # provisioning there, where the shellHook never fired.
    E2E_TYPES_NODE_MODULES = "${e2ePackage}/node_modules";
    E2E_PLAYWRIGHT_TEST = "${pkgs.playwright-test}/lib/node_modules/@playwright/test";
    shellHook = ''
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:$LD_LIBRARY_PATH"

      # Provision end2end/node_modules (the tsc type-dep closure) so the
      # devShell `tsc` and IDEs can type-check end2end/ offline in this
      # checkout. The same subcommand runs in-process from `devtool check
      # tsc`, so worktrees self-heal there; see
      # tools/devtool/src/provision.rs for the full rationale.
      devtool provision-node-modules
    '';
  };
in
{
  # Lean shell used by CI (`nix develop .#ci -c cargo xtask validate`).
  ci = pkgs.mkShell (shellEnv // { buildInputs = ciInputs; });
  # The scheduled mutation-testing job, and nothing else. `cargo-mutants`
  # stays out of `ciInputs` on purpose: every PR job enters `.#ci`, and
  # none of them run mutants. This shell is `ciInputs` plus that one
  # tool, so the weekly job gets it without the pull-request path paying
  # for it. See .github/workflows/mutants.yml.
  mutants = pkgs.mkShell (shellEnv // { buildInputs = ciInputs ++ [ pkgs.cargo-mutants ]; });
  # Full interactive shell for local development.
  default = pkgs.mkShell (shellEnv // { buildInputs = ciInputs ++ devOnly; });
}
