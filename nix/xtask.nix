# Build the host orchestrator independently; never execute it in a derivation.
{
  pkgs,
  craneLib,
  toolchain,
  cargoPackageClosure,
  cargoMemberSource,
  workspaceMembers,
  withWorkspacePlaceholders,
  vendorPinnedFork,
}:
let
  root = toString ../.;
  members = cargoPackageClosure [ "xtask" ];
  toolsManifest = builtins.fromTOML (builtins.readFile ../tools/Cargo.toml);
  toolsMembers = map (member: "tools/${member}") toolsManifest.workspace.members;
  manifestMembers = workspaceMembers ++ toolsMembers;
  # Expose this same admission predicate to checkout launchers. Nix remains the
  # authority for source selection, including files absent from Git snapshots.
  sourceAdmits = relative: type:
    if type == "directory" then
      relative == ""
      || builtins.any (member:
        relative == member || pkgs.lib.hasPrefix "${member}/" relative
        || pkgs.lib.hasPrefix "${relative}/" member
      ) (members ++ [ "host/system_theme_sources" ])
      || builtins.any (directory:
        relative == directory || pkgs.lib.hasPrefix "${relative}/" directory
      ) (manifestMembers ++ [ ".cargo" "csr" "server/assets" "docs" "end2end" "nix" ".github" ".github/workflows" "test-support/src" ])
    else cargoMemberSource members "${root}/${relative}" type
    || relative == "xtask/Cargo.lock"
    # The all-target recipe regression embeds the checkout's Justfile.
    || relative == "Justfile"
    || relative == ".gitignore"
    || relative == "test-support/src/lib.rs"
    || relative == "rust-toolchain.toml"
    || relative == ".clippy.toml"
    || relative == "clippy.toml"
    || relative == ".github/workflows/ci.yml"
    || relative == ".github/workflows/performance.yml"
    || relative == "tools/Cargo.toml"
    || relative == "tools/Cargo.lock"
    || builtins.any (member: relative == "${member}/Cargo.toml") manifestMembers
    || relative == "csr/index.html"
    || relative == "server/assets/jaunder.css"
    || relative == "docs/production-baseline.schema.json"
    || relative == "end2end/e2e-lanes.json"
    || relative == "end2end/otel-collector.yaml"
    || relative == "nix/otel-processors.yaml"
    || pkgs.lib.hasPrefix "host/system_theme_sources/" relative;
  filteredSource =
    assert sourceAdmits "xtask/src/lib.rs" "regular";
    assert sourceAdmits "xtask/tests" "directory";
    assert sourceAdmits "xtask/src/testdata" "directory";
    assert sourceAdmits "xtask/tests/testdata" "directory";
    assert sourceAdmits "xtask/Cargo.toml" "regular";
    assert sourceAdmits "xtask/Cargo.lock" "regular";
    assert sourceAdmits "Justfile" "regular";
    assert sourceAdmits "Cargo.toml" "regular";
    assert sourceAdmits "tools/Cargo.toml" "regular";
    assert sourceAdmits "tools/Cargo.lock" "regular";
    assert sourceAdmits "rust-toolchain.toml" "regular";
    assert sourceAdmits ".cargo/config.toml" "regular";
    assert sourceAdmits ".gitignore" "regular";
    assert sourceAdmits ".github/workflows/ci.yml" "regular";
    assert sourceAdmits ".github/workflows/performance.yml" "regular";
    assert sourceAdmits "nix/otel-processors.yaml" "regular";
    assert sourceAdmits "end2end/otel-collector.yaml" "regular";
    assert sourceAdmits "end2end/e2e-lanes.json" "regular";
    assert sourceAdmits "docs/production-baseline.schema.json" "regular";
    assert sourceAdmits "host/src/capture.rs" "regular";
    assert sourceAdmits "test-support/src" "directory";
    assert sourceAdmits "test-support/src/lib.rs" "regular";
    assert sourceAdmits "xtask/src/server_fn_coverage/testdata/coverage-sample.jsonl" "regular";
    assert sourceAdmits "xtask/src/testdata/e2e-evidence/ordinary-1-trace.jsonl" "regular";
    assert sourceAdmits "xtask/src/testdata/coverage-line-table.txt" "regular";
    assert !(sourceAdmits "tools/devtool/src/main.rs" "regular");
    pkgs.lib.cleanSourceWith {
    src = craneLib.path ../.;
    filter = path: type:
      toString path == root
      || (!(pkgs.lib.hasSuffix "/.git" path || pkgs.lib.hasInfix "/.git/" path)
      && sourceAdmits (pkgs.lib.removePrefix "${root}/" (toString path)) type);
  };
  source = withWorkspacePlaceholders "jaunder-xtask-cargo-source" filteredSource
    (builtins.filter (member: !(builtins.elem member members)) manifestMembers);
  rawClippySource = withWorkspacePlaceholders "jaunder-xtask-clippy-source" filteredSource
    (builtins.filter (member: !(builtins.elem member members) && member != "test-support") manifestMembers);
  vendor = craneLib.vendorCargoDeps {
    src = source;
    cargoLock = "${source}/xtask/Cargo.lock";
    overrideVendorGitCheckout = vendorPinnedFork "xtask";
  };
  rawClippy = craneLib.mkCargoDerivation {
    src = rawClippySource;
    pname = "jaunder-xtask-clippy";
    version = "0.1.0";
    cargoVendorDir = vendor;
    cargoArtifacts = null;
    nativeBuildInputs = [ toolchain pkgs.pkg-config ]
      ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.patchelf ];
    buildInputs = [ pkgs.openssl pkgs.sqlite pkgs.dav1d ];
    CARGO_PROFILE = "dev";
    CARGO_PROFILE_DEV_DEBUG = "0";
    JAUNDER_XTASK_BUILD_NIX_OUTPUT = builtins.placeholder "out";
    buildPhaseCargoCommand = ''
      cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings
    '';
    installPhaseCommand = "mkdir -p $out";
  };
in
craneLib.buildPackage {
  src = source;
  pname = "jaunder-xtask";
  version = "0.1.0";
  meta = {
    mainProgram = "xtask";
    license = pkgs.lib.licenses.gpl3Plus;
  };
  passthru = { inherit sourceAdmits rawClippy; };
  cargoVendorDir = vendor;
  cargoArtifacts = null;
  cargoExtraArgs = "--locked --manifest-path xtask/Cargo.toml --bin xtask";
  CARGO_PROFILE = "dev";
  CARGO_PROFILE_DEV_DEBUG = "0";
  JAUNDER_XTASK_BUILD_NIX_OUTPUT = builtins.placeholder "out";
  # Explicit host tests own this workspace's verdict (ADR-0141).
  doCheck = false;
  strictDeps = true;
  nativeBuildInputs = [ pkgs.pkg-config ]
    ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.patchelf ];
  buildInputs = [ pkgs.openssl pkgs.sqlite pkgs.dav1d ];
  # Cargo binary discovery at the product root would query an unrelated graph.
  doNotPostBuildInstallCargoBinaries = true;
  installPhase = ''
    mkdir -p "$out/bin"
    cp xtask/target/debug/xtask "$out/bin/xtask"
  '';
  postFixup = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
    patchelf --add-rpath ${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.sqlite pkgs.dav1d ]} "$out/bin/xtask"
  '';
}
