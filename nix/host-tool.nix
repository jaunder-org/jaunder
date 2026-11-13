# A host-run auxiliary tool package built from its real Cargo path closure in
# the tools workspace. It never executes inside a derivation; callers run its
# output on the host (ADR-0246).
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
{
  pname,
  member,
  bins,
  # Non-Cargo files the executables compile in or their build scripts read.
  extraSourceAdmits ? (_relative: false),
  env ? { },
}:
let
  root = toString ../.;
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
    # Host's build script stages the system theme inventory and CSR shell.
    || relative == "csr/index.html"
    || relative == "server/assets/jaunder.css"
    || pkgs.lib.hasPrefix "host/system_theme_sources/" relative
    || pkgs.lib.hasPrefix "storage/migrations/" relative
    || sqlxSqlitePatchSource "${root}/${relative}"
    || extraSourceAdmits relative;
  source =
    assert sourceAdmits "${member}/Cargo.toml" "regular";
    withWorkspacePlaceholders "${pname}-source" (
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
    overrideVendorGitCheckout = vendorPinnedFork pname;
  };
  libraryPath = pkgs.lib.makeLibraryPath [
    pkgs.openssl
    pkgs.sqlite
    pkgs.dav1d
  ];
in
craneLib.buildPackage (
  {
    inherit pname;
    version = "0.1.0";
    src = source;
    cargoVendorDir = vendor;
    cargoArtifacts = null;
    cargoExtraArgs =
      "--locked --manifest-path tools/Cargo.toml -p ${builtins.baseNameOf member} "
      + pkgs.lib.concatMapStringsSep " " (bin: "--bin ${bin}") bins;
    CARGO_PROFILE = "dev";
    CARGO_PROFILE_DEV_DEBUG = "0";
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
      for bin in ${pkgs.lib.escapeShellArgs bins}; do
        cp "tools/target/debug/$bin" "$out/bin/$bin"
      done
    '';
    postFixup = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
      for bin in ${pkgs.lib.escapeShellArgs bins}; do
        patchelf --add-rpath ${libraryPath} "$out/bin/$bin"
      done
    '';
    meta = {
      license = pkgs.lib.licenses.gpl3Plus;
      mainProgram = builtins.head bins;
    };
    passthru = { inherit sourceAdmits; };
  }
  // env
)
