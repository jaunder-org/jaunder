//! The runner merges monotonic fragments written by nested instrumentation.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn runner_merges_monotonic_fragments_from_nested_instrumentation() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("write-fragment.py");
    fs::write(
        &program,
        r#"#!/usr/bin/env python3
import json, os
context = json.loads(os.environ['JAUNDER_TRACED_RUN_CONTEXT'])
event = {'name':'inner.phase','cat':'xtask_trace','ph':'X','pid':os.getpid(),'tid':os.getpid(),'ts':42.0,'dur':7.0}
with open(os.path.join(context['trace_dir'], 'fragment-fixture.jsonl'), 'w') as output:
    output.write(json.dumps(event) + '\n')
"#,
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_traced-run"))
        .args(["--name", "outer", "--"])
        .arg(&program)
        .current_dir(dir.path())
        .env_remove("JAUNDER_TRACED_RUN_CONTEXT")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let traces: Vec<serde_json::Value> = fs::read_dir(dir.path().join(".xtask/run"))
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".trace.json"))
        .map(|entry| serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap())
        .collect();
    assert_eq!(traces.len(), 1);
    let events = traces[0]["traceEvents"].as_array().unwrap();
    assert!(events.iter().any(|event| event["name"] == "outer"));
    let fragment = events
        .iter()
        .find(|event| event["name"] == "inner.phase")
        .unwrap();
    assert_eq!(fragment["ts"], 42.0);
    assert_eq!(fragment["ph"], "X");
}
