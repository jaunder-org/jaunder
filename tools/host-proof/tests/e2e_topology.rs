//! The CI matrix projection is executable data, not a second lane catalog.
use serde_json::Value;
use std::{collections::BTreeSet, path::Path, process::Command};

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
}
fn project(catalog: &Value) -> std::process::Output {
    use std::io::Write;
    let mut child = Command::new("jq")
        .args(["-r", "-f", "scripts/e2e-matrix.jq"])
        .current_dir(repo())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(catalog.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
fn catalog() -> Value {
    serde_json::from_slice(&std::fs::read(repo().join("end2end/e2e-lanes.json")).unwrap()).unwrap()
}

#[test]
fn runtime_matrix_equals_enabled_catalog_and_reconciliation_waits_for_all_producers() {
    let catalog = catalog();
    let output = project(&catalog);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let values = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            let (key, value) = line.split_once('=').unwrap();
            (
                key.to_owned(),
                serde_json::from_str::<Value>(value).unwrap(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let emitted = values["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["lane"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let enabled = catalog["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|lane| lane["enabled"] == true)
        .map(|lane| lane["identity"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(emitted, enabled);
    assert_eq!(emitted.len(), 16);
    let mut pairs = Vec::new();
    for backend in ["postgres", "sqlite"] {
        for browser in ["chromium", "firefox"] {
            pairs.push(serde_json::json!({"backend": backend, "browser": browser}));
            assert_eq!(
                values["lanes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|lane| lane["backend"] == backend && lane["browser"] == browser)
                    .count(),
                4
            );
        }
    }
    assert_eq!(values["reconciles"], serde_json::Value::Array(pairs));
    let ci: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(repo().join(".github/workflows/ci.yml")).unwrap())
            .unwrap();
    // GitHub Actions uses the YAML 1.2 string key, not a YAML 1.1 boolean.
    assert!(ci["on"].as_mapping().is_some());
    let jobs = &ci["jobs"];
    // workflow_callers.rs owns the e2e jobs' `needs` and `if` conditions.
    for name in ["e2e-lane", "e2e-reconcile"] {
        assert!(
            jobs[name]["strategy"]["matrix"]
                .as_mapping()
                .unwrap()
                .values()
                .any(|v| v
                    .as_str()
                    .is_some_and(|s| s.contains("fromJSON(needs.e2e-catalog.outputs.")))
        );
    }
    let steps = jobs["e2e-reconcile"]["steps"].as_sequence().unwrap();
    let reconcile = steps
        .iter()
        .find(|step| {
            step["run"]
                .as_str()
                .is_some_and(|s| s.contains("dev e2e-reconcile"))
        })
        .unwrap();
    assert_eq!(reconcile["if"].as_str(), Some("always()"));
}

#[test]
fn runtime_projection_rejects_duplicate_identity_bad_shards_and_foreign_projects() {
    let base = catalog();
    let lanes = base["lanes"].as_array().unwrap();
    let index = |identity: &str| {
        lanes
            .iter()
            .position(|lane| lane["identity"] == identity)
            .unwrap()
    };
    let shard = index("sqlite-chromium-4-of-4");
    let mut duplicate = base.clone();
    duplicate["lanes"][2]["identity"] = duplicate["lanes"][0]["identity"].clone();
    // A pair whose shards repeat one index, or miss one.
    let mut shards = base.clone();
    shards["lanes"][shard]["shardIndex"] = 1.into();
    shards["lanes"][shard]["identity"] = "sqlite-chromium-1-of-4".into();
    let mut missing_shard = base.clone();
    missing_shard["lanes"][shard]["enabled"] = false.into();
    // A lane selecting a project other than its browser's one, or a second
    // project beside it.
    let mut foreign = base.clone();
    foreign["lanes"][shard]["projects"] = serde_json::json!(["firefox"]);
    let mut extra_project = base.clone();
    extra_project["lanes"][shard]["projects"] = serde_json::json!(["chromium-visual", "chromium"]);
    // An enabled unsplit lane.
    let mut unsplit = base.clone();
    unsplit["lanes"][index("sqlite-chromium-unsplit")]["enabled"] = true.into();
    // Shard weights must be a well-formed string, one weight per shard, for
    // exactly the gated browsers.
    let mut malformed_weights = base.clone();
    malformed_weights["shardWeights"]["firefox"] = "34/22/22/22".into();
    let mut miscounted_weights = base.clone();
    miscounted_weights["shardWeights"]["firefox"] = "34:33:33".into();
    let mut missing_weights = base.clone();
    missing_weights["shardWeights"]
        .as_object_mut()
        .unwrap()
        .remove("chromium");
    // Seeds that expand to the same trace id, or to the invalid all-zero id.
    let mut colliding_seed = base.clone();
    colliding_seed["lanes"][1]["traceSeed"] = "11".into();
    let mut zero_seed = base.clone();
    zero_seed["lanes"][0]["traceSeed"] = "00".into();
    for broken in [
        duplicate,
        shards,
        missing_shard,
        foreign,
        extra_project,
        unsplit,
        malformed_weights,
        miscounted_weights,
        missing_weights,
        colliding_seed,
        zero_seed,
    ] {
        let output = project(&broken);
        assert!(!output.status.success(), "malformed catalog accepted");
    }
}
