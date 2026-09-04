{
  system,
  pkgs,
  fenix,
  crane,
  atom-fork,
}:
let
  # One explicit screenshot font universe for host baseline generation and
  # NixOS-VM comparison. The file embeds the DejaVu store path, so the font
  # derivation stays in both closures without ambient system-font lookup.
  visualFontConfig = pkgs.makeFontsConf {
    fontDirectories = [ pkgs.dejavu_fonts ];
  };
  toolchain = fenix.packages.${system}.fromToolchainFile {
    file = ../rust-toolchain.toml;
    sha256 = "sha256-A1abGIbOtcBSdrUMhDGrER3pRM1hQP4fp9gh3Y4PKc8=";
  };

  craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
  # Cargo source filters follow target closures. All workspace manifests remain
  # available for resolution; excluded members receive deterministic placeholder
  # targets so Cargo can parse them without hashing unrelated source bytes.
  workspaceMembers = [
    "client"
    "common"
    "csr"
    "host"
    "macros"
    "server"
    "storage"
    "test-support"
    "web"
  ];
  cargoWorkspaceInput =
    path:
    let
      relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
    in
    relative == "Cargo.toml"
    || relative == "Cargo.lock"
    || relative == "rust-toolchain.toml"
    || relative == ".cargo/config.toml"
    || builtins.any (member: relative == "${member}/Cargo.toml") workspaceMembers;
  cargoTargetSource =
    members: path: type:
    let
      relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
    in
    type == "directory"
    || cargoWorkspaceInput path
    || builtins.any (
      member:
      relative == "${member}/build.rs"
      || pkgs.lib.hasPrefix "${member}/src/" relative
    ) members;
  workspacePlaceholderTargets =
    member:
    if member == "server" then
      [
        "src/lib.rs"
        "src/main.rs"
        "tests/main.rs"
      ]
    else if member == "test-support" then
      [
        "src/lib.rs"
        "src/main.rs"
      ]
    else
      [ "src/lib.rs" ];
  withWorkspacePlaceholders =
    name: source: excludedMembers:
    pkgs.runCommand name { } ''
      cp --no-preserve=mode -r ${source}/. "$out/"
      ${
        pkgs.lib.concatMapStringsSep "\n" (
          member:
          pkgs.lib.concatMapStringsSep "\n" (
            target: ''
              mkdir -p "$out/${member}/$(dirname ${target})"
              printf '%s\n' '// target-closure placeholder; excluded source remains absent.' > "$out/${member}/${target}"
            ''
          ) (workspacePlaceholderTargets member)
        ) excludedMembers
      }
    '';
  siteSrc = withWorkspacePlaceholders
    "jaunder-site-cargo-source"
    (pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      filter =
        path: type:
        cargoTargetSource [ "csr" "web" "client" "common" "macros" ] path type
        || pkgs.lib.hasSuffix "csr/index.html" path;
    })
    [ "host" "server" "storage" "test-support" ];
  wasmTestSrc = withWorkspacePlaceholders
    "jaunder-wasm-test-cargo-source"
    (pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      filter = cargoTargetSource [ "client" "common" "macros" ];
    })
    [ "csr" "host" "server" "storage" "test-support" "web" ];


  src = pkgs.lib.cleanSourceWith {
    src = craneLib.path ../.;
    filter =
      path: type:
      # xtask/ is the host-only dev driver (a separate workspace these
      # derivations never build). Excluding it keeps driver edits from
      # busting the app caches AND guarantees a derivation can never run a
      # stale xtask: it is not in the sandbox, so an accidental
      # `cargo xtask` fails loudly rather than running stale. xtask runs
      # only on the host (dev box / CI runner).
      # Nix assembly is not application source; exclude only its top-level root.
      !(type == "directory" && path == "${toString (craneLib.path ../.)}/nix")
      && (!pkgs.lib.hasInfix "/xtask/" path)
      && (
        (pkgs.lib.hasSuffix ".sql" path)
        || (pkgs.lib.hasSuffix ".css" path)
        # The CSR SPA shell the server embeds via include_str! (#239). Specific
        # (not a broad .html suffix) to keep stray HTML out of the crane src.
        || (pkgs.lib.hasSuffix "csr/index.html" path)
        || (builtins.match "scripts/.*" path != null)
        || (craneLib.filterCargoSources path type)
      );
  };

  # The #813 draft ADR pins atom_syndication's namespace-aware upstream
  # revision. Substitute its flake checkout during vendoring so product
  # builds resolve the Cargo git patch without sandbox network access.
  cargoVendorDir = craneLib.vendorCargoDeps {
    inherit src;
    overrideVendorGitCheckout =
      ps: drv:
      let
        p = builtins.head ps;
      in
      if p.name == "atom_syndication" then
        pkgs.runCommandLocal "atom-fork-vendor-${p.name}-${p.version}" { } ''
          dst="$out/${p.name}-${p.version}"
          mkdir -p "$dst"
          cp -a ${atom-fork}/. "$dst/"
          chmod -R u+w "$dst"
          echo '{"files":{},"package":null}' > "$dst/.cargo-checksum.json"
        ''
      else
        drv;
  };

  commonArgs = {
    inherit src cargoVendorDir;
    pname = "jaunder";
    version = "0.1.0";
    strictDeps = true;
    RUST_MIN_STACK = "16777216";
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [
      pkgs.openssl
      pkgs.sqlite
    ]
    ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
      pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
    ];
  };

  mkOfflineCargoHome =
    { name, vendorDir }:
    pkgs.runCommand "${name}-cargo-home" { } ''
      mkdir -p $out
      cp ${vendorDir}/config.toml $out/config.toml
      chmod u+w $out/config.toml
      cat >> $out/config.toml <<EOF

      [net]
      offline = true
      EOF
    '';

  appCargoVendorDir = cargoVendorDir;
  appOfflineCargoHome = mkOfflineCargoHome {
    name = "jaunder";
    vendorDir = appCargoVendorDir;
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  # Compile-only and test-only gates do not need full DWARF. Keep these
  # overrides local to gate derivations so production packages and normal
  # human debug builds keep their documented profiles.
  leanDevProfile = {
    CARGO_PROFILE_DEV_DEBUG = "0";
  };
  leanTestProfile = {
    CARGO_PROFILE_TEST_DEBUG = "0";
  };
  leanDevAndTestProfile = leanDevProfile // leanTestProfile;

  cargoArtifactsLeanDev = craneLib.buildDepsOnly (commonArgs // leanDevProfile);

  jaunderBin = craneLib.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      cargoExtraArgs = "-p jaunder";
      # Embed the CSR bundle + public assets into the binary (#237): the
      # release artifact is self-contained (ADR-0003/0008), serving pkg/*
      # and public/* with no external files. `server/build.rs` stages these
      # into the embed; the env vars are its inputs (the crane `src` filter
      # admits neither the bundle nor public/, so they arrive via env). This
      # is the build-order edge that makes the binary depend on the bundle.
      JAUNDER_CSR_BUNDLE_DIR = "${csrWasmBundle}";
      JAUNDER_PUBLIC_DIR = "${../public}";
      # Tests are covered by the separate `coverage` check (which runs the
      # instrumented nextest suite) and, for doctests — which nextest
      # structurally cannot run — the separate `doctests` check. Disabling
      # here avoids a redundant `cargo test` compile + run during the
      # package build.
      doCheck = false;
    }
  );

  # The out-of-process e2e seed helper (ADR-0046). Built as its own small
  # crane package (no leptos/wasm/web deps; shares cargoArtifacts) and placed
  # ONLY on the e2e VM PATH — deliberately absent from the `jaunder` prod
  # binary and the `services.jaunder` NixOS module, so there is no seed
  # surface anywhere near the release artifact.
  testSupportBin = craneLib.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      pname = "test-support";
      cargoExtraArgs = "-p test-support";
      doCheck = false;
    }
  );

  # The auxiliary tools workspace is separate from the product workspace
  # (ADR-0141). Keep its source and cargo artifacts separate from
  # `commonArgs`/`cargoArtifacts`: `tools/Cargo.lock` owns these deps, while
  # `xtask/` remains host-only and outside the flake source (ADR-0028).
  toolsSrc = pkgs.lib.cleanSourceWith {
    src = craneLib.path ../tools;
    filter = craneLib.filterCargoSources;
  };
  toolsArgs = {
    src = toolsSrc;
    pname = "jaunder-tools";
    version = "0.1.0";
    strictDeps = true;
  };
  toolsCargoArtifacts = craneLib.buildDepsOnly toolsArgs;
  toolsCargoVendorDir = craneLib.vendorCargoDeps toolsArgs;
  toolsOfflineCargoHome = mkOfflineCargoHome {
    name = "jaunder-tools";
    vendorDir = toolsCargoVendorDir;
  };

  # The in-sandbox dev tool (tools/ workspace: devtool + its coverage and
  # doctests path-deps). The offline coverage/doctests sandboxes run it
  # from PATH (nativeBuildInputs) instead of an in-sandbox `cargo run`,
  # whose deps would not be vendored.
  devtoolBin = craneLib.buildPackage (
    toolsArgs
    // {
      cargoArtifacts = toolsCargoArtifacts;
      pname = "devtool";
      cargoExtraArgs = "-p devtool";
      doCheck = false;
    }
  );

  cargo-crap = pkgs.callPackage (
    {
      lib,
      fetchCrate,
      fetchFromGitHub,
      rustPlatform,
    }:
    let
      crateSrc = fetchCrate {
        pname = "cargo-crap";
        version = "0.2.2";
        hash = "sha256-cZ30mdHHLXzpvMhkC6XoPMgfqAdsmdqhEfHq8T15Fmw=";
      };
    in
    rustPlatform.buildRustPackage (finalAttrs: {
      pname = "cargo-crap";
      version = "0.2.2";

      src = fetchFromGitHub {
        owner = "minikin";
        repo = "cargo-crap";
        rev = "v${finalAttrs.version}";
        hash = "sha256-yDoHqkMittJEFYxjpEb/C4+0sRg7ZnMpRO7a9aw5NvI=";
      };

      cargoLock.lockFile = "${crateSrc}/Cargo.lock";

      postPatch = ''
        ln -s ${crateSrc}/Cargo.lock Cargo.lock
      '';

      meta = {
        description = "Compute the CRAP (Change Risk Anti-Patterns) metric for Rust projects";
        mainProgram = "cargo-crap";
        homepage = "https://github.com/minikin/cargo-crap";
        changelog = "https://github.com/minikin/cargo-crap/blob/v${finalAttrs.version}/CHANGELOG.md";
        license = lib.licenses.mit;
        maintainers = [ lib.maintainers.mdorman ];
      };
    })
  ) { };

  # `buildRustPackage` requires a flat vendor directory plus Cargo.lock.
  # Crane fetches from static.crates.io reliably but groups packages by
  # registry hash, so adapt that output without re-downloading crates.
  vendorCargoDepsForBuildRustPackage =
    { name, src }:
    let
      vendor = craneLib.vendorCargoDeps { inherit src; };
      cratesIoDir = builtins.hashString "sha256" "registry+https://github.com/rust-lang/crates.io-index";
    in
    pkgs.runCommand "${name}-cargo-deps" { } ''
      mkdir -p $out
      cp -r ${vendor}/${cratesIoDir}/. $out/
      cp ${src}/Cargo.lock $out/Cargo.lock
    '';

  wasm-bindgen-cli = pkgs.wasm-bindgen-cli.overrideAttrs (old: rec {
    version = "0.2.121";
    src = pkgs.fetchCrate {
      pname = "wasm-bindgen-cli";
      inherit version;
      hash = "sha256-ZOMgFNOcGkO66Jz/Z83eoIu+DIzo3Z/vq6Z5g6BDY/w=";
    };
    cargoDeps = vendorCargoDepsForBuildRustPackage {
      name = "wasm-bindgen-cli";
      inherit src;
    };
  });

  wasmTestWebdriverConfig = pkgs.writeText "wasm-bindgen-test-webdriver.json" (
    builtins.toJSON {
      "goog:chromeOptions" = {
        binary = "${pkgs.chromium}/bin/chromium";
        args = [
          "--no-sandbox"
          "--disable-dev-shm-usage"
        ];
      };
    }
  );

  # leptosfmt pinned past its last release (#420): 0.1.33 mangles wrapping
  # generic component tags; the fix is merged upstream but unreleased.
  # REMOVE THIS OVERRIDE once a leptosfmt release later than 0.1.33
  # exists: drop this binding and take `pkgs.leptosfmt` again. The
  # override mechanics (`src` swap, the `cargoDeps` cascade, Crane's
  # static.crates.io vendoring adapter, and why `version` stays "0.1.33")
  # are in docs/adr/0118-leptosfmt-pinned-past-release.md.
  leptosfmt = pkgs.leptosfmt.overrideAttrs (_old: rec {
    src = pkgs.fetchFromGitHub {
      owner = "bram209";
      repo = "leptosfmt";
      rev = "8b4194ba33eee417ababdd15498940014fd6d237";
      # PR #167 bumps a `prettyplease` submodule; replacing `src`
      # wholesale drops nixpkgs' own `fetchSubmodules`, so it is restated.
      fetchSubmodules = true;
      hash = "sha256-F06Ag99rCn3qZywdxyP7ULOgyhbSzWNe+drBDZJWVxo=";
    };
    # Overriding `src` alone is not enough: nixpkgs passes `cargoHash`,
    # which `buildRustPackage` consumes *before* `overrideAttrs` applies,
    # so the 0.1.33 vendor tree would survive a bare `src` swap.
    cargoDeps = vendorCargoDepsForBuildRustPackage {
      name = "leptosfmt";
      inherit src;
    };
  });

  # The CSR client's wasm bundle (`pkg/*`) + public assets, assembled as a
  # tree. The server does not serve this from disk — it embeds the bundle
  # + public assets (#237) and the SPA shell (#239) into the binary. `site`
  # exists because `cargo xtask audit-wasm` builds `.#site` and inspects
  # `$out/pkg/jaunder.{wasm,js}` for bundle-size analysis (ADR-0028).
  site = pkgs.runCommand "jaunder-site" { } ''
    mkdir -p $out/pkg
    cp -r ${csrWasmBundle}/. $out/pkg/
    cp -r ${../public}/. $out/
  '';

  # --- leptos-CSR client (#177/#180) --------------------------------------
  # The client-side-render wasm binary — the only client (#180).
  # `csrWasmBundle` runs wasm-bindgen over it; `site`
  # (above) bundles it with the public assets + the CSR SPA shell.
  csrWasm = craneLib.buildPackage (
    commonArgs
    // {
      src = siteSrc;
      cargoArtifacts = craneLib.buildDepsOnly (
        commonArgs
        // {
          src = siteSrc;
          CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
          cargoExtraArgs = "-p csr";
          doCheck = false;
        }
      );
      CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
      cargoExtraArgs = "-p csr";
      doCheck = false;
      installPhaseCommand = ''
        mkdir -p $out/lib
        cp target/wasm32-unknown-unknown/release/csr.wasm $out/lib/
      '';
    }
  );

  # Measurement-only direct-init arm label. Empty is committed and preserves
  # the normal e2e derivation hash; set for one #864 measurement arm, then
  # revert before committing.
  wasmExperimentArm = "";
  wasmShapeSection = "";
  wasmShapeSectionCount = 1;

  csrWasmBundle =
    pkgs.runCommand "jaunder-csr-wasm-bundle"
      {
        nativeBuildInputs = [
          devtoolBin
          pkgs.binaryen
          wasm-bindgen-cli
        ];
      }
      ''
        # Post-process the crane-built csr.wasm into the served bundle
        # (pkg/jaunder.{js,wasm}) via the shared `devtool csr-bundle` — the
        # SAME implementation the host build (`cargo xtask build-csr`) runs, so
        # host and Nix cannot drift (#236). devtool shells out to
        # `wasm-bindgen` (on PATH here) and does the rename + js wasm-ref fix.
        devtool csr-bundle --wasm ${csrWasm}/lib/csr.wasm --out $out${pkgs.lib.optionalString (wasmExperimentArm != "") " --wasm-experiment-arm ${wasmExperimentArm}"}${pkgs.lib.optionalString (wasmShapeSection != "") " --wasm-shape-section ${wasmShapeSection} --wasm-shape-section-count ${toString wasmShapeSectionCount}"}
      '';

  e2ePackage = pkgs.buildNpmPackage {
    name = "jaunder-e2e";
    src = ../end2end;
    npmDepsHash = "sha256-9rjRjO+430wgKWPJnFM0t2rRcZyeE3pipyTTPIZvD8U=";
    dontNpmBuild = true;
    installPhase = ''
      mkdir -p $out
      cp -r . $out/
    '';
  };

  emacsSrc = pkgs.lib.cleanSourceWith {
    src = ../elisp;
  };

  # One emacs for both the host verify gate (the xtask StepSpecs) and the
  # hermetic nix checks, so they cannot diverge. withPackages (vs bare
  # pkgs.emacs) is the extension point for units C/D to add elisp packages
  # via nix. `plz` is the AtomPub client's HTTP transport (ADR-0037) — it
  # drives the `curl` binary, so anything running plz also needs `curl` on
  # PATH (the e2e VM and the ci dev shell, below). cmark-el is fetched at
  # a fixed upstream revision because it is neither packaged by Nixpkgs nor
  # MELPA; fetched source preserves the upstream license notices.
  emacsForCi = pkgs.emacs.pkgs.withPackages (
    epkgs:
    let
      cmarkEl = epkgs.trivialBuild {
        pname = "cmark";
        version = "0.29.3";
        src = pkgs.fetchFromGitHub {
          owner = "taku0";
          repo = "cmark-el";
          rev = "86fe43daeea967f00992936b0917272e89a0967b";
          hash = "sha256-SKO7GB4m9Qojv3GWwkmmDXCdE+JREIk3EzgZ8imUI7o=";
        };
        preInstall = ''
          mkdir -p "$out/share/emacs/site-lisp/maps"
          cp "$src"/maps/*.json "$out/share/emacs/site-lisp/maps/"
          cp "$src/LICENSE" "$out/share/emacs/site-lisp/"
        '';
      };
    in
    [
      epkgs.plz
      epkgs.undercover
      cmarkEl
    ]
  );
in
{
  packages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
    jaunder = jaunderBin;
    site = site;
    # The pre-wasm-bindgen, unstripped wasm. Exposed so
    # `cargo xtask audit-wasm --breakdown` has an artifact that still
    # carries a name section: `wasm-opt` strips names from the shipped
    # bundle, so the shipped file cannot be attributed to crates (#836).
    inherit csrWasm;
    devtool = devtoolBin;
    # The out-of-process e2e seed helper (ADR-0046). Exposed so it is
    # directly buildable/verifiable; it is placed only on the e2e VM PATH,
    # never in the prod artifact or the NixOS module.
    test-support = testSupportBin;
  };

  internals = {
    inherit
      visualFontConfig
      toolchain
      craneLib
      commonArgs
      wasmTestSrc
      appOfflineCargoHome
      toolsOfflineCargoHome
      cargoArtifacts
      leanTestProfile
      leanDevAndTestProfile
      jaunderBin
      testSupportBin
      devtoolBin
      cargo-crap
      wasm-bindgen-cli
      wasmTestWebdriverConfig
      leptosfmt
      csrWasmBundle
      e2ePackage
      emacsSrc
      emacsForCi
      ;
  };
}
