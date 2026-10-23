{
  pkgs,
  craneLib,
  mkToolingCargo,
  devtoolBin,
  nonSubstitutable,
}:
let
  src = pkgs.runCommand "source" { } ''
    cp --no-preserve=mode -r ${./testdata/tooling-cargo}/. "$out/"
    # Rust fixtures are test data (ADR-0095); materialize Cargo sources only here.
    mv "$out/shared/src/lib.rs.txt" "$out/shared/src/lib.rs"
    mv "$out/tools/probe/src/main.rs.txt" "$out/tools/probe/src/main.rs"
  '';
  cargoLock = ./testdata/tooling-cargo/tools/Cargo.lock;
  cargoVendorDir = craneLib.vendorCargoDeps { inherit cargoLock; };
  changedSource =
    script:
    pkgs.runCommand "source" { } ''
      cp --no-preserve=mode -r ${src}/. "$out/"
      ${script}
    '';
  leafSrc = changedSource ''
    substituteInPlace "$out/tools/probe/src/main.rs" \
      --replace-fail 'producer {}' 'consumer {}'
  '';
  sharedSrc = changedSource ''
    substituteInPlace "$out/shared/src/lib.rs" \
      --replace-fail '7_u32' '9_u32'
  '';
  # Dependency-affecting manifests and the explicit lock stay preparation inputs.
  # Lock format 3 is also valid for this graph: no registry upgrade is needed to
  # check lockfile admission (this is not a resolved-feature-graph experiment).
  manifestSrc = changedSource ''
    substituteInPlace "$out/shared/Cargo.toml" \
      --replace-fail 'itoa = "=1.0.18"' 'itoa = { version = "=1.0.18", default-features = false }'
  '';
  changedLock = pkgs.writeText "Cargo.lock" (
    builtins.replaceStrings [ "version = 4" ] [ "version = 3" ] (builtins.readFile cargoLock)
  );
  args = source: {
    src = source;
    inherit cargoLock cargoVendorDir;
    pname = "tooling-cargo-regression";
    version = "0.1.0";
    strictDeps = true;
    cargoExtraArgs = "--locked --manifest-path tools/Cargo.toml";
    postPatch = ''
      ln -sf ../Cargo.lock tools/Cargo.lock
      bash ${./tooling-workspace-patch.sh} Cargo.toml
      # Both dummy preparation and real consumers must resolve the patched root.
      if grep -E '^[[:space:]]*"(client|csr|server|test-support|web)"' Cargo.toml; then
        echo 'unpatched tooling root workspace' >&2
        exit 1
      fi
      grep -E '^[[:space:]]*"shared"' Cargo.toml
      test "$(readlink tools/Cargo.lock)" = ../Cargo.lock
    '';
  };
  tooling = mkToolingCargo (args src);
  leafTooling = mkToolingCargo (args leafSrc);
  sharedTooling = mkToolingCargo (args sharedSrc);
  manifestTooling = mkToolingCargo (args manifestSrc);
  lockTooling = mkToolingCargo (
    (args src)
    // {
      cargoLock = changedLock;
      cargoVendorDir = craneLib.vendorCargoDeps { cargoLock = changedLock; };
    }
  );
  consumer =
    label: source:
    nonSubstitutable (
      tooling.buildPackage {
        src = source;
        pname = "tooling-cargo-regression-${label}";
        doCheck = false;
        doNotPostBuildInstallCargoBinaries = true;
        nativeBuildInputs = [ pkgs.jq ];
        preBuild = ''
          find target -name 'libitoa-*.rlib' > imported-registry-artifacts.txt
          test -s imported-registry-artifacts.txt
        '';
        # JSON compiler messages distinguish imported freshness from a cached final output.
        cargoBuildCommand = "cargoWithProfile build --verbose";
        postBuild = ''
          jq -s -e '
            any(.[]; .reason == "compiler-artifact" and .target.name == "itoa" and .fresh == true)
            and any(.[]; .reason == "compiler-artifact" and .target.name == "artifact_shared" and .fresh == false)
            and any(.[]; .reason == "compiler-artifact" and .target.name == "artifact-probe" and .fresh == false)
          ' "$cargoBuildLog" > /dev/null
        '';
        installPhase = ''
          mkdir -p "$out/bin"
          cp target/release/artifact-probe "$out/bin/artifact-probe"
        '';
      }
    );
  leaf = consumer "leaf" leafSrc;
  shared = consumer "shared" sharedSrc;
  # Clippy needs compatible dev metadata, not the release executable archive.
  # Invoke the shared selector so its all-targets and warnings policy has one owner.
  devArgs =
    source:
    (args source)
    // {
      CARGO_PROFILE = "dev";
      CARGO_TERM_VERBOSE = "true";
      nativeBuildInputs = [ devtoolBin ];
      doCheck = false;
      buildPhaseCargoCommand = "devtool check tools-clippy";
    };
  devTooling = mkToolingCargo (devArgs src);
  # A test-only lint makes dropping --all-targets observable without weakening policy.
  lintSrc = changedSource ''
    substituteInPlace "$out/tools/probe/src/main.rs" --replace-fail 'fn main() {' '#[cfg(test)]
    #[test]
    fn rejects_unused_binding() {
        let regression_unused = artifact_shared::value();
    }

    fn main() {'
  '';
  clippyConsumer =
    label: source: expectLint:
    nonSubstitutable (
      devTooling.buildPackage {
        src = source;
        pname = "tooling-clippy-regression-${label}";
        doNotPostBuildInstallCargoBinaries = true;
        preBuild = ''
          find target/debug -name 'libitoa-*.rmeta' > imported-dev-artifacts.txt
          test -s imported-dev-artifacts.txt
        '';
        buildPhaseCargoCommand = ''
          status=0
          devtool check tools-clippy >clippy.out 2>clippy.err || status=$?
          cat clippy.out clippy.err
          # Fresh external metadata and a rechecked local target must be observable.
          grep -F 'Fresh itoa v1.0.18' clippy.err
          grep -F 'Checking artifact-probe v0.1.0' clippy.err
          ${
            if expectLint then
              ''
                if [ "$status" -eq 0 ]; then
                  echo 'expected unused_variables failure was not produced' >&2
                  exit 1
                fi
                test "$status" -eq 1
                grep -F 'error: unused variable: `regression_unused`' clippy.err
                grep -F 'unused-variables' clippy.err
                grep -F 'error: could not compile `artifact-probe` (bin "artifact-probe" test) due to 1 previous error' clippy.err
                grep -F 'implied by `-D warnings`' clippy.err
              ''
            else
              ''
                test "$status" -eq 0
                grep -F 'Finished `dev` profile' clippy.err
              ''
          }
        '';
        installPhase = ''
          mkdir -p "$out"
          cp clippy.out clippy.err "$out/"
        '';
      }
    );
  clippyLeaf = clippyConsumer "leaf" leafSrc false;
  clippyLint = clippyConsumer "lint" lintSrc true;
in
assert tooling.cargoArtifacts.drvPath == leafTooling.cargoArtifacts.drvPath;
assert tooling.cargoArtifacts.drvPath == sharedTooling.cargoArtifacts.drvPath;
assert tooling.cargoArtifacts.drvPath != manifestTooling.cargoArtifacts.drvPath;
assert tooling.cargoArtifacts.drvPath != lockTooling.cargoArtifacts.drvPath;
assert tooling.cargoArtifacts.drvPath != devTooling.cargoArtifacts.drvPath;
assert
  devTooling.cargoArtifacts.drvPath == (mkToolingCargo (devArgs leafSrc)).cargoArtifacts.drvPath;
# Hold every consumer argument constant except the real source input.
assert (tooling.buildPackage { }).drvPath != (tooling.buildPackage { src = leafSrc; }).drvPath;
assert (tooling.buildPackage { }).drvPath != (tooling.buildPackage { src = sharedSrc; }).drvPath;
# Verdicts cannot come from a substituter; preparations intentionally remain cacheable.
# Locally valid verdict outputs still require separate executing-CI evidence.
nonSubstitutable (
  pkgs.runCommand "tooling-cargo-artifacts" { } ''
    # Run the exact production transformation against both supported TOML layouts.
    cp ${./testdata/tooling-workspace/raw.toml} raw-Cargo.toml
    bash ${./tooling-workspace-patch.sh} raw-Cargo.toml
    cmp raw-Cargo.toml ${./testdata/tooling-workspace/expected-raw.toml}
    cp ${./testdata/tooling-workspace/generated.toml} generated-Cargo.toml
    bash ${./tooling-workspace-patch.sh} generated-Cargo.toml
    cmp generated-Cargo.toml ${./testdata/tooling-workspace/expected-generated.toml}
    # A real source edit must execute its new behavior, not an imported local dummy.
    ${leaf}/bin/artifact-probe > leaf-output.txt
    grep -Fx 'consumer 7' leaf-output.txt
    ${shared}/bin/artifact-probe > shared-output.txt
    grep -Fx 'producer 9' shared-output.txt
    # Both the successful changed leaf and the diagnostic-specific negative case
    # must execute through the shared dev-profile clippy selector.
    test -f ${clippyLeaf}/clippy.err
    test -f ${clippyLint}/clippy.err
    mkdir -p "$out"
  ''
)
