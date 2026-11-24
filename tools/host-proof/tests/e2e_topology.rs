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
        // Each browser's shard count is its own (Chromium 3, Firefox 5).
        for (browser, shards) in [("chromium", 3), ("firefox", 5)] {
            pairs.push(serde_json::json!({"backend": backend, "browser": browser}));
            assert_eq!(
                values["lanes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|lane| lane["backend"] == backend && lane["browser"] == browser)
                    .count(),
                shards
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
fn runtime_projection_rejects_duplicate_identity_bad_shards_and_ungated_browsers() {
    let base = catalog();
    let lanes = base["lanes"].as_array().unwrap();
    let index = |identity: &str| {
        lanes
            .iter()
            .position(|lane| lane["identity"] == identity)
            .unwrap()
    };
    let shard = index("sqlite-chromium-3-of-3");
    let mut duplicate = base.clone();
    duplicate["lanes"][2]["identity"] = duplicate["lanes"][0]["identity"].clone();
    // A pair whose shards repeat one index, or miss one.
    let mut shards = base.clone();
    shards["lanes"][shard]["shardIndex"] = 1.into();
    shards["lanes"][shard]["identity"] = "sqlite-chromium-1-of-3".into();
    let mut missing_shard = base.clone();
    missing_shard["lanes"][shard]["enabled"] = false.into();
    // A lane's browser selects its one project, so it must be a gated browser.
    let mut ungated = base.clone();
    ungated["lanes"][shard]["browser"] = "webkit".into();
    ungated["lanes"][shard]["identity"] = "sqlite-webkit-3-of-3".into();
    // An enabled unsplit lane.
    let mut unsplit = base.clone();
    unsplit["lanes"][index("sqlite-chromium-unsplit")]["enabled"] = true.into();
    // Shard weights must be a well-formed string, one weight per shard, for
    // exactly the gated browsers.
    let mut malformed_weights = base.clone();
    malformed_weights["shardWeights"]["firefox"] = "34/22/22/22".into();
    // A browser's weight count must match its lanes' shard count.
    let mut miscounted_weights = base.clone();
    miscounted_weights["shardWeights"]["firefox"] = "34:33:33".into();
    // A lane's shard count must be its own browser's, not another browser's.
    let mut foreign_count = base.clone();
    foreign_count["lanes"][shard]["shardCount"] = 5.into();
    foreign_count["lanes"][shard]["identity"] = "sqlite-chromium-3-of-5".into();
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
        ungated,
        unsplit,
        malformed_weights,
        miscounted_weights,
        foreign_count,
        missing_weights,
        colliding_seed,
        zero_seed,
    ] {
        let output = project(&broken);
        assert!(!output.status.success(), "malformed catalog accepted");
    }
}

#[test]
fn runtime_projection_takes_each_browser_shard_count_from_its_weights() {
    // Retarget Firefox to four shards: drop its fifth lanes, renumber the
    // rest and give it four weights. Chromium keeps three.
    let mut catalog = catalog();
    catalog["shardWeights"]["firefox"] = "25:25:25:25".into();
    let lanes = catalog["lanes"].as_array_mut().unwrap();
    lanes.retain(|lane| !(lane["browser"] == "firefox" && lane["shardIndex"] == 5));
    for lane in lanes.iter_mut() {
        if lane["browser"] == "firefox" && lane["enabled"] == true {
            lane["shardCount"] = 4.into();
            lane["identity"] = format!(
                "{}-firefox-{}-of-4",
                lane["backend"].as_str().unwrap(),
                lane["shardIndex"]
            )
            .into();
        }
    }
    let output = project(&catalog);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let emitted: Value = serde_json::from_str(
        stdout
            .lines()
            .find_map(|line| line.strip_prefix("lanes="))
            .unwrap(),
    )
    .unwrap();
    let count = |browser: &str| {
        emitted
            .as_array()
            .unwrap()
            .iter()
            .filter(|lane| lane["browser"] == browser)
            .count()
    };
    assert_eq!((count("chromium"), count("firefox")), (6, 8));
}
