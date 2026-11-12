# The host-run qualification executable. Nix builds it from its real Cargo path
# closure and binds the executable to its own output; it never executes inside a
# derivation (ADR-0239, ADR-0246).
{
  pkgs,
  craneLib,
  cargoMemberSource,
  cargoPackageClosure,
  workspaceMembers,
  withWorkspacePlaceholders,
  vendorPinnedFork,
  sqlxSqlitePatchSource,
}:
let
  root = toString ../.;
  member = "tools/qualification";
  toolsMembers = map (name: "tools/${name}") (
    builtins.fromTOML (builtins.readFile ../tools/Cargo.toml)
  ).workspace.members;
  members = cargoPackageClosure [ member ];
  placeholders = builtins.filter (name: !(builtins.elem name members)) (workspaceMembers ++ toolsMembers);
  sourceAdmits =
    relative: type:
    type == "directory"
    || cargoMemberSource members "${root}/${relative}" type
    || relative == "tools/Cargo.toml"
    || relative == "tools/Cargo.lock"
    || builtins.any (name: relative == "${name}/Cargo.toml") placeholders
    # The evidence schema is compiled into the executable.
    || relative == "docs/production-baseline.schema.json"
    # Host's build script stages the system theme inventory and CSR shell.
    || relative == "csr/index.html"
    || relative == "server/assets/jaunder.css"
    || pkgs.lib.hasPrefix "host/system_theme_sources/" relative
    || pkgs.lib.hasPrefix "storage/migrations/" relative
    || sqlxSqlitePatchSource "${root}/${relative}";
  source =
    assert sourceAdmits "tools/qualification/src/main.rs" "regular";
    assert sourceAdmits "docs/production-baseline.schema.json" "regular";
    assert !(sourceAdmits "tools/devtool/src/main.rs" "regular");
    assert !(sourceAdmits "xtask/src/main.rs" "regular");
    withWorkspacePlaceholders "jaunder-qualification-source" (
      pkgs.lib.cleanSourceWith {
        src = craneLib.path ../.;
        filter =
          path: type:
          !(pkgs.lib.hasSuffix "/.git" path || pkgs.lib.hasInfix "/.git/" path)
          && sourceAdmits (pkgs.lib.removePrefix "${root}/" (toString path)) type;
      }
    ) placeholders;
  vendor = craneLib.vendorCargoDeps {
    src = source;
    cargoLock = "${source}/tools/Cargo.lock";
    overrideVendorGitCheckout = vendorPinnedFork "qualification";
  };
in
craneLib.buildPackage {
  pname = "qualification-host";
  version = "0.1.0";
  src = source;
  cargoVendorDir = vendor;
  cargoArtifacts = null;
  cargoExtraArgs = "--locked --manifest-path tools/Cargo.toml -p qualification --bin qualification-host";
  CARGO_PROFILE = "dev";
  CARGO_PROFILE_DEV_DEBUG = "0";
  JAUNDER_QUALIFICATION_BUILD_NIX_OUTPUT = builtins.placeholder "out";
  # Auxiliary native tests run through the host tools workspace, not inside Nix.
  doCheck = false;
  strictDeps = true;
  nativeBuildInputs = [
    pkgs.pkg-config
  ]
  ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.patchelf ];
  buildInputs = [
    pkgs.openssl
    pkgs.sqlite
    pkgs.dav1d
  ];
  doNotPostBuildInstallCargoBinaries = true;
  installPhase = ''
    mkdir -p "$out/bin"
    cp tools/target/debug/qualification-host "$out/bin/qualification-host"
  '';
  postFixup = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
    patchelf --add-rpath ${
      pkgs.lib.makeLibraryPath [
        pkgs.openssl
        pkgs.sqlite
        pkgs.dav1d
      ]
    } "$out/bin/qualification-host"
  '';
  meta = {
    license = pkgs.lib.licenses.gpl3Plus;
    mainProgram = "qualification-host";
  };
  passthru = { inherit sourceAdmits; };
}
