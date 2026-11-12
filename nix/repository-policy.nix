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
in
craneLib.buildPackage {
  pname = "repository-policy";
  version = "0.1.0";
  src = source;
  cargoVendorDir = vendor;
  cargoArtifacts = null;
  cargoExtraArgs = "--locked --manifest-path tools/Cargo.toml -p repository-policy --bins";
  CARGO_PROFILE = "dev";
  CARGO_PROFILE_DEV_DEBUG = "0";
  JAUNDER_REPOSITORY_POLICY_SOURCE = ".#repository-policy.sourceAdmits";
  # Auxiliary native tests run through the host tools workspace, not inside Nix.
  doCheck = false;
  doNotPostBuildInstallCargoBinaries = true;
  installPhase = ''
    mkdir -p "$out/bin"
    cp tools/target/debug/{repository-docs,source-contracts,git-index-safety,org-timezone-manifest,scoped-leptosfmt} "$out/bin/"
  '';
  meta = {
    license = pkgs.lib.licenses.gpl3Plus;
    mainProgram = "repository-docs";
  };
  passthru = { inherit sourceAdmits cargoSourceAdmits; };
}
