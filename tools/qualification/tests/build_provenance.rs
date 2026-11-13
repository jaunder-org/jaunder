//! Exercise the build script's input separately from Cargo's inherited runtime
//! metadata: only the dedicated Nix build input may give the tool provenance.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn inherited_runtime_provenance_never_marks_a_host_build_as_nix_built() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("build-script");
    let output = Command::new("rustc")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build.rs"))
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Cargo-run exports rustc-env metadata to children. Neither an empty value
    // nor a parent's store identity may classify a host build as Nix-built.
    for inherited in [
        "",
        "/nix/store/00000000000000000000000000000000-parent-tool",
    ] {
        let output = Command::new(&binary)
            .env_remove("JAUNDER_QUALIFICATION_BUILD_NIX_OUTPUT")
            .env("JAUNDER_QUALIFICATION_NIX_OUTPUT", inherited)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("cargo:rustc-env=JAUNDER_QUALIFICATION_NIX_OUTPUT=\n"),
            "{stdout}"
        );
    }

    // The dedicated build input is the only provenance source, and it must be a store path.
    let built = Command::new(&binary)
        .env(
            "JAUNDER_QUALIFICATION_BUILD_NIX_OUTPUT",
            "/nix/store/abc-qualification-host",
        )
        .output()
        .unwrap();
    assert!(String::from_utf8(built.stdout).unwrap().contains(
        "cargo:rustc-env=JAUNDER_QUALIFICATION_NIX_OUTPUT=/nix/store/abc-qualification-host\n"
    ));
    let refused = Command::new(&binary)
        .env(
            "JAUNDER_QUALIFICATION_BUILD_NIX_OUTPUT",
            "/tmp/not-a-store-path",
        )
        .output()
        .unwrap();
    assert!(!refused.status.success());
}
