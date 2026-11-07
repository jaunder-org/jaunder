{
  system,
  pkgs,
  packageInternals,
}:
let
  inherit (packageInternals)
    visualFontConfig
    themeThumbnailEnvironment
    toolchain
    cargo-crap
    devtoolBin
    tracedRunBin
    emacsForCi
    leptosfmt
    wasm-bindgen-cli
    e2ePackage
    nodeModulesSetup
    appOfflineCargoHome
    ;

  # The shell caches only a thin transport, never a checkout's xtask executable.
  xtaskLauncher = pkgs.writeShellApplication {
    name = "cargo-xtask";
    runtimeInputs = [ pkgs.git pkgs.nix pkgs.coreutils ];
    text = builtins.readFile ../scripts/cargo-xtask;
  };

  # Everything `just validate` needs on the host (toolchain + the
  # static-check tools) plus what the Nix checks pull anyway — so the CI
  # shell shares those store paths rather than adding cost.
  ciInputs =
    [
      toolchain
      xtaskLauncher
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
      # Standalone runner is the sole PATH provider; devtool provides
      # independent in-sandbox commands.
      devtoolBin
      tracedRunBin
      nodeModulesSetup
      emacsForCi
      pkgs.jq
      # Just producers invoke Nix directly, not through the xtask launcher.
      pkgs.nix
      # Independent metadata reads in upload privacy tests, never a runtime editor.
      pkgs.exiftool
      leptosfmt
      # Rust's wasm32-unknown-unknown target invokes the generic `lld` driver on
      # Darwin; `cargo xtask e2e-local` owns the CSR wasm build, so the shell it
      # runs in must provide the linker instead of relying on a host install.
      pkgs.lld
      pkgs.nodejs
      pkgs.openssl
      pkgs.pkg-config
      pkgs.dav1d
      pkgs.playwright-test
      pkgs.postgresql_18
      # `cargo xtask e2e-local` supervises this pinned collector for its
      # shared VM/host JSONL trace pipeline.
      pkgs.opentelemetry-collector-contrib
      pkgs.prettier
      # xtask's executable NixOS-test-script regressions run the generated
      # Python helper under the same pinned interpreter as CI and developers.
      pkgs.python3
      pkgs.sqlite
      pkgs.typescript
      # Theme-package export assertions use this pinned ZIP reader on the host.
      pkgs.unzip
      # Host xtask steps opt Rust-compiling cargo invocations into
      # `RUSTC_WRAPPER=sccache`; xtask maintains the multi-checkout
      # `SCCACHE_BASEDIRS` registry at runtime.
      pkgs.sccache
      # `wasm-opt`, run by the shared locked Vite pipeline after `wasm-bindgen` (#836).
      pkgs.binaryen
      # The standalone and host-server CSR build callers share this recipe.
      pkgs.just
      wasm-bindgen-cli
    ];

  # Host-only E2E orchestration shell. Keep this separate from ciInputs:
  # Cargo is only the external-subcommand dispatcher here; scripts/cargo-xtask
  # builds its executable with Nix and runs that binary on the host. Guest/test
  # dependencies are declared by the Nix checks and are intentionally unaffected.
  ciE2eInputs = [
    pkgs.bash
    pkgs.coreutils
    pkgs.findutils
    pkgs.just
    pkgs.nix
    pkgs.cargo
    xtaskLauncher
    pkgs.git
    tracedRunBin
    pkgs.jq
    pkgs.openssl
    pkgs.dav1d
  ];

  # Interactive-only tools that `just validate` never invokes and no
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

  linuxBrowserEnv = {
    PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
    PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";
    FONTCONFIG_FILE = "${visualFontConfig}";
    JAUNDER_THEME_THUMBNAIL_BROWSER = "${pkgs.chromium}/bin/chromium";
  };

  darwinBrowserEnv = {
    # Playwright's browser binaries are not packaged for Darwin in nixpkgs.
    # Keep shell entry offline; the shellHook points Playwright at an
    # operator-managed cache whose contents match the pinned @playwright/test
    # version from Nix. The path needs runtime $HOME expansion, so it cannot be
    # a plain mkShell environment attribute.
    PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";
  };

  shellEnv = (if pkgs.stdenv.hostPlatform.isDarwin then darwinBrowserEnv else linuxBrowserEnv) // {
    RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
    LC_ALL = "C.UTF-8";
    TZ = "UTC";
    # The host `ert` step (run via `nix develop .#ci -c cargo xtask …`)
    # computes timezone->UTC from IANA zone names, which need a zone
    # database for `encode-time` to resolve. A clean CI runner has none
    # in this shell, so provide it deterministically rather than relying
    # on the host system's own TZDIR (which masked this locally). Mirrors
    # the ert-check derivation's TZDIR (#160).
    TZDIR = "${pkgs.tzdata}/share/zoneinfo";
    # Host checks that spawn Cargo for the product workspace use the same
    # reviewed, offline dependency closure as the sandboxed static checks.
    JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME = "${appOfflineCargoHome}";
    # Just's TSC workflow retains both pins after cd into another worktree,
    # where the IDE setup hook has not run.
    E2E_TYPES_NODE_MODULES = "${e2ePackage}/node_modules";
    E2E_PLAYWRIGHT_TEST = "${pkgs.playwright-test}/lib/node_modules/@playwright/test";
    shellHook = ''
      # Bind host temp routing to the checkout at entry, before provisioning.
      # Exit (not return) prevents nix develop from running a command after failure.
      if ! jaunder_checkout_root="$(git rev-parse --show-toplevel)" || [[ "$jaunder_checkout_root" != /* ]]; then
        printf 'jaunder: cannot resolve an absolute Git checkout root for TMPDIR\n' >&2
        exit 1
      fi
      jaunder_tmpdir="$jaunder_checkout_root/.xtask/tmp"
      # Exercise directory use as well as creation; an existing path is not proof
      # that host tools can create temporary files there.
      if ! mkdir -p -- "$jaunder_tmpdir" || ! jaunder_tmp_probe="$(mktemp "$jaunder_tmpdir/.shell-entry.XXXXXXXXXX")"; then
        printf 'jaunder: cannot create or use TMPDIR %s\n' "$jaunder_tmpdir" >&2
        exit 1
      fi
      if ! rm -- "$jaunder_tmp_probe"; then
        printf 'jaunder: cannot remove TMPDIR usability probe %s\n' "$jaunder_tmp_probe" >&2
        exit 1
      fi
      export TMPDIR="$jaunder_tmpdir"
      unset jaunder_checkout_root jaunder_tmpdir jaunder_tmp_probe

      ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
        export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.dav1d ]}:$LD_LIBRARY_PATH"
      ''}
      ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
        export PLAYWRIGHT_BROWSERS_PATH="''${XDG_CACHE_HOME:-$HOME/Library/Caches}/ms-playwright-jaunder/${pkgs.playwright-test.version}"
      ''}

      # IDEs need the same pinned links as the explicit TSC workflow.
      setup-e2e-node-modules "$E2E_TYPES_NODE_MODULES" "$E2E_PLAYWRIGHT_TEST"
    '';
  };
in
{
  # Lean shell used by CI (`nix develop .#ci -c just ci-validate-host`).
  ci = pkgs.mkShell (shellEnv // { buildInputs = ciInputs; });
  # The scheduled mutation-testing job, and nothing else. `cargo-mutants`
  # stays out of `ciInputs` on purpose: every PR job enters `.#ci`, and
  # none of them run mutants. This shell is `ciInputs` plus that one
  # tool, so the weekly job gets it without the pull-request path paying
  # for it. See .github/workflows/mutants.yml.
  mutants = pkgs.mkShell (shellEnv // { buildInputs = ciInputs ++ [ pkgs.cargo-mutants ]; });
  # Full interactive shell for local development.
  default = pkgs.mkShell (shellEnv // { buildInputs = ciInputs ++ devOnly; });
} // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
  # Only e2e-chromium and e2e-firefox-lane enter this host shell. Keep the
  # checkout-bound TMPDIR contract and native library prepend from shellEnv,
  # but do not inherit validation, browser, IDE-provisioning, or theme tools.
  ci-e2e = pkgs.mkShell {
    buildInputs = ciE2eInputs;
    LC_ALL = "C.UTF-8";
    TZ = "UTC";
    TZDIR = "${pkgs.tzdata}/share/zoneinfo";
    shellHook = ''
      if ! jaunder_checkout_root="$(git rev-parse --show-toplevel)" || [[ "$jaunder_checkout_root" != /* ]]; then
        printf 'jaunder: cannot resolve an absolute Git checkout root for TMPDIR\n' >&2
        exit 1
      fi
      jaunder_tmpdir="$jaunder_checkout_root/.xtask/tmp"
      if ! mkdir -p -- "$jaunder_tmpdir" || ! jaunder_tmp_probe="$(mktemp "$jaunder_tmpdir/.shell-entry.XXXXXXXXXX")"; then
        printf 'jaunder: cannot create or use TMPDIR %s\n' "$jaunder_tmpdir" >&2
        exit 1
      fi
      if ! rm -- "$jaunder_tmp_probe"; then
        printf 'jaunder: cannot remove TMPDIR usability probe %s\n' "$jaunder_tmp_probe" >&2
        exit 1
      fi
      export TMPDIR="$jaunder_tmpdir"
      unset jaunder_checkout_root jaunder_tmpdir jaunder_tmp_probe
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.dav1d ]}:$LD_LIBRARY_PATH"
    '';
  };
  theme-thumbnail = pkgs.mkShell {
    buildInputs = [ themeThumbnailEnvironment ];
    FONTCONFIG_FILE = "${visualFontConfig}";
    JAUNDER_THEME_THUMBNAIL_BROWSER = "${pkgs.chromium}/bin/chromium";
    LC_ALL = "C.UTF-8";
    TZ = "UTC";
  };
}
