# Internal host-harness entry point: the caller admits the source revision,
# while the package layer admits only closed fixture selectors. Ordinary flake
# package and application outputs do not expose this constructor.
{
  sourceFlake,
  system,
  fixture,
}:
let
  packageLayer = import "${sourceFlake.outPath}/nix/packages.nix" {
    inherit system;
    pkgs = sourceFlake.inputs.nixpkgs.legacyPackages.${system};
    fenix = sourceFlake.inputs.fenix;
    crane = sourceFlake.inputs.crane;
    atom-fork = sourceFlake.inputs.atom-fork;
    orgize-fork = sourceFlake.inputs.orgize-fork;
  };
in
packageLayer.internals.mkSystemArtifactQualificationPackage fixture
