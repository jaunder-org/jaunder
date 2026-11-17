{ craneLib }:
args:
let
  # A nested tools workspace must compile into the same target the archive hooks use.
  buildArgs = args // { CARGO_TARGET_DIR = "target"; };
  cargoArtifacts = craneLib.buildDepsOnly buildArgs;
in
{
  inherit cargoArtifacts;
  # `cargo test --doc` over the workspace, sharing the dependency build.
  docTest = overrides:
    craneLib.cargoDocTest (buildArgs // overrides // { inherit cargoArtifacts; });
  buildPackage = overrides:
    craneLib.buildPackage (
      buildArgs
      // overrides
      // {
        inherit cargoArtifacts;
        inherit (buildArgs) CARGO_TARGET_DIR;
        # An empty producer output is not an imported compiled-dependency cache.
        preBuild = ''
          test -s ${cargoArtifacts}/target.tar.zst
        '' + (args.preBuild or "") + (overrides.preBuild or "");
      }
    );
}
