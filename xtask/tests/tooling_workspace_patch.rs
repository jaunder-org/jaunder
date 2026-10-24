//! Protect the tooling workspace population without changing unrelated TOML.

use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn tooling_workspace_patch_preserves_unrelated_tables_in_both_manifest_layouts() {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixtures = checkout.join("nix/testdata/tooling-workspace");
    let temp = tempfile::tempdir().unwrap();
    for layout in ["raw", "generated"] {
        let manifest = temp.path().join(format!("{layout}-Cargo.toml"));
        fs::copy(fixtures.join(format!("{layout}.toml")), &manifest).unwrap();
        let output = Command::new("bash")
            .arg(checkout.join("nix/tooling-workspace-patch.sh"))
            .arg(&manifest)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{layout}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read(&manifest).unwrap(),
            fs::read(fixtures.join(format!("expected-{layout}.toml"))).unwrap(),
            "{layout}: only the selected workspace members should be removed"
        );
    }
}
