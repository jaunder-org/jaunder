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
  # wasm-bindgen #5268 identifies nightly-2026-08-05 as the last LLVM 22
  # nightly; this earlier dated pin stays in that compatible range.  It must
  # never track Fenix `latest`: LLVM coverage data, minicov's C runtime, and
  # the manual Clang link are one version-sensitive unit.
  diagnosticNightlyName = "nightly-2026-07-27";
  diagnosticNightlySha256 = "sha256-e0NxVNFY345jKKjY/QdZiWrqKmDRBvmohTt4ZuKwx1A=";
  diagnosticNightly = fenix.packages.${system}.fromToolchainName {
    name = diagnosticNightlyName;
    sha256 = diagnosticNightlySha256;
  };
  diagnosticToolchain = fenix.packages.${system}.combine [
    (diagnosticNightly.withComponents [
      "cargo"
      "llvm-tools-preview"
      "rust-std"
      "rustc"
    ])
    (fenix.packages.${system}.targets.wasm32-unknown-unknown.fromToolchainName {
      name = diagnosticNightlyName;
      sha256 = diagnosticNightlySha256;
    }).rust-std
  ];
  # minicov builds compiler-rt C for wasm.  The unwrapped Clang avoids Nix's
  # host cc-wrapper injecting non-wasm sysroot and hardening flags.
  diagnosticClang = pkgs.llvmPackages_22.clang-unwrapped;
  # This source is an input only of the diagnostic derivative. The production
  # workspace lock, vendor source, `csrWasm`, and `csrWasmBundle` never name it.
  diagnosticMinicov = pkgs.fetchCrate {
    pname = "minicov";
    version = "0.3.8";
    hash = "sha256-MAyEaF1M9mPr0rQRjG21XJaWgxwvOWZrObjCeFidgoA=";
  };
  # `llvm-tools-preview` installs profile tools below rustlib rather than the
  # cargo/rustc bin directory.  Keep this path explicit so every diagnostic
  # identity and exported analyzer symlink names the compiler-matched tools.
  diagnosticLlvmTools =
    "${diagnosticToolchain}/lib/rustlib/${pkgs.stdenv.hostPlatform.config}/bin";
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
      relative == "${member}/Cargo.toml"
      || relative == "${member}/build.rs"
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
        cargoTargetSource [ "csr" "web" "client" "common" "macros" "tools/csr_bundle" ] path type
        || pkgs.lib.hasSuffix "csr/index.html" path;
    })
    [ "host" "server" "storage" "test-support" ];
  wasmTestSrc = withWorkspacePlaceholders
    "jaunder-wasm-test-cargo-source"
    (pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      filter =
        path: type:
        let
          relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
        in
        cargoTargetSource [ "client" "common" "macros" "tools/csr_bundle" ] path type
        || pkgs.lib.hasPrefix "client/tests/" relative;
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

  # Native AVIF decoding is host-only; keep dav1d out of `commonArgs`, which
  # also builds the wasm CSR package.
  hostArgs = commonArgs // {
    buildInputs = commonArgs.buildInputs ++ [ pkgs.dav1d ];
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

  cargoArtifacts = craneLib.buildDepsOnly hostArgs;

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
    hostArgs
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
      nativeBuildInputs =
        hostArgs.nativeBuildInputs
        ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.patchelf ];
      postFixup = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
        patchelf --add-rpath \
          "${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.dav1d ]}" \
          "$out/bin/jaunder"
      '';
    }
  );

  # The out-of-process e2e seed helper (ADR-0046). Built as its own small
  # crane package (no leptos/wasm/web deps; shares cargoArtifacts) and placed
  # ONLY on the e2e VM PATH — deliberately absent from the `jaunder` prod
  # binary and the `services.jaunder` NixOS module, so there is no seed
  # surface anywhere near the release artifact.
  testSupportBin = craneLib.buildPackage (
    hostArgs
    // {
      inherit cargoArtifacts;
      pname = "test-support";
      cargoExtraArgs = "-p test-support";
      doCheck = false;
      nativeBuildInputs =
        hostArgs.nativeBuildInputs
        ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.patchelf ];
      postFixup = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
        patchelf --add-rpath \
          "${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.dav1d ]}" \
          "$out/bin/test-support"
      '';
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
  # whose deps would not be vendored. `csr/index.html` remains the one
  # tracked shell template; materialize the declared store input at the
  # relative compile-time include path without widening toolsSrc to product
  # sources or copying another tracked template.
  devtoolBin = craneLib.buildPackage (
    toolsArgs
    // {
      cargoArtifacts = toolsCargoArtifacts;
      pname = "devtool";
      cargoExtraArgs = "-p devtool";
      preBuild = (toolsArgs.preBuild or "") + ''
        mkdir -p ../csr
        cp ${../csr/index.html} ../csr/index.html
      '';
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

  # The CSR client's rendered shell, content-addressed `pkg/*` runtime assets,
  # and public assets, assembled as a tree. The manifest remains build-only:
  # server/build.rs receives the bundle root directly, while site exposes only
  # the rendered shell and declared served assets.
  site = pkgs.runCommand "jaunder-site" { } ''
    mkdir -p $out/pkg
    cp ${csrWasmBundle}/index.html $out/
    cp -r ${csrWasmBundle}/pkg/. $out/pkg/
    cp -r ${../public}/. $out/
  '';

  # --- leptos-CSR client (#177/#180) --------------------------------------
  # The client-side-render wasm binary — the only client (#180).
  # `csrWasmBundle` runs wasm-bindgen over it; `site`
  # (above) bundles it with the public assets + rendered CSR shell.
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
        # Post-process the crane-built csr.wasm into a content-addressed CSR
        # bundle via the shared `devtool csr-bundle` — the SAME implementation
        # the host build (`cargo xtask build-csr`) runs, so host and Nix cannot
        # drift (#236). The resulting root holds the build-only manifest,
        # rendered shell, and served `pkg/` assets.
        devtool csr-bundle --wasm ${csrWasm}/lib/csr.wasm --out $out${pkgs.lib.optionalString (wasmExperimentArm != "") " --wasm-experiment-arm ${wasmExperimentArm}"}${pkgs.lib.optionalString (wasmShapeSection != "") " --wasm-shape-section ${wasmShapeSection} --wasm-shape-section-count ${toString wasmShapeSectionCount}"}
      '';

  # Separately instrumented CSR producer for the Playwright/WASM coverage probe.
  #
  # Cargo is deliberately only an LLVM-IR producer here.  Its ordinary final
  # wasm link cannot supply minicov's profiler archive, and treating that failed
  # link as the diagnostic artifact would lose the exact source-mappable module.
  # The pinned nightly emits every target crate's IR without a root link; LLVM
  # 22 Clang compiles that exact closed graph and links it with minicov's archive.
  # This derivative is not an input to `site` or `jaunderBin`.
  diagnosticCsrWasmBundle = pkgs.runCommand "jaunder-diagnostic-csr-wasm-bundle"
    {
      nativeBuildInputs = [
        # Cargo build scripts are host programs and require Nix's ordinary
        # `cc`; the target-specific CC/CFLAGS below still force minicov's C
        # profiler runtime through unwrapped LLVM 22 Clang.
        pkgs.stdenv.cc
        diagnosticToolchain
        diagnosticClang
        devtoolBin
        pkgs.binaryen
        pkgs.python3
        wasm-bindgen-cli
      ];
    }
    ''
      mkdir -p $out/{instrumented/ir,tools}
      ln -s ${diagnosticLlvmTools}/llvm-cov $out/tools/llvm-cov
      ln -s ${diagnosticLlvmTools}/llvm-profdata $out/tools/llvm-profdata
      export PATH=${diagnosticToolchain}/bin:${diagnosticLlvmTools}:${diagnosticClang}/bin:$PATH
      rustc -Vv > $out/toolchain-rustc-vv.txt
      clang --version > $out/toolchain-clang-version.txt
      ${diagnosticLlvmTools}/llvm-profdata --version > $out/toolchain-llvm-profdata-version.txt
      ${diagnosticLlvmTools}/llvm-cov --version > $out/toolchain-llvm-cov-version.txt
      cat > $out/build-configuration.json <<'EOF'
      {"version":5,"nightly":"${diagnosticNightlyName} (rustc 1.99.0-nightly dc3f85158; LLVM 22.1.8)","nightly_source_sha256":"${diagnosticNightlySha256}","instrumented_first_party_crates":["csr"],"omitted_first_party_crates":["client","common","macros","web"],"producer":"Cargo emits only CSR LLVM IR and an rlink without a final link; pinned rustc -Zlink-only recreates its complete Cargo/sysroot/minicov link graph.","rustflags":["-Cinstrument-coverage","-Zno-profiler-runtime","--emit=llvm-ir","-C link-arg=--no-gc-sections","-Zno-link"],"linker":"pinned rustc -Zlink-only","c_compiler":"llvmPackages_22.clang-unwrapped","diagnostic_runtime":{"wrapper_crate":"diagnostic-coverage-runtime","crate":"minicov","version":"0.3.8","source_sha256":"4869b6a491569605d66d3952bcdf03df789e5b536e5f0cf7758a7f08a55ae24d"}}
      EOF
      work="$TMPDIR/wasm-coverage-csr"
      mkdir -p "$work"
      cp -r ${siteSrc}/. "$work/source"
      chmod -R u+w "$work/source"
      python3 - "$work/source" "${diagnosticMinicov}" "${../tools/diagnostic-coverage-runtime}" <<'PY'
      import pathlib
      import sys

      source, minicov, runtime = map(pathlib.Path, sys.argv[1:])
      root_manifest = source / "Cargo.toml"
      root = root_manifest.read_text()
      root = root.replace(
          "[patch.crates-io]\n",
          f'[patch.crates-io]\nminicov = {{ path = "{minicov}" }}\n',
          1,
      )
      root_manifest.write_text(root)
      client_manifest = source / "client/Cargo.toml"
      client = client_manifest.read_text()
      client = client.replace(
          "diagnostic-coverage = []",
          'diagnostic-coverage = ["dep:diagnostic-coverage-runtime"]',
          1,
      )
      client += f'\n[target.\'cfg(target_arch = "wasm32")\'.dependencies]\ndiagnostic-coverage-runtime = {{ path = "{runtime}", optional = true }}\n'
      client_manifest.write_text(client)
      PY
      python3 - "$out/source-identity.json" "$work/source" "${siteSrc}" <<'PY'
      import json
      import pathlib
      import sys

      output, compilation_directory, nix_source = map(pathlib.Path, sys.argv[1:])
      output.write_text(json.dumps({
          "version": 1,
          "source_identity": {
              "kind": "nix-store-source",
              "value": str(nix_source),
          },
          "compilation_directory": str(compilation_directory.resolve()),
          "path_equivalence": {
              "compiled_source_prefix": str(compilation_directory.resolve()),
              "retained_source_mappable_module": "instrumented/csr.wasm",
              "relationship": "The retained module is linked from LLVM IR compiled below compiled_source_prefix; the content-addressed wasm selected by pkg/manifest.json is its wasm-bindgen/wasm-opt derivative.",
          },
      }, indent=2) + "\n")
      PY

      set +e
      (
        set -e
        python3 - "$out/toolchain-rustc-vv.txt" "$out/toolchain-clang-version.txt" <<'PY'
      import pathlib
      import re
      import sys

      rustc, clang = (pathlib.Path(path).read_text() for path in sys.argv[1:])
      rust_llvm = re.search(r"^LLVM version: ([0-9]+)", rustc, re.MULTILINE)
      clang_llvm = re.search(r"clang version ([0-9]+)", clang)
      if rust_llvm is None or clang_llvm is None:
          raise SystemExit("could not establish Rust and Clang LLVM major versions")
      if {rust_llvm.group(1), clang_llvm.group(1)} != {"22"}:
          raise SystemExit(f"coverage pipeline requires LLVM 22; rustc={rust_llvm.group(1)} clang={clang_llvm.group(1)}")
      PY
        cd "$work/source"
        export CARGO_HOME=${appOfflineCargoHome}
        export CARGO_TARGET_DIR="$work/target"
        export CARGO_BUILD_TARGET=wasm32-unknown-unknown
        export CC_wasm32_unknown_unknown=clang
        export CFLAGS_wasm32_unknown_unknown="--target=wasm32-unknown-unknown"
        cargo rustc -p csr --features diagnostic-coverage --target wasm32-unknown-unknown --release --lib --message-format=json-render-diagnostics -- -Cinstrument-coverage -Zno-profiler-runtime --emit=llvm-ir -C link-arg=--no-gc-sections -Zno-link > "$work/cargo-artifacts.jsonl"
        cp "$work/cargo-artifacts.jsonl" "$out/instrumented/cargo-artifacts.jsonl"
        python3 - "$work/target/wasm32-unknown-unknown/release" "$work/root-ir" "$work/root-rlink" "$out/instrumented/ir-manifest.json" <<'PY'
      import json
      import pathlib
      import sys

      target_release, ir_output, rlink_output, manifest = map(pathlib.Path, sys.argv[1:])
      target_release = target_release.resolve()
      search_roots = (target_release, target_release / "deps")
      root_ir = sorted(
          path for directory in search_roots for path in directory.glob("csr*.ll") if path.is_file()
      )
      root_rlink = sorted(
          path for directory in search_roots for path in directory.glob("csr*.rlink") if path.is_file()
      )
      if len(root_ir) != 1 or len(root_rlink) != 1:
          raise SystemExit(f"expected one fresh csr LLVM IR and rlink; ir={root_ir}, rlink={root_rlink}")
      ir_output.write_text(f"{root_ir[0]}\n")
      rlink_output.write_text(f"{root_rlink[0]}\n")
      manifest.write_text(json.dumps({
          "version": 4,
          "root_ir_selection": ["release/csr*.ll", "release/deps/csr*.ll"],
          "root_rlink_selection": ["release/csr*.rlink", "release/deps/csr*.rlink"],
          "instrumented_root_ir": str(root_ir[0]),
          "rustc_link_metadata": str(root_rlink[0]),
          "cargo_artifacts": "cargo-artifacts.jsonl",
      }, indent=2) + "\n")
      PY
        root_ir="$(cat "$work/root-ir")"
        root_rlink="$(cat "$work/root-rlink")"
        cp "$root_ir" "$out/instrumented/csr.ll"
        cp "$root_rlink" "$out/instrumented/csr.rlink"
        rustc --target wasm32-unknown-unknown -Zlink-only "$out/instrumented/csr.rlink"
        python3 - "$work/target/wasm32-unknown-unknown/release" "$out/instrumented/ir-manifest.json" "$out/instrumented/csr.wasm" <<'PY'
      import json
      import pathlib
      import shutil
      import sys

      target_release, manifest_path, retained_wasm = map(pathlib.Path, sys.argv[1:])
      candidates = sorted(
          path
          for directory in (target_release, target_release / "deps")
          for path in directory.glob("csr*.wasm")
          if path.is_file()
      )
      if len(candidates) != 1:
          raise SystemExit(f"expected exactly one fresh CSR wasm after rustc -Zlink-only; found {candidates}")
      shutil.copyfile(candidates[0], retained_wasm)
      manifest = json.loads(manifest_path.read_text())
      manifest["linked_wasm_selection"] = ["release/csr*.wasm", "release/deps/csr*.wasm"]
      manifest["linked_wasm"] = str(candidates[0])
      manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
      PY
        devtool csr-bundle \
          --wasm "$out/instrumented/csr.wasm" \
          --out "$out/pkg" \
          --diagnostic-coverage-metadata "$out/coverage-metadata.json" \
          --diagnostic-toolchain-identity "$out/toolchain-identity.json" \
          --diagnostic-minicov-version 0.3.8
        python3 - "$out/coverage-metadata.json" <<'PY'
      import json
      import pathlib
      import sys

      metadata = json.loads(pathlib.Path(sys.argv[1]).read_text())
      if metadata["result"] != "preserved":
          raise SystemExit(f"coverage metadata was not retained: {metadata['result']}")
      if not metadata["input"]["wasm_bindgen_metadata"]:
          raise SystemExit("manual link omitted __wasm_bindgen_unstable before wasm-bindgen")
      PY
      ) > "$out/pipeline.log" 2>&1
      pipeline_exit=$?
      set -e

      python3 - "$out/status.json" "$pipeline_exit" <<'PY'
      import hashlib
      import json
      import pathlib
      import sys

      status_path = pathlib.Path(sys.argv[1])
      pipeline_exit = int(sys.argv[2])
      root = status_path.parent
      succeeded = pipeline_exit == 0

      def sha256(relative_path):
          path = root / relative_path
          if not path.is_file():
              return None
          return hashlib.sha256(path.read_bytes()).hexdigest()

      retained_module = "instrumented/csr.wasm"
      served_module = None
      if succeeded:
          manifest = json.loads((root / "pkg/manifest.json").read_text())
          wasm = next(asset for asset in manifest["assets"] if asset.get("role") == "wasm")
          served_module = wasm["path"]
      status = {
          "version": 3,
          "outcome": "succeeded" if succeeded else "failed",
          "pipeline_exit": pipeline_exit,
          "module": retained_module if succeeded else None,
          "bundle": served_module if succeeded else None,
          "served_module": {
              "path": served_module,
              "sha256": sha256(f"pkg/{served_module}"),
          } if succeeded else None,
          "source_mappable_module": {
              "path": retained_module,
              "sha256": sha256(retained_module),
              "relationship_to_served_module": "input to wasm-bindgen and wasm-opt that produced the content-addressed module selected by pkg/manifest.json",
          } if succeeded else None,
          "source_identity": "source-identity.json",
          "coverage_metadata": "coverage-metadata.json" if (root / "coverage-metadata.json").is_file() else None,
          "toolchain_identity": "toolchain-identity.json" if (root / "toolchain-identity.json").is_file() else None,
          "ir_manifest": "instrumented/ir-manifest.json" if (root / "instrumented/ir-manifest.json").is_file() else None,
          "diagnostic_log": "pipeline.log",
      }
      status_path.write_text(json.dumps(status, indent=2) + "\n")
      PY
    '';

  # The timing baseline intentionally shares the diagnostic pinned nightly, source
  # closure, wasm-bindgen/wasm-opt bundling, service, and focused browser flow. It
  # differs only by omitting -Cinstrument-coverage, minicov, and the diagnostic
  # exports; those are unavoidable because they are what this experiment measures.
  diagnosticBaselineCsrWasmBundle = pkgs.runCommand "jaunder-diagnostic-baseline-csr-wasm-bundle"
    {
      nativeBuildInputs = [ pkgs.stdenv.cc diagnosticToolchain devtoolBin pkgs.binaryen pkgs.python3 wasm-bindgen-cli ];
    }
    ''
      export PATH=${diagnosticToolchain}/bin:$PATH
      work="$TMPDIR/wasm-coverage-baseline"
      mkdir -p "$work"
      cp -r ${siteSrc}/. "$work/source"
      chmod -R u+w "$work/source"
      cd "$work/source"
      export CARGO_HOME=${appOfflineCargoHome}
      export CARGO_TARGET_DIR="$work/target"
      cargo build -p csr --target wasm32-unknown-unknown --release
      devtool csr-bundle --wasm "$work/target/wasm32-unknown-unknown/release/csr.wasm" --out "$out/pkg"
      python3 - "$out/status.json" <<'PY'
      import hashlib, json, pathlib, sys
      status = pathlib.Path(sys.argv[1])
      root = status.parent
      manifest = json.loads((root / "pkg/manifest.json").read_text())
      wasm = next(asset for asset in manifest["assets"] if asset.get("role") == "wasm")
      served_module = wasm["path"]
      status.write_text(json.dumps({
          "version": 1, "outcome": "succeeded",
          "served_module": {
              "path": served_module,
              "sha256": hashlib.sha256((root / "pkg" / served_module).read_bytes()).hexdigest(),
          },
          "unavoidable_deviations": [
              "omits -Cinstrument-coverage and minicov profiler runtime",
              "omits diagnostic-coverage feature and diagnostic browser exports",
          ],
      }, indent=2) + "\n")
      PY
    '';

  diagnosticBaselineJaunderBin = craneLib.buildPackage (
    hostArgs
    // {
      inherit cargoArtifacts;
      pname = "jaunder-diagnostic-wasm-coverage-baseline";
      cargoExtraArgs = "-p jaunder";
      JAUNDER_CSR_BUNDLE_DIR = "${diagnosticBaselineCsrWasmBundle}/pkg";
      JAUNDER_PUBLIC_DIR = "${../public}";
      doCheck = false;
      nativeBuildInputs =
        hostArgs.nativeBuildInputs
        ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.patchelf ];
      postFixup = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
        patchelf --add-rpath \
          "${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.dav1d ]}" \
          "$out/bin/jaunder"
      '';
    }
  );

  # The diagnostic browser probe must serve the same instrumented bundle that
  # it later records.  Keep this derivative separate from the release binary:
  # only the probe VM selects it through an explicit service override.
  diagnosticJaunderBin = craneLib.buildPackage (
    hostArgs
    // {
      inherit cargoArtifacts;
      pname = "jaunder-diagnostic-wasm-coverage";
      cargoExtraArgs = "-p jaunder";
      JAUNDER_CSR_BUNDLE_DIR = "${diagnosticCsrWasmBundle}/pkg";
      JAUNDER_PUBLIC_DIR = "${../public}";
      doCheck = false;
      nativeBuildInputs =
        hostArgs.nativeBuildInputs
        ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.patchelf ];
      postFixup = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
        patchelf --add-rpath \
          "${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.dav1d ]}" \
          "$out/bin/jaunder"
      '';
    }
  );

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
    csrBundle = csrWasmBundle;
    devtool = devtoolBin;
    # The out-of-process e2e seed helper (ADR-0046). Exposed so it is
    # directly buildable/verifiable; it is placed only on the e2e VM PATH,
    # never in the prod artifact or the NixOS module.
    # The manually linked diagnostic module and its retained IR/link evidence.
    # A failed producer still exposes its durable `status.json` and `pipeline.log`
    # for the later browser producer; no independent blocker derivation masks it.
    wasm-coverage-csr = diagnosticCsrWasmBundle;
    test-support = testSupportBin;
  };

  internals = {
    inherit
      visualFontConfig
      toolchain
      diagnosticToolchain
      craneLib
      commonArgs
      hostArgs
      wasmTestSrc
      siteSrc
      appOfflineCargoHome
      toolsOfflineCargoHome
      cargoArtifacts
      leanTestProfile
      leanDevAndTestProfile
      jaunderBin
      diagnosticJaunderBin
      diagnosticBaselineJaunderBin
      diagnosticCsrWasmBundle
      diagnosticBaselineCsrWasmBundle
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
