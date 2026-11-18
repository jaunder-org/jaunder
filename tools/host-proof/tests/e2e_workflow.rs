//! Artifact reconciliation reads kept evidence only. The `dev` workflows that
//! drive producers and reconciliations are tested in `tools/dev/tests/`.
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

#[test]
fn artifact_reconciliation_is_build_free_and_has_no_result_envelope() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    for dir in ["end2end", "bin"] {
        fs::create_dir(root.path().join(dir)).unwrap();
    }
    fs::copy(
        repo.join("end2end/e2e-lanes.json"),
        root.path().join("end2end/e2e-lanes.json"),
    )
    .unwrap();
    // Any build attempt leaves an event behind.
    let nix = root.path().join("bin/nix");
    fs::write(&nix, "#!/bin/sh\necho \"nix $*\" >> \"$EVENTS\"\nexit 1\n").unwrap();
    fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_e2e-evidence"))
        .args([
            "reconcile",
            "sqlite",
            "--catalog",
            "end2end/e2e-lanes.json",
            "--diagnostics-root",
            "missing",
        ])
        .current_dir(root.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.path().join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("EVENTS", root.path().join("events"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        !root.path().join("events").exists(),
        "artifact reconciliation invoked a build"
    );
    assert!(!root.path().join(".xtask/last-result.json").exists());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert!(
        report["steps"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("reading expected census")
    );
}
