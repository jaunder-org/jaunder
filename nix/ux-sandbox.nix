# Keep helper compilation independent of application artifacts and their assets.
{
  pkgs,
  craneLib,
  cargoPackageClosure,
  cargoMemberSource,
  workspaceMembers,
  withWorkspacePlaceholders,
  vendorPinnedFork,
  sqlxSqlitePatchRoot,
  sqlxSqlitePatchSource,
}:
let
  root = toString ../.;
  members = cargoPackageClosure [ "ux-sandbox" ];
  # Cargo still resolves the workspace graph; excluded targets need manifests
  # and placeholders, not their implementation or embedded application inputs.
  metadataMembers = cargoPackageClosure workspaceMembers;
  metadataInputs = [
    "Cargo.toml"
    "Cargo.lock"
    "rust-toolchain.toml"
    ".cargo/config.toml"
  ] ++ map (member: "${member}/Cargo.toml") metadataMembers;
  filteredSource = pkgs.lib.cleanSourceWith {
    src = craneLib.path ../.;
    filter = path: type:
      let relative = pkgs.lib.removePrefix "${root}/" (toString path);
      in
      if type == "directory" then
        toString path == root
        # Root Cargo metadata resolves this patch even without a SQLx consumer.
        || relative == "vendor"
        || relative == sqlxSqlitePatchRoot
        || relative == "${sqlxSqlitePatchRoot}/src"
        || pkgs.lib.hasPrefix "${sqlxSqlitePatchRoot}/src/" relative
        || builtins.any (input: pkgs.lib.hasPrefix "${relative}/" input) metadataInputs
        || builtins.any (member:
          relative == member || pkgs.lib.hasPrefix "${member}/" relative
        ) members
      else
        sqlxSqlitePatchSource path
        || builtins.elem relative metadataInputs
        || cargoMemberSource members path type;
  };
  source = withWorkspacePlaceholders "ux-sandbox-cargo-source" filteredSource
    (builtins.filter (member: !(builtins.elem member members)) metadataMembers);
  vendor = craneLib.vendorCargoDeps {
    src = source;
    overrideVendorGitCheckout = vendorPinnedFork "ux-sandbox";
  };
in
craneLib.buildPackage {
  pname = "ux-sandbox";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;
  src = source;
  cargoVendorDir = vendor;
  cargoExtraArgs = "--locked -p ux-sandbox --bin ux-sandbox";
  CARGO_PROFILE = "dev";
  strictDeps = true;
  # Behavioral verdicts remain in the existing host/coverage test lanes.
  doCheck = false;
  meta = {
    mainProgram = "ux-sandbox";
    license = pkgs.lib.licenses.gpl3Plus;
  };
}
