# Native repository policy admits only its real Cargo path closure (including
# common/macros for Org parsing), never the product or other host tool owners.
{
  pkgs,
  craneLib,
  cargoMemberSource,
  cargoPackageClosure,
  workspaceMembers,
  withWorkspacePlaceholders,
  vendorPinnedFork,
}:
let
  root = toString ../.;
  member = "tools/repository-policy";
  toolsMembers = map (name: "tools/${name}") (
    builtins.fromTOML (builtins.readFile ../tools/Cargo.toml)
  ).workspace.members;
  members = cargoPackageClosure [ member ];
  placeholders = builtins.filter (name: !(builtins.elem name members)) (workspaceMembers ++ toolsMembers);
  cargoSourceAdmits = relative: type:
    type == "directory"
    || cargoMemberSource members "${root}/${relative}" type
    || relative == "tools/Cargo.toml"
    || relative == "tools/Cargo.lock"
    || builtins.any (name: relative == "${name}/Cargo.toml") placeholders;
  # Output admission includes evaluation inputs; the Cargo source stays narrower.
  sourceAdmits = relative: type:
    cargoSourceAdmits relative type
    || relative == "flake.nix"
    || relative == "flake.lock"
    || pkgs.lib.hasPrefix "nix/" relative;
  source = withWorkspacePlaceholders "jaunder-repository-policy-source" (
    pkgs.lib.cleanSourceWith {
      src = craneLib.path ../.;
      filter = path: type:
        cargoSourceAdmits (pkgs.lib.removePrefix "${root}/" (toString path)) type;
    }
  ) placeholders;
  vendor = craneLib.vendorCargoDeps {
    src = source;
    cargoLock = "${source}/tools/Cargo.lock";
    overrideVendorGitCheckout = vendorPinnedFork "repository-policy";
  };
  common = {
    pname = "repository-policy";
    version = "0.1.0";
    src = source;
    cargoVendorDir = vendor;
    # Crane places this lock at the source root (its dummy source keeps no
    # nested lock); the link gives the nested workspace the same one.
    cargoLock = "${source}/tools/Cargo.lock";
    postPatch = ''
      ln -sf ../Cargo.lock tools/Cargo.lock
    '';
    # The nested tools workspace must compile into the target crane's
    # dependency archive restores.
    CARGO_TARGET_DIR = "target";
    cargoExtraArgs = "--locked --manifest-path tools/Cargo.toml -p repository-policy";
    CARGO_PROFILE = "dev";
    CARGO_PROFILE_DEV_DEBUG = "0";
    # Auxiliary native tests run through the host tools workspace, not inside Nix.
    doCheck = false;
  };
in
craneLib.buildPackage (common // {
  # Depends only on the manifests and lockfile, so a source edit recompiles
  # workspace crates alone.
  cargoArtifacts = craneLib.buildDepsOnly common;
  cargoExtraArgs = common.cargoExtraArgs + " --bins";
  JAUNDER_REPOSITORY_POLICY_SOURCE = ".#repository-policy.sourceAdmits";
  doNotPostBuildInstallCargoBinaries = true;
  installPhase = ''
    mkdir -p "$out/bin"
    cp target/debug/{repository-docs,source-contracts,org-timezone-manifest,scoped-leptosfmt} "$out/bin/"
  '';
  meta = {
    license = pkgs.lib.licenses.gpl3Plus;
    mainProgram = "repository-docs";
  };
  passthru = { inherit sourceAdmits cargoSourceAdmits; };
})
