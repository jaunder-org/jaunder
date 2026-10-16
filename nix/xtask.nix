# Build the host orchestrator independently; never execute it in a derivation.
{
  pkgs,
  craneLib,
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
      ) (manifestMembers ++ [ ".cargo" "csr" "server/assets" "docs" "end2end" "nix" ])
    else cargoMemberSource members "${root}/${relative}" type
    || relative == "xtask/Cargo.lock"
    || relative == "tools/Cargo.toml"
    || relative == "tools/Cargo.lock"
    || builtins.any (member: relative == "${member}/Cargo.toml") manifestMembers
    || relative == "csr/index.html"
    || relative == "server/assets/jaunder.css"
    || relative == "docs/production-baseline.schema.json"
    || relative == "end2end/e2e-lanes.json"
    || relative == "end2end/otel-collector.yaml"
    || relative == "nix/otel-processors.json"
    || pkgs.lib.hasPrefix "host/system_theme_sources/" relative;
  filteredSource = pkgs.lib.cleanSourceWith {
    src = craneLib.path ../.;
    filter = path: type:
      toString path == root
      || (!(pkgs.lib.hasSuffix "/.git" path || pkgs.lib.hasInfix "/.git/" path)
      && sourceAdmits (pkgs.lib.removePrefix "${root}/" (toString path)) type);
  };
  source = withWorkspacePlaceholders "jaunder-xtask-cargo-source" filteredSource
    (builtins.filter (member: !(builtins.elem member members)) manifestMembers);
  vendor = craneLib.vendorCargoDeps {
    src = source;
    cargoLock = "${source}/xtask/Cargo.lock";
    overrideVendorGitCheckout = vendorPinnedFork "xtask";
  };
in
craneLib.buildPackage {
  src = source;
  pname = "jaunder-xtask";
  version = "0.1.0";
  meta.mainProgram = "xtask";
  passthru = { inherit sourceAdmits; };
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
