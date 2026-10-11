//! Exercise build-script inputs separately from Cargo's inherited runtime metadata.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn cargo_runtime_provenance_does_not_select_nix_build_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("checkout");
    let manifest = root.join("xtask");
    fs::create_dir_all(&manifest).unwrap();
    fs::write(manifest.join("Cargo.toml"), "[workspace]\n").unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec!["add", "xtask/Cargo.toml"],
        vec![
            "-c",
            "core.hooksPath=",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    ] {
        let output = xtask::git::at(&root).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
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

    // Cargo-run exports rustc-env metadata. Neither an empty native identity nor
    // a parent's store identity may classify a child host build as Nix-built.
    for inherited in [
        "",
        "/nix/store/00000000000000000000000000000000-parent-xtask",
    ] {
        let output = Command::new(&binary)
            .env("CARGO_MANIFEST_DIR", &manifest)
            .env_remove("JAUNDER_XTASK_BUILD_NIX_OUTPUT")
            .env("JAUNDER_XTASK_NIX_OUTPUT", inherited)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("cargo:rustc-env=JAUNDER_XTASK_NIX_OUTPUT=\n"));
        assert!(stdout.contains("cargo:rustc-env=JAUNDER_BUILD_DIRTY=0\n"));
    }
}
