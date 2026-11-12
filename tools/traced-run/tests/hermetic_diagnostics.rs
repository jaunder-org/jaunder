//! Structural protection of the selected native CI lane and its optional upload.
use std::process::Command;

fn lane() -> &'static str {
    include_str!("../../../.github/workflows/ci.yml")
        .split_once("  validate-hermetic:\n")
        .unwrap()
        .1
        .split_once("\n  validate-test-checks:")
        .unwrap()
        .0
}

#[test]
fn native_hermetic_verdict_stays_required_while_full_partial_diagnostics_are_optional() {
    let lane = lane();
    assert!(
        lane.contains("run: nix develop .#ci --accept-flake-config -c just ci-validate-hermetic")
    );
    let upload = lane
        .split_once("      - name: Upload hermetic validation diagnostics\n")
        .unwrap()
        .1;
    assert!(upload.contains("id: hermetic-diagnostics"));
    assert!(upload.contains("if: always()"));
    assert!(upload.contains("continue-on-error: true"));
    assert!(upload.contains("uses: actions/upload-artifact@v7.0.1"));
    assert!(upload.contains(".xtask/diagnostics/"));
    assert!(upload.contains(".xtask/run/"));
    assert!(upload.contains("include-hidden-files: true"));
    assert!(upload.contains("if-no-files-found: warn"));
    assert!(upload.contains("retention-days: 14"));
    assert!(upload.contains("if: always() && steps.hermetic-diagnostics.outcome == 'failure'"));
    let workflow = include_str!("../../../.github/workflows/ci.yml");
    assert!(workflow.contains("hermetic=\"${{ needs.validate-hermetic.result }}\""));
    assert!(workflow.contains("test \"$hermetic\" = \"success\""));
    assert!(!lane.contains("--launcher"));
    assert!(!lane.contains("record-start"));
}

#[test]
fn failed_upload_warning_is_visible_and_does_not_replace_saved_workload_status() {
    let warning = lane()
        .split_once("      - name: Warn on hermetic diagnostics upload failure\n")
        .unwrap()
        .1;
    let command = warning.split_once("        run: |\n").unwrap().1.trim();
    for status in [0, 17] {
        let output = Command::new("bash")
            .args([
                "-c",
                &format!("status={status}; {command}; exit \"$status\""),
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(status));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("::warning::"));
        assert!(stdout.contains("workload verdict remains authoritative"));
    }
}
