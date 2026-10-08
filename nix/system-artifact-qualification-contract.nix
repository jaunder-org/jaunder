# Evaluate the actual private package graph without realizing qualification
# releases. Runtime inventory and browser continuity remain separate proofs.
{
  sourceFlake,
  system,
}:
let
  lib = sourceFlake.inputs.nixpkgs.lib;
  ordinary = sourceFlake.packages.${system};
  mk =
    fixture:
    import ./system-artifact-qualification.nix {
      inherit sourceFlake system fixture;
    };
  fixtures = [
    "a"
    "b-app"
    "b-theme"
  ];
  packages = map mk fixtures;
  rejects = fixture: !(builtins.tryEval (mk fixture)).success;
  variantMatches =
    fixture:
    let
      package = mk fixture;
      bundle = package.csrBundle;
      producer = builtins.head bundle.nativeBuildInputs;
    in
    package.fixture == fixture
    && package.JAUNDER_CSR_BUNDLE_DIR == toString bundle
    && package.systemArtifactInventory == "${bundle}/system-artifacts"
    && lib.hasInfix "--features qualification" producer.buildPhase
    && lib.hasInfix "--system-artifact-fixture ${fixture}" bundle.buildCommand
    # A regex pattern cannot carry a store context. Only the comparison pattern
    # loses its context; the real build command retains its dependency edge.
    && lib.hasInfix (builtins.unsafeDiscardStringContext "${ordinary.csrWasm}/lib/csr.wasm") bundle.buildCommand;
  constructorArguments = builtins.attrNames (
    builtins.functionArgs (import ./system-artifact-qualification.nix)
  );
in
assert builtins.all rejects [
  ""
  "A"
  "b"
  "b_app"
  "reader"
  "../a"
  "style.css"
];
assert
  constructorArguments == [
    "fixture"
    "sourceFlake"
    "system"
  ];
assert builtins.all variantMatches fixtures;
assert builtins.length (lib.unique (map (package: package.drvPath) packages)) == 3;
assert builtins.length (lib.unique (map (package: package.csrBundle.drvPath) packages)) == 3;
assert !(lib.hasInfix "--features qualification" ordinary.devtool.buildPhase);
assert !(lib.hasInfix "--system-artifact-fixture" ordinary.csrBundle.buildCommand);
{
  acceptedFixtures = fixtures;
  unknownFixturesRejected = true;
  inherit constructorArguments;
  sharedWasmAndExactBundleWiring = true;
  ordinaryReleaseExcludesQualification = true;
}
