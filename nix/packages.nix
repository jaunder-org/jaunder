{
  system,
  pkgs,
  fenix,
  crane,
  atom-fork,
  orgize-fork,
}:
let
  # One explicit screenshot font universe for host baseline generation and
  # NixOS-VM comparison. The file embeds the DejaVu store path, so the font
  # derivation stays in both closures without ambient system-font lookup.
  visualFontConfig = pkgs.makeFontsConf {
    fontDirectories = [ pkgs.dejavu_fonts ];
  };

  # The browser and font closure used by the canonical repository-thumbnail
  # command. Consumers inherit this one environment instead of pinning a
  # browser independently in CI or a workflow.
  themeThumbnailEnvironment = pkgs.buildEnv {
    name = "jaunder-theme-thumbnail-environment";
    paths = [
      pkgs.dejavu_fonts
      pkgs.fontconfig
    ]
    ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.chromium ];
  };
  toolchain = fenix.packages.${system}.fromToolchainFile {
    file = ../rust-toolchain.toml;
    sha256 = "sha256-A1abGIbOtcBSdrUMhDGrER3pRM1hQP4fp9gh3Y4PKc8=";
  };

  craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
  # wasm-bindgen #5268 identifies nightly-2026-08-05 as the last LLVM 22
  # nightly, so this dated pin stays in that compatible range. It must
  # never track Fenix `latest`: LLVM coverage data, minicov's C runtime, and
  # the manual Clang link are one version-sensitive unit.
  diagnosticNightlyName = "nightly-2026-08-05";
  diagnosticNightlySha256 = "sha256-41hI6kTIBH4+SiyMb3wf8Um6q0930AeV2KIksoT8aLU=";
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
  # The diagnostic producer/runtime input is fetched separately from the
  # production CSR pipeline; minicov may also occur in the root lock for tests.
  # 0.3.9 bundles LLVM 23's incompatible profile record/header layout.
  # 0.3.8 matches LLVM 22 (raw v10); a matching Clang major alone is insufficient.
  diagnosticMinicov = pkgs.fetchCrate {
    pname = "minicov";
    version = "0.3.8";
    hash = "sha256-MAyEaF1M9mPr0rQRjG21XJaWgxwvOWZrObjCeFidgoA=";
  };
  # `llvm-tools-preview` installs profile tools below rustlib rather than the
  # cargo/rustc bin directory.  Keep this path explicit so every diagnostic
  # identity and exported analyzer symlink names the compiler-matched tools.
  diagnosticLlvmTools = "${diagnosticToolchain}/lib/rustlib/${pkgs.stdenv.hostPlatform.config}/bin";
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
    "ux-sandbox"
    "web"
  ];
  sqlxSqlitePatchRoot = "vendor/sqlx-sqlite-0.9.0";
  # This temporary local patch is the complete normalized manifest, unchanged
  # Rust source tree, and upstream licenses only (issue #1928).
  sqlxSqlitePatchSource =
    path:
    let
      absolute = toString path;
      relative =
        if pkgs.lib.hasPrefix "${toString ../.}/" absolute then
          pkgs.lib.removePrefix "${toString ../.}/" absolute
        else if pkgs.lib.hasPrefix "${toString (craneLib.path ../.)}/" absolute then
          pkgs.lib.removePrefix "${toString (craneLib.path ../.)}/" absolute
        else
          absolute;
    in
    relative == "vendor/LICENSE-MIT"
    || pkgs.lib.hasSuffix "/vendor/LICENSE-MIT" absolute
    || relative == "vendor/LICENSE-APACHE"
    || pkgs.lib.hasSuffix "/vendor/LICENSE-APACHE" absolute
    || relative == "${sqlxSqlitePatchRoot}/Cargo.toml"
    || pkgs.lib.hasSuffix "/${sqlxSqlitePatchRoot}/Cargo.toml" absolute
    || pkgs.lib.hasPrefix "${sqlxSqlitePatchRoot}/src/" relative
    || pkgs.lib.hasInfix "/${sqlxSqlitePatchRoot}/src/" absolute
    || pkgs.lib.hasPrefix "${sqlxSqlitePatchRoot}/src/" absolute
    || relative == "${sqlxSqlitePatchRoot}/LICENSE-MIT"
    || pkgs.lib.hasSuffix "/${sqlxSqlitePatchRoot}/LICENSE-MIT" absolute
    || relative == "${sqlxSqlitePatchRoot}/LICENSE-APACHE"
    || pkgs.lib.hasSuffix "/${sqlxSqlitePatchRoot}/LICENSE-APACHE" absolute;
  # Registry macros consume this path dependency's real API during dependency
  # preparation. Keep only the approved driver real; Crane still stubs members.
  sqlxSqliteExtraDummyScript = ''
    chmod -R u+w "$out/${sqlxSqlitePatchRoot}"
    cp -r --no-preserve=mode ${../vendor/sqlx-sqlite-0.9.0}/. "$out/${sqlxSqlitePatchRoot}/"
    cp ${../vendor/LICENSE-MIT} ${../vendor/LICENSE-APACHE} "$out/vendor/"
  '';
  cargoWorkspaceInput =
    path:
    let
      relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
    in
    sqlxSqlitePatchSource path
    || relative == "Cargo.toml"
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
  # Complete Cargo inputs for selected members. Unlike `cargoTargetSource`, this
  # retains every Cargo-recognized target below each member (tests, examples,
  # benches, and custom targets) without admitting another workspace's files.
  cargoMemberSource =
    members: path: type:
    let
      relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
    in
    type == "directory"
    || cargoTargetSource members path type
    || builtins.any (
      member:
      (relative == member || pkgs.lib.hasPrefix "${member}/" relative)
      && craneLib.filterCargoSources path type
    ) members;
  # Resolve the Cargo `path` package closure at evaluation time. Every nested
  # dependency table is searched, including target-specific build/dev tables;
  # normalizing through the repository-relative member root makes an escaping
  # path a Nix evaluation error rather than an untracked source admission.
  normalizeRelativePath =
    path:
    let
      step =
        parts: part:
        if part == "" || part == "." then
          parts
        else if part == ".." then
          if parts == [ ] then
            throw "Cargo path dependency escapes repository: ${path}"
          else
            pkgs.lib.init parts
        else
          parts ++ [ part ];
    in
    pkgs.lib.concatStringsSep "/" (pkgs.lib.foldl' step [ ] (pkgs.lib.splitString "/" path));
  cargoDependencyPaths =
    manifest:
    let
      pathsInDependencyTable =
        value:
        if builtins.isAttrs value then
          (pkgs.lib.optional (value ? path && builtins.isString value.path) value.path)
          ++ pkgs.lib.concatMap pathsInDependencyTable (builtins.attrValues value)
        else if builtins.isList value then
          pkgs.lib.concatMap pathsInDependencyTable value
        else
          [ ];
      visit =
        value:
        if builtins.isAttrs value then
          pkgs.lib.concatMap (
            name:
            let
              child = builtins.getAttr name value;
            in
            if name == "dependencies" || pkgs.lib.hasSuffix "-dependencies" name then
              pathsInDependencyTable child
            else
              visit child
          ) (builtins.attrNames value)
        else if builtins.isList value then
          pkgs.lib.concatMap visit value
        else
          [ ];
    in
    visit manifest;
  cargoPackageClosure =
    roots:
    let
      packageRoot =
        member: dependency:
        if pkgs.lib.hasPrefix "/" dependency then
          throw "Cargo path dependency must be repository-relative: ${dependency}"
        else
          normalizeRelativePath "${member}/${dependency}";
      visit =
        seen: pending:
        if pending == [ ] then
          seen
        else
          let
            member = builtins.head pending;
            remaining = builtins.tail pending;
            manifestPath = "${toString ../.}/${member}/Cargo.toml";
          in
          if builtins.elem member seen then
            visit seen remaining
          else if !builtins.pathExists manifestPath then
            throw "Cargo path dependency manifest is missing: ${member}/Cargo.toml"
          else
            let
              manifest = builtins.fromTOML (builtins.readFile manifestPath);
              dependencies = map (packageRoot member) (cargoDependencyPaths manifest);
            in
            visit (seen ++ [ member ]) (remaining ++ dependencies);
    in
    visit [ ] roots;
  workspacePlaceholderTargets =
    member:
    if member == "server" then
      [
        "src/lib.rs"
        "src/main.rs"
        "tests/main.rs"
      ]
    else if member == "ux-sandbox" then
      [ "src/main.rs" ]
    else if member == "test-support" then
      [
        "src/lib.rs"
        "src/main.rs"
        "src/bin/sqlx-query-diagnostics.rs"
      ]
    else
      [ "src/lib.rs" ];
  withWorkspacePlaceholders =
    name: source: excludedMembers:
    pkgs.runCommand name { } ''
      cp --no-preserve=mode -r ${source}/. "$out/"
      ${pkgs.lib.concatMapStringsSep "\n" (
        member:
        pkgs.lib.concatMapStringsSep "\n" (target: ''
          mkdir -p "$out/${member}/$(dirname ${target})"
          printf '%s\n' '// target-closure placeholder; excluded source remains absent.' > "$out/${member}/${target}"
        '') (workspacePlaceholderTargets member)
      ) excludedMembers}
    '';
  siteCargoMembers = [
    "csr"
    "web"
    "client"
    "common"
    "macros"
    "tools/performance"
  ];
  wasmTestCargoMembers = [
    "client"
    "common"
    "macros"
    "tools/performance"
  ];
  siteSourceFilter =
    path: type:
    sqlxSqlitePatchSource path
    || cargoTargetSource siteCargoMembers path type
    || pkgs.lib.hasSuffix "csr/index.html" path;
  # Minimal source closure for the Vite CSR producer. Cargo source remains a
  # separate derivation input; Vite receives only the HTML/entry/config/pins.
  csrViteSourceFilter =
    path: type:
    let
      relative = pkgs.lib.removePrefix "${toString (craneLib.path ../.)}/" (toString path);
    in
    (
      type == "directory"
      && builtins.elem relative [
        ""
        "csr"
        "scripts"
      ]
    )
    || builtins.elem relative [
      "package.json"
      "package-lock.json"
      "vite.config.js"
      "csr/index.html"
      "csr/entry.js"
      "scripts/build-csr-assets.sh"
    ];
  csrViteSrc = pkgs.lib.cleanSourceWith {
    src = craneLib.path ../.;
    filter = csrViteSourceFilter;
  };
  # Diagnostic and baseline producers run in runCommand derivations, but consume
  # the exact same locked Vite/compression tool graph as the production bundle.
  csrViteNodeModules = pkgs.buildNpmPackage {
    pname = "jaunder-csr-vite-node-modules";
    version = "0.1.0";
    src = csrViteSrc;
    npmDepsHash = "sha256-iq1ioeuLXIqCugh1mDvDsdjZCvwoJx87Cfp+rDVdoPU=";
    dontNpmBuild = true;
    installPhase = ''
      runHook preInstall
      mkdir -p "$out/node_modules"
      cp -a node_modules/. "$out/node_modules/"
      runHook postInstall
    '';
  };
  siteSrc =
    assert siteSourceFilter "${toString (craneLib.path ../.)}/vendor" "directory";
    assert siteSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}" "directory";
    assert siteSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/src/connection"
      "directory";
    assert siteSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/Cargo.toml"
      "regular";
    assert siteSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/src/database.rs"
      "regular";
    assert siteSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/LICENSE-MIT"
      "regular";
    assert siteSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/LICENSE-APACHE"
      "regular";
    assert builtins.all
      (
        name:
        !(siteSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/${name}" "regular")
      )
      [
        "Cargo.toml.orig"
        "Cargo.lock"
        ".cargo-ok"
        ".cargo_vcs_info.json"
        "excluded.rs"
        "excluded.sql"
        "excluded.css"
      ];
    assert siteSourceFilter "${toString (craneLib.path ../.)}/vendor/LICENSE-MIT" "regular";
    assert siteSourceFilter "${toString (craneLib.path ../.)}/vendor/LICENSE-APACHE" "regular";
    withWorkspacePlaceholders "jaunder-site-cargo-source"
      (pkgs.lib.cleanSourceWith {
        src = craneLib.path ../.;
        filter = siteSourceFilter;
      })
      (builtins.filter (member: !(builtins.elem member siteCargoMembers)) workspaceMembers);
  wasmTestSrc =
    withWorkspacePlaceholders "jaunder-wasm-test-cargo-source"
      (pkgs.lib.cleanSourceWith {
        src = craneLib.path ../.;
        filter =
          path: type:
          let
            relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
          in
          cargoTargetSource wasmTestCargoMembers path type || pkgs.lib.hasPrefix "client/tests/" relative;
      })
      (builtins.filter (member: !(builtins.elem member wasmTestCargoMembers)) workspaceMembers);

  productSourceFilter =
    path: type:
    # Do not let generic Cargo/data filters admit archive metadata beneath
    # the local patch. Only its approved files and traversal belong here.
    if
      pkgs.lib.hasSuffix "/${sqlxSqlitePatchRoot}" path
      || pkgs.lib.hasInfix "/${sqlxSqlitePatchRoot}/" path
    then
      sqlxSqlitePatchSource path
      || (
        type == "directory"
        && (
          pkgs.lib.hasSuffix "/${sqlxSqlitePatchRoot}" path
          || pkgs.lib.hasSuffix "/${sqlxSqlitePatchRoot}/src" path
        )
      )
    else
      # Nix assembly is not application source; exclude only its top-level root.
      !(type == "directory" && path == "${toString (craneLib.path ../.)}/nix")
      # Cargo build scripts treat a present `.git` as authoritative metadata.
      # Never retain the filtered empty directory that crane would otherwise admit.
      && !(pkgs.lib.hasSuffix "/.git" path || pkgs.lib.hasInfix "/.git/" path)
      # Repository policy is native tooling; retain its manifest, not implementation.
      && !(pkgs.lib.hasPrefix "${toString (craneLib.path ../.)}/tools/repository-policy/" path
        && path != "${toString (craneLib.path ../.)}/tools/repository-policy/Cargo.toml")
      # Qualification is host-run tooling; retain its manifest, not implementation.
      && !(pkgs.lib.hasPrefix "${toString (craneLib.path ../.)}/tools/qualification/" path
        && path != "${toString (craneLib.path ../.)}/tools/qualification/Cargo.toml")
      && !(pkgs.lib.hasPrefix "${toString (craneLib.path ../.)}/tools/host-proof/" path
        && path != "${toString (craneLib.path ../.)}/tools/host-proof/Cargo.toml")
      # Host-run evidence consumers behind nondefault `host` features: the
      # sandboxed library builds never compile them, so they must not invalidate.
      && !(pkgs.lib.hasPrefix "${toString (craneLib.path ../.)}/tools/coverage/src/host/" path)
      && !(pkgs.lib.hasPrefix "${toString (craneLib.path ../.)}/tools/coverage/tests/" path)
      && path != "${toString (craneLib.path ../.)}/tools/doctests/src/host.rs"
      && !(pkgs.lib.hasPrefix "${toString (craneLib.path ../.)}/tools/doctests/tests/" path)
      && (
        (pkgs.lib.hasSuffix ".sql" path)
        || (pkgs.lib.hasSuffix ".css" path)
        # Shipped Theme Package manifests are host-producer inputs, not generic
        # data files. Keep this closed path in the server/Nix source closure so
        # the same producer validates the exact release inputs in both builds.
        || (pkgs.lib.hasInfix "/host/system_theme_sources/" path)
        # The CSR SPA shell the server embeds via include_str! (#239). Specific
        # (not a broad .html suffix) to keep stray HTML out of the crane src.
        || (pkgs.lib.hasSuffix "csr/index.html" path)
        || (builtins.match "scripts/.*" path != null)
        || (
          let
            relative = pkgs.lib.removePrefix "${toString (craneLib.path ../.)}/" (toString path);
          in
          (
            type == "directory"
            && (relative == "vendor" || pkgs.lib.hasPrefix "${sqlxSqlitePatchRoot}/" relative)
          )
          || sqlxSqlitePatchSource path
          || (craneLib.filterCargoSources path type)
        )
      );
  src =
    assert builtins.all
      (
        name:
        !(productSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/${name}" "regular")
      )
      [
        "Cargo.toml.orig"
        "Cargo.lock"
        ".cargo-ok"
        ".cargo_vcs_info.json"
        "excluded.rs"
        "excluded.sql"
        "excluded.css"
      ];
    pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      filter = productSourceFilter;
    };

  # Supply each exact-revision Cargo git patch from its pinned flake checkout
  # for the product and tools workspaces without sandbox network access.
  vendorPinnedFork =
    prefix: ps: drv:
    let
      p = builtins.head ps;
    in
    if p.name == "atom_syndication" || p.name == "orgize" then
      pkgs.runCommandLocal "${prefix}-fork-vendor-${p.name}-${p.version}" { } ''
        dst="$out/${p.name}-${p.version}"
        mkdir -p "$dst"
        cp -a ${if p.name == "orgize" then orgize-fork else atom-fork}/. "$dst/"
        chmod -R u+w "$dst"
        echo '{"files":{},"package":null}' > "$dst/.cargo-checksum.json"
      ''
    else
      drv;

  mkHostTool = import ./host-tool.nix {
    inherit
      pkgs
      craneLib
      cargoMemberSource
      cargoPackageClosure
      workspaceMembers
      withWorkspacePlaceholders
      vendorPinnedFork
      sqlxSqlitePatchSource
      ;
  };
  qualificationHostBin = import ./qualification.nix { inherit mkHostTool; };
  # The host-proof executables CI runs from a cached output instead of
  # compiling the tools workspace: the e2e verifier and mutants' pg-run.
  hostProofBin = mkHostTool {
    pname = "host-proof";
    member = "tools/host-proof";
    bins = [ "e2e-evidence" "pg-run" ];
  };

  repositoryPolicyBin = import ./repository-policy.nix {
    inherit pkgs craneLib cargoMemberSource cargoPackageClosure workspaceMembers withWorkspacePlaceholders vendorPinnedFork;
  };
  cargoVendorDir = craneLib.vendorCargoDeps {
    inherit src;
    overrideVendorGitCheckout = vendorPinnedFork "product";
  };

  commonArgs = {
    inherit src cargoVendorDir;
    extraDummyScript = sqlxSqliteExtraDummyScript;
    meta.license = pkgs.lib.licenses.gpl3Plus;
    pname = "jaunder";
    version = "0.1.0";
    strictDeps = true;
    RUST_MIN_STACK = "16777216";
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [
      pkgs.openssl
      pkgs.sqlite
    ];
  };

  # Darwin rustc invokes generic lld for wasm; both sandboxed CSR builds and
  # dependency preparation need it, independent of the development shell.
  csrArgs = commonArgs // {
    nativeBuildInputs =
      commonArgs.nativeBuildInputs ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.lld ];
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
  debugHostArgs = hostArgs // {
    CARGO_PROFILE = "dev";
  };
  debugCargoArtifacts = craneLib.buildDepsOnly debugHostArgs;

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

  mkJaunderBin =
    {
      pname ? "jaunder",
      debug ? false,
      csrBundle ? csrWasmBundle,
      passthru ? { },
    }:
    craneLib.buildPackage (
      (if debug then debugHostArgs else hostArgs)
      // {
        inherit pname passthru;
        cargoArtifacts = if debug then debugCargoArtifacts else cargoArtifacts;
        # RustEmbed otherwise reads the vanished build directory in dev builds.
        cargoExtraArgs = "-p jaunder" + pkgs.lib.optionalString debug " --features rust-embed/debug-embed";
        # Embed the CSR bundle + public assets into the binary (#237): the
        # release artifact is self-contained (ADR-0003/0008), serving pkg/*
        # and public/* with no external files. `server/build.rs` stages these
        # into the embed; the env vars are its inputs (the crane `src` filter
        # admits neither the bundle nor public/, so they arrive via env). This
        # is the build-order edge that makes the binary depend on the bundle.
        JAUNDER_CSR_BUNDLE_DIR = "${csrBundle}";
        JAUNDER_PUBLIC_DIR = "${../public}";
        # Tests are covered by the separate `coverage` check (which runs the
        # instrumented nextest suite) and, for doctests — which nextest
        # structurally cannot run — the separate `doctests` check. Disabling
        # here avoids a redundant `cargo test` compile + run during the
        # package build.
        doCheck = false;
        nativeBuildInputs =
          hostArgs.nativeBuildInputs ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.patchelf ];
        postFixup = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
          patchelf --add-rpath \
            "${
              pkgs.lib.makeLibraryPath [
                pkgs.openssl
                pkgs.dav1d
              ]
            }" \
            "$out/bin/jaunder"
        '';
      }
      // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
        # The canonical inventory loader links host AVIF decoding into the
        # server build script. It runs before output postFixup can add rpaths.
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.dav1d ];
      }
    );
  jaunderBin = mkJaunderBin { };

  # The out-of-process e2e seed helper (ADR-0046). Built as its own small
  # crane package (no leptos/wasm/web deps; shares cargoArtifacts) and placed
  # ONLY on the e2e VM PATH — deliberately absent from the `jaunder` prod
  # binary and the `services.jaunder` NixOS module, so there is no seed
  # surface anywhere near the release artifact.
  mkTestSupportBin =
    {
      debug ? false,
    }:
    craneLib.buildPackage (
      (if debug then debugHostArgs else hostArgs)
      // {
        cargoArtifacts = if debug then debugCargoArtifacts else cargoArtifacts;
        pname = "test-support";
        cargoExtraArgs = "-p test-support";
        doCheck = false;
        nativeBuildInputs =
          hostArgs.nativeBuildInputs ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.patchelf ];
        postFixup = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
          patchelf --add-rpath \
            "${
              pkgs.lib.makeLibraryPath [
                pkgs.openssl
                pkgs.dav1d
              ]
            }" \
            "$out/bin/test-support"
        '';
      }
    );

  testSupportBin = mkTestSupportBin { };

  # One rooted selection retains both executables and the server's embedded CSR.
  # Consumers admit untracked inputs with sourceAdmits before nix build, use only
  # the successful request's output, and hold their unique --out-link until exit.
  mkSandboxArtifacts =
    csrBundle:
    let
      jaunder = mkJaunderBin {
        pname = "jaunder-debug";
        debug = true;
        inherit csrBundle;
      };
      testSupport = mkTestSupportBin { debug = true; };
    in
    (pkgs.linkFarm "jaunder-sandbox-artifacts" [
      {
        name = "bin/jaunder";
        path = "${jaunder}/bin/jaunder";
      }
      {
        name = "bin/test-support";
        path = "${testSupport}/bin/test-support";
      }
    ]).overrideAttrs
      (_: {
        passthru = {
          inherit jaunder testSupport csrBundle;
          sourceAdmits =
            relative: type:
            productSourceFilter "${toString ../.}/${relative}" type
            || csrSourceAdmits relative type
            || relative == "public"
            || pkgs.lib.hasPrefix "public/" relative;
        };
      });

  # Keep the Python host package independent of Cargo and application sources.
  # Tests are admitted with runtime sources so build-time pytest is fully offline.
  adrPromoterSourceAdmits = import ./adr-promoter-source.nix;
  adrPromoterSrc = pkgs.lib.cleanSourceWith {
    src = ../tools/adr-promoter;
    filter =
      path: type:
      let
        relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
      in
      adrPromoterSourceAdmits relative type;
  };
  adrPromoter = pkgs.python3Packages.buildPythonApplication {
    pname = "adr-promoter";
    version = "0.1.0";
    pyproject = true;
    src = adrPromoterSrc;
    build-system = [ pkgs.python3Packages.setuptools ];
    dependencies = [
      pkgs.python3Packages.marko
      pkgs.python3Packages.pygithub
    ];
    makeWrapperArgs = [
      "--prefix PATH : ${
        pkgs.lib.makeBinPath [
          pkgs.git
          pkgs.prettier
        ]
      }"
    ];
    nativeCheckInputs = [
      pkgs.python3Packages.pytestCheckHook
      pkgs.git
    ];
    doCheck = true;
    pythonImportsCheck = [ "adr_promoter" ];
    passthru.sourceAdmits = adrPromoterSourceAdmits;
    meta.mainProgram = "adr-promoter";
  };

  # The auxiliary tools workspace is separate from the product workspace
  # (ADR-0141): its lockfile and cargo artifacts stay independent, while the
  # performance producer links the selected typed product-storage closure.
  # Share the producer's existing filter with live-checkout admission; do not
  # duplicate or narrow its broad tooling closure to manufacture cache hits.
  toolsSourceFilter =
    path: type:
    let
      relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
    in
    type == "directory"
    || relative == "tools/Cargo.lock"
    || relative == "csr/index.html"
    # Host system-artifact production needs both Theme Package inputs and
    # trusted application CSS, not just Cargo-recognized host sources.
    || relative == "server/assets/jaunder.css"
    || pkgs.lib.hasPrefix "host/system_theme_sources/" relative
    || pkgs.lib.hasPrefix "storage/migrations/" relative
    || sqlxSqlitePatchSource path
    || cargoMemberSource [ "common" "host" "macros" "storage" ] path type
    || pkgs.lib.hasPrefix "tools/doctests/testdata/" relative
    || (
      pkgs.lib.hasPrefix "tools/" relative
      && !(pkgs.lib.hasPrefix "tools/repository-policy/" relative && relative != "tools/repository-policy/Cargo.toml")
      && !(pkgs.lib.hasPrefix "tools/qualification/" relative && relative != "tools/qualification/Cargo.toml")
      && !(pkgs.lib.hasPrefix "tools/host-proof/" relative && relative != "tools/host-proof/Cargo.toml")
      && !(pkgs.lib.hasPrefix "tools/coverage/src/host/" relative)
      && !(pkgs.lib.hasPrefix "tools/coverage/tests/" relative)
      && relative != "tools/doctests/src/host.rs"
      && !(pkgs.lib.hasPrefix "tools/doctests/tests/" relative)
      && craneLib.filterCargoSources path type
    );
  csrSourceAdmits =
    relative: type:
    let
      path = "${toString ../.}/${relative}";
    in
    siteSourceFilter path type
    || toolsSourceFilter path type
    || csrViteSourceFilter path type
    # Nix expressions and the lock select producer/toolchain/native dependencies,
    # even when they are not copied into either Cargo source tree.
    || relative == "Justfile"
    || relative == "flake.nix"
    || relative == "flake.lock"
    || pkgs.lib.hasPrefix "nix/" relative;
  toolsSrc =
    assert toolsSourceFilter "${toString (craneLib.path ../.)}/vendor" "directory";
    assert toolsSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}" "directory";
    assert toolsSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/src/connection"
      "directory";
    assert toolsSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/Cargo.toml"
      "regular";
    assert toolsSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/src/database.rs"
      "regular";
    assert toolsSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/LICENSE-MIT"
      "regular";
    assert toolsSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/LICENSE-APACHE"
      "regular";
    assert builtins.all
      (
        name:
        !(toolsSourceFilter "${toString (craneLib.path ../.)}/${sqlxSqlitePatchRoot}/${name}" "regular")
      )
      [
        "Cargo.toml.orig"
        "Cargo.lock"
        ".cargo-ok"
        ".cargo_vcs_info.json"
        "excluded.rs"
        "excluded.sql"
        "excluded.css"
      ];
    assert toolsSourceFilter "${toString (craneLib.path ../.)}/vendor/LICENSE-MIT" "regular";
    assert toolsSourceFilter "${toString (craneLib.path ../.)}/vendor/LICENSE-APACHE" "regular";
    withWorkspacePlaceholders "jaunder-tools-cargo-source"
      (pkgs.lib.cleanSourceWith {
        src = craneLib.path ../.;
        filter = toolsSourceFilter;
      })
      [
        "client"
        "csr"
        "server"
        "test-support"
        "web"
        "tools/repository-policy"
        "tools/qualification"
        "tools/host-proof"
      ];
  toolsBaseArgs = {
    src = toolsSrc;
    extraDummyScript = sqlxSqliteExtraDummyScript;
    pname = "jaunder-tools";
    version = "0.1.0";
    meta.license = pkgs.lib.licenses.gpl3Plus;
    strictDeps = true;
    cargoExtraArgs = "--manifest-path tools/Cargo.toml";
    cargoLock = ../tools/Cargo.lock;
    postPatch = ''
      ln -sf ../Cargo.lock tools/Cargo.lock
    '';
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [
      pkgs.openssl
      pkgs.sqlite
      pkgs.dav1d
    ];
  };
  toolsVendorArgs = {
    overrideVendorGitCheckout = vendorPinnedFork "tools";
  };
  tracedRunPlaceholderMembers =
    workspaceMembers
    ++ map (member: "tools/${member}") (
      builtins.filter (member: member != "traced-run")
        (builtins.fromTOML (builtins.readFile ../tools/Cargo.toml)).workspace.members
    );
  tracedRunSourceFilter =
    path: type:
    let
      relative = pkgs.lib.removePrefix "${toString ../.}/" (toString path);
    in
    # Traversal must admit ancestors as well as member roots. Directories add
    # no unrelated source bytes; manifests resolve the workspace, while excluded
    # targets are supplied by the same placeholder convention as product builds.
    type == "directory"
    || cargoWorkspaceInput path
    || relative == "tools/Cargo.toml"
    || relative == "tools/Cargo.lock"
    || builtins.any (member: relative == "${member}/Cargo.toml") tracedRunPlaceholderMembers
    || (pkgs.lib.hasPrefix "tools/traced-run/" relative && craneLib.filterCargoSources path type);
  tracedRunSrc = withWorkspacePlaceholders "jaunder-traced-run-cargo-source" (pkgs.lib.cleanSourceWith
    {
      src = craneLib.path ../.;
      filter = tracedRunSourceFilter;
    }
  ) tracedRunPlaceholderMembers;
  tracedRunBaseArgs = toolsBaseArgs // {
    src = tracedRunSrc;
    pname = "jaunder-traced-run";
    cargoExtraArgs = "--manifest-path tools/Cargo.toml -p traced-run";
  };
  tracedRunCargoVendorDir = craneLib.vendorCargoDeps (tracedRunBaseArgs // toolsVendorArgs);
  tracedRunTooling = mkToolingCargo (
    tracedRunBaseArgs // { cargoVendorDir = tracedRunCargoVendorDir; }
  );
  tracedRunBin = tracedRunTooling.buildPackage {
    pname = "traced-run";
    doCheck = false;
    installPhase = ''
      mkdir -p $out/bin
      cp target/release/traced-run $out/bin/traced-run
    '';
    doNotPostBuildInstallCargoBinaries = true;
  };
  toolsCargoVendorDir = craneLib.vendorCargoDeps (toolsBaseArgs // toolsVendorArgs);
  toolsArgs = toolsBaseArgs // {
    cargoVendorDir = toolsCargoVendorDir;
  };
  mkToolingCargo = import ./tooling-cargo.nix { inherit craneLib; };
  # Generated dependency sources and the file-scoped lock keep implementation edits
  # out of preparation, while every final consumer retains its complete real source.
  toolsBuild = mkToolingCargo toolsArgs;
  toolsOfflineCargoHome = mkOfflineCargoHome {
    name = "jaunder-tools";
    vendorDir = toolsCargoVendorDir;
  };

  # The in-sandbox dev tool. Offline coverage/doctest sandboxes run it from
  # PATH instead of an in-sandbox `cargo run`, whose dependencies would not be
  # vendored. The CSR shell is retained directly in toolsSrc for include_str!.
  mkDevtoolBin =
    {
      pname ? "devtool",
      qualification ? false,
    }:
    let
      cargoExtraArgs =
        "${toolsArgs.cargoExtraArgs} -p devtool"
        + pkgs.lib.optionalString qualification " --features qualification";
      tooling =
        if qualification then mkToolingCargo (toolsArgs // { inherit cargoExtraArgs; }) else toolsBuild;
    in
    tooling.buildPackage {
      inherit cargoExtraArgs pname;
      doCheck = false;
      passthru.toolingDependencies = tooling.cargoArtifacts;
      installPhase = ''
        mkdir -p $out/bin
        cp target/release/devtool $out/bin/devtool
      '';
      # `installPhase` copies a host-linked executable directly, bypassing
      # Crane's normal cargo binary install path. Give the full producer an
      # ELF runtime closure for its native host dependencies.
      nativeBuildInputs =
        toolsArgs.nativeBuildInputs
        ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.patchelf ];
      postFixup = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
        patchelf --add-rpath \
          "${
            pkgs.lib.makeLibraryPath [
              pkgs.openssl
              pkgs.dav1d
            ]
          }" \
          "$out/bin/devtool"
      '';
      doNotPostBuildInstallCargoBinaries = true;
    };
  devtoolBin = mkDevtoolBin { };
  qualificationDevtoolBin = mkDevtoolBin {
    pname = "jaunder-system-artifact-qualification-devtool";
    qualification = true;
  };

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
        version = "0.5.0";
        hash = "sha256-5RhRFUh1w5/yItkmc3Vk1B6oyrmzKKl6EEZ3v0aBLwk=";
      };
    in
    rustPlatform.buildRustPackage (finalAttrs: {
      pname = "cargo-crap";
      version = "0.5.0";

      src = fetchFromGitHub {
        owner = "minikin";
        repo = "cargo-crap";
        rev = "v${finalAttrs.version}";
        hash = "sha256-90RVGII+CcDBh492b/tJ8wRwl8lPk7bW+02u7q5dG78=";
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
    version = "0.2.128";
    src = pkgs.fetchCrate {
      pname = "wasm-bindgen-cli";
      inherit version;
      hash = "sha256-a7lcXJnnZkYReja+iUO7NqqrWyv3toxnUgQb8s4IS5s=";
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

  # The CSR client's Vite-rendered HTML entry and emitted assets, assembled as
  # a static tree. The build-only .vite manifest and private system inventory
  # stay outside the public site output.
  site = pkgs.runCommand "jaunder-site" { } ''
    mkdir -p $out
    for path in ${csrWasmBundle}/*; do
      case "$(basename "$path")" in system-artifacts) continue ;; esac
      cp -a "$path" $out/
    done
  '';

  # --- leptos-CSR client (#177/#180) --------------------------------------
  # The client-side-render wasm binary — the only client (#180).
  # `csrWasmBundle` runs wasm-bindgen over it; `site`
  # (above) bundles it with the public assets + rendered CSR shell.
  mkCsrWasm =
    {
      debug ? false,
    }:
    let
      # Leave Crane's existing release defaults untouched; debug dependencies
      # and the final module must use the same Cargo profile.
      profileArgs = pkgs.lib.optionalAttrs debug { CARGO_PROFILE = "dev"; };
      artifactProfile = if debug then "debug" else "release";
    in
    craneLib.buildPackage (
      csrArgs
      // profileArgs
      // {
        src = siteSrc;
        cargoArtifacts = craneLib.buildDepsOnly (
          csrArgs
          // profileArgs
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
          cp target/wasm32-unknown-unknown/${artifactProfile}/csr.wasm $out/lib/
        '';
      }
    );
  csrWasm = mkCsrWasm { };
  csrDebugWasm = mkCsrWasm { debug = true; };

  # Measurement-only direct-init arm label. Empty is committed and preserves
  # the normal e2e derivation hash; set for one #864 measurement arm, then
  # revert before committing.
  wasmExperimentArm = "";

  mkCsrWasmBundle =
    {
      devtool,
      fixture ? null,
      wasm ? csrWasm,
      name ? "jaunder-csr-vite-bundle",
    }:
    pkgs.buildNpmPackage {
      pname = "${name}${pkgs.lib.optionalString (fixture != null) "-${fixture}"}";
      version = "0.1.0";
      src = csrViteSrc;
      npmDepsHash = "sha256-iq1ioeuLXIqCugh1mDvDsdjZCvwoJx87Cfp+rDVdoPU=";
      dontNpmBuild = true;
      passthru.sourceAdmits = csrSourceAdmits;
      nativeBuildInputs = [
        devtool
        pkgs.binaryen
        wasm-bindgen-cli
      ];
      CSR_WASM_INPUT = "${wasm}/lib/csr.wasm";
      CSR_WASM_EXPERIMENT_ARM = wasmExperimentArm;
      buildPhase = ''
        runHook preBuild
        work="$TMPDIR/csr-vite"
        input="$work/input"
        system_artifacts="$work/system-artifacts"
        dist="$work/dist"
        mkdir -p "$input" "$system_artifacts"
        cp csr/index.html "$input/index.html"
        cp csr/entry.js "$input/entry.js"
        devtool system-artifacts --out "$system_artifacts"${
          pkgs.lib.optionalString (fixture != null) " --fixture ${fixture}"
        }
        CSR_VITE_PUBLIC_DIR="${../public}" bash scripts/build-csr-assets.sh \
          "$CSR_WASM_INPUT" "$PWD" "$input" "$system_artifacts" "$dist"
        runHook postBuild
      '';
      installPhase = ''
        runHook preInstall
        mkdir -p "$out"
        cp -a "$TMPDIR/csr-vite/dist/." "$out/"
        runHook postInstall
      '';
    };
  csrWasmBundle = mkCsrWasmBundle { devtool = devtoolBin; };
  csrDebugBundle = mkCsrWasmBundle {
    devtool = devtoolBin;
    wasm = csrDebugWasm;
    name = "jaunder-csr-debug-wasm-bundle";
  };
  mkSystemArtifactQualificationPackage =
    fixture:
    if
      !builtins.elem fixture [
        "a"
        "b-app"
        "b-theme"
      ]
    then
      throw "unknown system-artifact qualification fixture `${fixture}`"
    else
      let
        csrBundle = mkCsrWasmBundle {
          devtool = qualificationDevtoolBin;
          inherit fixture;
        };
      in
      mkJaunderBin {
        pname = "jaunder-system-artifact-qualification-${fixture}";
        inherit csrBundle;
        passthru = {
          inherit csrBundle fixture;
          systemArtifactInventory = "${csrBundle}/system-artifacts";
        };
      };

  # Separately instrumented CSR producer for the Playwright/WASM coverage probe.
  #
  # Cargo is deliberately only an LLVM-IR producer here.  Its ordinary final
  # wasm link cannot supply minicov's profiler archive, and treating that failed
  # link as the diagnostic artifact would lose the exact source-mappable module.
  # The pinned nightly emits every target crate's IR without a root link; LLVM
  # 22 Clang compiles that exact closed graph and links it with minicov's archive.
  # This derivative is not an input to `site` or `jaunderBin`.
  diagnosticCsrWasmBundle =
    pkgs.runCommand "jaunder-diagnostic-csr-wasm-bundle"
      {
        nativeBuildInputs = [
          # Cargo build scripts are host programs and require Nix's ordinary
          # `cc`; the target-specific CC/CFLAGS below still force minicov's C
          # profiler runtime through unwrapped LLVM 22 Clang.
          pkgs.stdenv.cc
          pkgs.nodejs
          diagnosticToolchain
          diagnosticClang
          devtoolBin
          pkgs.binaryen
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
        {"version":5,"nightly":"${diagnosticNightlyName} (rustc 1.99.0-nightly 1ed2df61a; LLVM 22.1.8)","nightly_source_sha256":"${diagnosticNightlySha256}","instrumented_first_party_crates":["csr"],"omitted_first_party_crates":["client","common","macros","web"],"producer":"Cargo emits only CSR LLVM IR and an rlink without a final link; pinned rustc -Zlink-only recreates its complete Cargo/sysroot/minicov link graph.","rustflags":["-Cinstrument-coverage","-Zno-profiler-runtime","--emit=llvm-ir","-C link-arg=--no-gc-sections","-Zno-link"],"linker":"pinned rustc -Zlink-only","c_compiler":"llvmPackages_22.clang-unwrapped","diagnostic_runtime":{"wrapper_crate":"diagnostic-coverage-runtime","crate":"minicov","version":"0.3.8","source_sha256":"4869b6a491569605d66d3952bcdf03df789e5b536e5f0cf7758a7f08a55ae24d"}}
        EOF
        work="$TMPDIR/wasm-coverage-csr"
        vite_project="$work/vite-project"
        mkdir -p "$work" "$vite_project"
        cp -a ${csrViteSrc}/. "$vite_project/"
        chmod -R u+w "$vite_project"
        ln -s ${csrViteNodeModules}/node_modules "$vite_project/node_modules"
        cp -r ${siteSrc}/. "$work/source"
        chmod -R u+w "$work/source"
        devtool diagnostic-build prepare-source \
          --source "$work/source" \
          --minicov "${diagnosticMinicov}" \
          --runtime "${../tools/diagnostic-coverage-runtime}" \
          --source-identity "$out/source-identity.json" \
          --nix-source "${siteSrc}"

        set +e
        (
          set -e
          devtool diagnostic-build validate-llvm \
            --rustc-version "$out/toolchain-rustc-vv.txt" \
            --clang-version "$out/toolchain-clang-version.txt"
          cd "$work/source"
          export CARGO_HOME=${appOfflineCargoHome}
          export CARGO_TARGET_DIR="$work/target"
          export CARGO_BUILD_TARGET=wasm32-unknown-unknown
          export CC_wasm32_unknown_unknown=clang
          export CFLAGS_wasm32_unknown_unknown="--target=wasm32-unknown-unknown"
          cargo rustc -p csr --features diagnostic-coverage --target wasm32-unknown-unknown --release --lib --message-format=json-render-diagnostics -- -Cinstrument-coverage -Zno-profiler-runtime --emit=llvm-ir -C link-arg=--no-gc-sections -Zno-link > "$work/cargo-artifacts.jsonl"
          cp "$work/cargo-artifacts.jsonl" "$out/instrumented/cargo-artifacts.jsonl"
          devtool diagnostic-build discover-root \
            --target-release "$work/target/wasm32-unknown-unknown/release" \
            --root-ir "$work/root-ir" \
            --root-rlink "$work/root-rlink" \
            --manifest "$out/instrumented/ir-manifest.json"
          root_ir="$(cat "$work/root-ir")"
          root_rlink="$(cat "$work/root-rlink")"
          cp "$root_ir" "$out/instrumented/csr.ll"
          cp "$root_rlink" "$out/instrumented/csr.rlink"
          rustc --target wasm32-unknown-unknown -Zlink-only "$out/instrumented/csr.rlink"
          devtool diagnostic-build retain-linked-wasm \
            --target-release "$work/target/wasm32-unknown-unknown/release" \
            --manifest "$out/instrumented/ir-manifest.json" \
            --retained-wasm "$out/instrumented/csr.wasm"
          input="$work/input"
          system_artifacts="$work/system-artifacts"
          dist="$work/dist"
          mkdir -p "$input" "$system_artifacts"
          devtool system-artifacts --out "$system_artifacts"
          CSR_VITE_PUBLIC_DIR="${../public}" bash "$vite_project/scripts/build-csr-assets.sh" \
            "$out/instrumented/csr.wasm" "$vite_project" "$input" "$system_artifacts" "$dist" "$work/wasm-bindgen.wasm"
          mkdir -p "$out/pkg"
          cp -a "$dist/." "$out/pkg/"
          devtool diagnostic-build write-coverage-artifacts \
            --metadata "$out/coverage-metadata.json" \
            --toolchain-identity "$out/toolchain-identity.json" \
            --minicov-version 0.3.8 \
            --input-wasm "$out/instrumented/csr.wasm" \
            --wasm-bindgen-wasm "$work/wasm-bindgen.wasm" \
            --wasm-opt-wasm "$input/csr_bg.wasm"
          devtool diagnostic-build assert-coverage --metadata "$out/coverage-metadata.json"
        ) > "$out/pipeline.log" 2>&1
        pipeline_exit=$?
        set -e

        devtool diagnostic-build write-instrumented-status \
          --status "$out/status.json" \
          --pipeline-exit "$pipeline_exit"
      '';

  # The timing baseline intentionally shares the diagnostic pinned nightly, source
  # closure, wasm-bindgen/wasm-opt bundling, service, and focused browser flow. It
  # differs only by omitting -Cinstrument-coverage, minicov, and the diagnostic
  # exports; those are unavoidable because they are what this experiment measures.
  diagnosticBaselineCsrWasmBundle =
    pkgs.runCommand "jaunder-diagnostic-baseline-csr-wasm-bundle"
      {
        nativeBuildInputs = [
          pkgs.stdenv.cc
          pkgs.nodejs
          diagnosticToolchain
          devtoolBin
          pkgs.binaryen
          wasm-bindgen-cli
        ];
      }
      ''
        export PATH=${diagnosticToolchain}/bin:$PATH
        work="$TMPDIR/wasm-coverage-baseline"
        vite_project="$work/vite-project"
        mkdir -p "$work" "$vite_project"
        cp -a ${csrViteSrc}/. "$vite_project/"
        chmod -R u+w "$vite_project"
        ln -s ${csrViteNodeModules}/node_modules "$vite_project/node_modules"
        cp -r ${siteSrc}/. "$work/source"
        chmod -R u+w "$work/source"
        cd "$work/source"
        export CARGO_HOME=${appOfflineCargoHome}
        export CARGO_TARGET_DIR="$work/target"
        cargo build -p csr --target wasm32-unknown-unknown --release
        input="$work/input"
        system_artifacts="$work/system-artifacts"
        dist="$work/dist"
        mkdir -p "$input" "$system_artifacts"
        devtool system-artifacts --out "$system_artifacts"
        CSR_VITE_PUBLIC_DIR="${../public}" bash "$vite_project/scripts/build-csr-assets.sh" \
          "$work/target/wasm32-unknown-unknown/release/csr.wasm" "$vite_project" "$input" "$system_artifacts" "$dist"
        mkdir -p "$out/pkg"
        cp -a "$dist/." "$out/pkg/"
        devtool diagnostic-build write-baseline-status --status "$out/status.json"
      '';
  diagnosticBaselineJaunderBin = mkJaunderBin {
    pname = "jaunder-diagnostic-wasm-coverage-baseline";
    csrBundle = "${diagnosticBaselineCsrWasmBundle}/pkg";
  };

  # The diagnostic browser probe must serve the same instrumented bundle that
  # it later records.  Keep this derivative separate from the release binary:
  # only the probe VM selects it through an explicit service override.
  diagnosticJaunderBin = mkJaunderBin {
    pname = "jaunder-diagnostic-wasm-coverage";
    csrBundle = "${diagnosticCsrWasmBundle}/pkg";
  };

  nodeModulesSetup = import ./node-modules-setup.nix { inherit pkgs; };

  e2ePackage = pkgs.buildNpmPackage {
    name = "jaunder-e2e";
    src = ../end2end;
    npmDepsHash = "sha256-5CoGIp+bFkuxLiwfdCSIBDaREC99bQnh6sMpmyj7IBc=";
    dontNpmBuild = true;
    installPhase = ''
      mkdir -p $out
      cp -r . $out/
    '';
  };

  # The CI source includes its tests and batch drivers. The public package
  # admits only production Lisp modules, excluding test/, scripts/, and docs.
  emacsSrc = pkgs.lib.cleanSourceWith {
    src = ../elisp;
  };
  jaunderEmacsSrc = pkgs.lib.cleanSourceWith {
    src = ../elisp;
    filter =
      path: type:
      let
        relative = pkgs.lib.removePrefix "${toString ../elisp}/" (toString path);
      in
      path == toString ../elisp
      || (type == "directory" && relative != "test" && relative != "scripts")
      || (type == "regular" && pkgs.lib.hasSuffix ".el" relative);
  };

  # CI and the public package share one Emacs package set, including the pinned
  # cmark-el source that Nixpkgs and MELPA do not package. CI retains its
  # explicit curl PATH for its existing test environment; Nixpkgs's plz package
  # resolves curl to an immutable store path for the exported Jaunder package.
  emacsPackages = pkgs.emacs.pkgs;
  cmarkEl = emacsPackages.trivialBuild {
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
  jaunderEmacsPackage = emacsPackages.trivialBuild {
    pname = "jaunder";
    version = "0.1.0";
    src = jaunderEmacsSrc;
    packageRequires = [
      emacsPackages.plz
      cmarkEl
    ];
  };
  emacsForCi = emacsPackages.withPackages (epkgs: [
    epkgs.plz
    epkgs.undercover
    cmarkEl
  ]);
in
{
  packages = {
    jaunder = jaunderBin;
    site = site;
    # The pre-wasm-bindgen, unstripped wasm. Exposed so
    # `just wasm-audit report --breakdown` has an artifact that still
    # carries a name section: `wasm-opt` strips names from the shipped
    # bundle, so the shipped file cannot be attributed to crates (#836).
    inherit csrWasm;
    csrBundle = csrWasmBundle;
    csr-vite-node-modules = csrViteNodeModules;
    inherit csrDebugBundle;
    sandboxArtifacts = mkSandboxArtifacts csrDebugBundle;
    sandboxArtifactsReleaseCsr = mkSandboxArtifacts csrWasmBundle;
    traced-run = tracedRunBin;
    devtool = devtoolBin;
    node-modules-setup = nodeModulesSetup;
    ux-sandbox = import ./ux-sandbox.nix {
      inherit
        pkgs
        craneLib
        cargoPackageClosure
        cargoMemberSource
        workspaceMembers
        withWorkspacePlaceholders
        vendorPinnedFork
        sqlxSqlitePatchRoot
        sqlxSqlitePatchSource
        ;
    };
    repository-policy = repositoryPolicyBin;
    qualification-host = qualificationHostBin;
    host-proof = hostProofBin;
    adr-promoter = adrPromoter;
    # The out-of-process e2e seed helper (ADR-0046). Exposed so it is
    # directly buildable/verifiable; it is placed only on the e2e VM PATH,
    # never in the prod artifact or the NixOS module.
    test-support = testSupportBin;
  }
  // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
    qualification-harness = import ./qualification-harness.nix {
      inherit pkgs;
      qualificationHost = qualificationHostBin;
      testSupport = testSupportBin;
      browser = e2ePackage;
    };
    qualification-harness-legacy-1419 = import ./qualification-harness.nix {
      inherit pkgs;
      name = "jaunder-qualification-harness-legacy-1419";
      qualificationHost = qualificationHostBin;
      # Keep the accepted fixture's complete historical dependency graph.
      testSupport =
        (builtins.getFlake "github:jaunder-org/jaunder/78d201e6969ed17b8a54b87d3de24be6afa1f5ee")
        .packages.${pkgs.stdenv.hostPlatform.system}.test-support;
      browser = e2ePackage;
    };
    theme-thumbnail-environment = themeThumbnailEnvironment;
    # The manually linked diagnostic module and its retained IR/link evidence.
    # A failed producer still exposes its durable `status.json` and `pipeline.log`
    # for the later browser producer; no independent blocker derivation masks it.
    wasm-coverage-csr = diagnosticCsrWasmBundle;
  };

  emacsPackages = {
    jaunder = jaunderEmacsPackage;
  };

  internals = {
    inherit
      visualFontConfig
      themeThumbnailEnvironment
      toolchain
      diagnosticToolchain
      craneLib
      commonArgs
      hostArgs
      wasmTestSrc
      siteSrc
      siteCargoMembers
      wasmTestCargoMembers
      workspaceMembers
      cargoTargetSource
      cargoMemberSource
      cargoPackageClosure
      withWorkspacePlaceholders
      sqlxSqlitePatchRoot
      sqlxSqlitePatchSource
      appOfflineCargoHome
      toolsOfflineCargoHome
      cargoArtifacts
      leanTestProfile
      leanDevAndTestProfile
      jaunderBin
      mkSystemArtifactQualificationPackage
      diagnosticJaunderBin
      diagnosticBaselineJaunderBin
      diagnosticCsrWasmBundle
      diagnosticBaselineCsrWasmBundle
      testSupportBin
      devtoolBin
      tracedRunBin
      cargo-crap
      wasm-bindgen-cli
      wasmTestWebdriverConfig
      leptosfmt
      csrWasmBundle
      e2ePackage
      nodeModulesSetup
      emacsSrc
      emacsForCi
      ;
  };
}
