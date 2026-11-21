# A host-run executable package built from the nested tools workspace, on a
# dependencies-only build that the source cannot invalidate: only the
# lockfile, toolchain, features and the members' manifests change it. A source
# edit inside the closure (common/, host/, the tool) recompiles workspace
# crates alone (ADR draft docs/adr/0258-dev-runs-nix-built-host-tools.md).
{ pkgs, craneLib }:
{
  pname,
  # The filtered checkout source, and Cargo arguments selecting the package.
  source,
  cargoExtraArgs,
  vendorPinnedFork,
  bins,
  # Crane arguments both builds share (inputs, dummy-source fixups).
  shared ? { },
  # Arguments of the final package only (env, fixups, meta, passthru).
  package ? { },
}:
let
  # A store copy of the lockfile alone: a string into `source` would carry the
  # whole source as context and rebuild the dependencies on every edit.
  cargoLock = ../tools/Cargo.lock;
  common = {
    inherit pname cargoExtraArgs cargoLock;
    version = "0.1.0";
    src = source;
    cargoVendorDir = craneLib.vendorCargoDeps {
      inherit cargoLock;
      overrideVendorGitCheckout = vendorPinnedFork pname;
    };
    # Crane places the lock at the source root (its dummy source keeps no
    # nested lock); the link gives the nested workspace the same one.
    postPatch = ''
      ln -sf ../Cargo.lock tools/Cargo.lock
    '';
    # The nested workspace must compile into the target crane's dependency
    # archive restores.
    CARGO_TARGET_DIR = "target";
    CARGO_PROFILE = "dev";
    CARGO_PROFILE_DEV_DEBUG = "0";
    # Auxiliary native tests run through the host tools workspace, not inside Nix.
    doCheck = false;
    strictDeps = true;
  }
  // shared;
in
craneLib.buildPackage (
  common
  // {
    cargoArtifacts = craneLib.buildDepsOnly common;
    cargoExtraArgs = cargoExtraArgs + pkgs.lib.concatMapStrings (bin: " --bin ${bin}") bins;
    doNotPostBuildInstallCargoBinaries = true;
    installPhase = ''
      mkdir -p "$out/bin"
      for bin in ${pkgs.lib.escapeShellArgs bins}; do
        cp "target/debug/$bin" "$out/bin/$bin"
      done
    '';
    meta = {
      license = pkgs.lib.licenses.gpl3Plus;
      mainProgram = builtins.head bins;
    };
  }
  // package
)
