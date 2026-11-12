//! Public Just forwarding remains literal, artifact-only and independent of Git bootstrap.
#[cfg(unix)]
#[test]
fn explicit_file_frontdoor_preserves_argv_streams_status_and_no_bootstrap() {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
    let root = tempfile::tempdir().unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Justfile"),
        root.path().join("Justfile"),
    )
    .unwrap();
    fs::create_dir(root.path().join("bin")).unwrap();
    let cargo = root.path().join("bin/cargo");
    fs::write(
        &cargo,
        "#!/bin/sh\nwhile [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$ORG_BINARY\" \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    // Shell syntax and whitespace must remain one original-file argument.
    let name = "local ; touch unexpected.org";
    let text = "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n";
    fs::write(root.path().join(name), text).unwrap();
    let mut paths = vec![root.path().join("bin")];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    for (file, status) in [(name, 0), ("missing.org", 1)] {
        let output = Command::new("just")
            .args(["org-timezone-manifest", file])
            .current_dir(root.path())
            .env("PATH", std::env::join_paths(&paths).unwrap())
            .env("ORG_BINARY", env!("CARGO_BIN_EXE_org-timezone-manifest"))
            .env("GITHUB_STEP_SUMMARY", root.path().join("summary.md"))
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(status),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["mappings"].as_array().unwrap().len(),
            usize::from(status == 0)
        );
        assert!(!root.path().join("summary.md").exists());
        assert!(!root.path().join(".xtask").exists());
        assert!(!root.path().join("unexpected.org").exists());
    }
    assert_eq!(fs::read_to_string(root.path().join(name)).unwrap(), text);
}
