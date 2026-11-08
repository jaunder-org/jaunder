use anyhow::Result;
use performance::{
    Backend, Browser, DatasetProfile, FragmentEnvelope, RawSample, canonical_plan, summarize,
};
use serde_json::json;

pub fn fragment(
    profile: DatasetProfile,
    backend: Backend,
    browser: Option<Browser>,
) -> Result<FragmentEnvelope> {
    let plan = canonical_plan(profile);
    let cursors = ["public_timeline", "authenticated_timeline", "owner_history", "post_history"].map(|workload| json!({
        "workload": workload, "target_percent": 80, "matching_result_count": 100, "resolved_rank": 80,
        "cursor": if workload.ends_with("timeline") { json!({"kind": "timeline", "value": {"created_at_us": 1, "post_id": 1}}) }
            else { json!({"kind": "history", "value": {"revision_id": 1}}) }
    }));
    let identities = if browser.is_some() {
        vec![
            ("home", "initial"),
            ("app", "initial"),
            ("global_history", "initial"),
            ("global_history", "deep"),
            ("browser_post_history", "initial"),
            ("browser_revision_detail", "point"),
        ]
    } else {
        vec![
            ("public_timeline", "initial"),
            ("public_timeline", "deep"),
            ("authenticated_timeline", "initial"),
            ("authenticated_timeline", "deep"),
            ("owner_history", "initial"),
            ("owner_history", "deep"),
            ("post_history", "initial"),
            ("post_history", "deep"),
            ("revision_detail", "point"),
        ]
    };
    let mut workloads = Vec::new();
    for (workload, position) in identities {
        for frame in if browser.is_some() {
            vec!["cold"]
        } else {
            vec!["cold", "warm"]
        } {
            let count = if browser.is_some() {
                20
            } else if frame == "warm" {
                30
            } else {
                1
            };
            let samples = (1..=count)
                .map(|duration_us| RawSample { duration_us })
                .collect::<Vec<_>>();
            let deep = position == "deep" && workload != "global_history";
            workloads.push(json!({
                "key": {"result_schema_version": 1, "generator": plan.generator, "profile": profile, "workload": workload,
                    "backend": backend, "browser": browser, "build_mode": "release", "measurement_frame": frame,
                    "measurement_position": position, "sample_count": count, "page_size": if position == "point" { None } else { Some(50) },
                    "cursor_target_percent": if deep { Some(80) } else { None }, "cursor_resolved_rank": if deep { Some(80) } else { None },
                    "nix_system": "x86_64-linux", "stable_derivation_identities": [{"name": "producer", "identity": "stable"}],
                    "runner_image": "image", "runner_architecture": "x86_64", "cpu_model": "cpu", "database_version": "database",
                    "browser_version": browser.map(|_| "browser")},
                "summary": summarize(&samples)?, "samples": samples, "rows_returned": 50
            }));
        }
    }
    let producer = if browser.is_some() {
        "browser"
    } else {
        "storage"
    };
    let mut result = json!({"setup": {"producer": producer, "backend": backend, "provisioning_us": 1, "seeding_us": 1}, "workloads": workloads});
    if browser.is_some() {
        result["diagnostics"] = json!({"navigation_artifacts": (0..120).map(|i| format!("navigation-{i}.json")).collect::<Vec<_>>(),
            "trace_artifacts": (0..120).map(|i| format!("trace-{i}.zip")).collect::<Vec<_>>(), "otel_trace_artifact": "otel-traces.jsonl"});
    }
    Ok(serde_json::from_value(
        json!({"schema_version": 1, "manifest": {"schema_version": 1, "plan": plan,
        "subjects": {"username": "user", "history_post_id": 1, "revision_id": 1,
            "browser_initial_rows": {"home": 100, "app": 100, "global_history": 100, "post_history": 100}}, "cursors": cursors},
        "fragment": {"producer": producer, "result": result}}),
    )?)
}
