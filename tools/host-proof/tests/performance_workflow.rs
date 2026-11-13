use anyhow::{Context, Result};
use serde_yaml::Value;

#[test]
fn workflow_uses_native_just_and_github_failure_guards_for_evidence_retention() -> Result<()> {
    let workflow: Value =
        serde_yaml::from_str(include_str!("../../../.github/workflows/performance.yml"))?;
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
