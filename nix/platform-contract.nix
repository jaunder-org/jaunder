# Evaluation-only regression for the public platform boundary.
# Run with:
# nix eval --impure --expr 'import ./nix/platform-contract.nix { flake = builtins.getFlake (toString ./.); }'
{ flake }:
let
  systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
  portable = [ "jaunder" "site" "csrWasm" "csrBundle" "devtool" "test-support" ];
  linuxOnly = [ "theme-thumbnail-environment" "wasm-coverage-csr" ];
  check = system:
    let
      packages = flake.packages.${system};
      shells = flake.devShells.${system};
      linux = builtins.match ".*-linux" system != null;
      paths = map (name: packages.${name}.drvPath) portable
        ++ map (name: shells.${name}.drvPath) [ "default" "ci" ];
    in
    assert builtins.all (name: builtins.hasAttr name packages == linux) linuxOnly;
    assert builtins.hasAttr "theme-thumbnail" shells == linux;
    assert linux || builtins.attrNames flake.checks.${system} == [ ];
    assert builtins.hasAttr "FONTCONFIG_FILE" shells.ci == linux;
    assert builtins.hasAttr "JAUNDER_THEME_THUMBNAIL_BROWSER" shells.default == linux;
    assert shells.ci.PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD == "1";
    # Force derivation evaluation, not just attribute membership: platform
    # compatibility of browser and SDK dependencies requires forcing paths.
    # Cargo source preparation uses import-from-derivation, so force paths
    # only on the native system; other systems need their own runner/builder.
    assert system != builtins.currentSystem
      || builtins.all (path: builtins.isString path && path != "") paths;
    true;
in
assert builtins.attrNames flake.devShells == builtins.sort builtins.lessThan systems;
builtins.all check systems
