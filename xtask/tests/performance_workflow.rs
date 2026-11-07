use anyhow::{Context, Result};
use serde_yaml::Value;
use std::process::Command;

#[test]
fn workflow_uses_native_just_and_github_failure_guards_for_evidence_retention() -> Result<()> {
    let workflow: Value =
        serde_yaml::from_str(include_str!("../../.github/workflows/performance.yml"))?;
    let steps = workflow["jobs"]["performance"]["steps"]
        .as_sequence()
        .context("performance steps")?;
    let find = |name: &str| {
        steps
            .iter()
            .position(|step| step["name"].as_str() == Some(name))
            .context("named workflow step")
    };
    let measure = find("Measure canonical medium matrix")?;
    let compare = find("Compare committed baseline")?;
    let upload = find("Upload performance result")?;
    assert!(measure < compare && compare < upload);
    assert_eq!(
        steps[measure]["run"].as_str(),
        Some("nix develop .#ci --accept-flake-config -c just perf medium")
    );
    assert_eq!(
        steps[compare]["run"].as_str(),
        Some(
            "nix develop .#ci --accept-flake-config -c just perf-compare .xtask/performance/performance-result-v1.json"
        )
    );
    // GitHub owns execution conditions: a failed required measurement must not
    // be continued or converted into a successful comparison prerequisite.
    assert!(steps[measure].get("continue-on-error").is_none());
    assert!(steps[compare].get("continue-on-error").is_none());
    assert_eq!(
        steps[compare]["if"].as_str(),
        Some("success() && hashFiles('tools/performance/baseline-v1.json') != ''")
    );
    assert_eq!(steps[upload]["if"].as_str(), Some("always()"));
    assert!(
        steps[upload]["uses"]
            .as_str()
            .context("upload action")?
            .starts_with("actions/upload-artifact@")
    );
    assert_eq!(
        steps[upload]["with"]["name"].as_str(),
        Some("performance-result")
    );
    assert_eq!(
        steps[upload]["with"]["path"].as_str(),
        Some(".xtask/performance/")
    );
    assert_eq!(
        steps[upload]["with"]["include-hidden-files"].as_bool(),
        Some(true)
    );
    assert_eq!(
        steps[upload]["with"]["if-no-files-found"].as_str(),
        Some("error")
    );
    for step in steps {
        if let Some(run) = step["run"].as_str() {
            assert!(!run.contains("cargo xtask perf"));
            assert!(!run.contains("perf-import-baseline"));
        }
    }
    Ok(())
}

#[test]
fn retired_performance_commands_are_absent_from_the_compiled_xtask_frontdoor() -> Result<()> {
    let help = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("--help")
        .output()?;
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout)?;
    assert!(
        !text
            .lines()
            .any(|line| line.trim_start().starts_with("perf "))
    );
    for arguments in [
        vec!["perf", "--help"],
        vec!["perf", "small"],
        vec!["perf", "compare", "--result", "result.json"],
        vec!["perf", "import-baseline", "42"],
        vec!["performance", "--help"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(arguments)
            .output()?;
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
    }
    Ok(())
}
