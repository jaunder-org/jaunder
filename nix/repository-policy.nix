# Native repository policy admits only its real Cargo path closure (itself and
# the shared host-tool library), never the product or other host tool owners.
{
  pkgs,
  craneLib,
  cargoMemberSource,
  cargoPackageClosure,
  workspaceMembers,
  withWorkspacePlaceholders,
  vendorPinnedFork,
  mkToolsWorkspacePackage,
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
in
mkToolsWorkspacePackage {
  pname = "repository-policy";
  inherit source vendorPinnedFork;
  cargoExtraArgs = "--locked --manifest-path tools/Cargo.toml -p repository-policy";
  bins = [
    "repository-docs"
    "source-contracts"
    "scoped-leptosfmt"
  ];
  package = {
    JAUNDER_REPOSITORY_POLICY_SOURCE = ".#repository-policy.sourceAdmits";
    passthru = { inherit sourceAdmits cargoSourceAdmits; };
  };
}
