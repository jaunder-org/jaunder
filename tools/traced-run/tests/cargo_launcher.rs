//! Exercise the real launcher and runner without realizing Nix packages.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::{Command, Stdio};

struct Fixture {
    dir: tempfile::TempDir,
    path: std::ffi::OsString,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        symlink(env!("CARGO_BIN_EXE_traced-run"), bin.join("traced-run")).unwrap();
        symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/cargo-xtask"),
            bin.join("cargo-xtask"),
        )
        .unwrap();
        let store = dir.path().join("store");
        fs::create_dir_all(store.join("bin")).unwrap();
        let xtask = store.join("bin/xtask");
        fs::write(
            &xtask,
            r#"#!/usr/bin/env python3
import json
import os
import subprocess
import sys

context = json.loads(os.environ['JAUNDER_TRACED_RUN_CONTEXT'])
assert context['parent_name'] == 'xtask-execute'
assert isinstance(context['parent_flow_id'], int)
assert os.getcwd() == os.environ['JAUNDER_TEST_ROOT']

args = sys.argv[1:]
if args == ['nested']:
    flow_id = '%016x' % context['parent_flow_id']
    timestamp = 42.0
    workload_args = {
        'subcommand': 'test-local',
        'package': 'storage',
        'test_selector': 'site_config_primitives_round_trip',
    }
    events = [
        {
            'name': 'xtask.command', 'cat': 'xtask_trace', 'ph': 'B',
            'pid': os.getpid(), 'tid': os.getpid(), 'ts': timestamp,
            'args': workload_args,
        },
        {
            'name': 'recorded child entry', 'cat': 'xtask_trace',
            'ph': 'i', 's': 't', 'pid': context['parent_pid'],
            'tid': context['parent_pid'], 'ts': timestamp,
            'args': {
                'parent_span_id': context['parent_span_id'],
                'child_name': 'xtask.command',
            },
        },
        {
            'name': 'xtask-execute-to-command', 'cat': 'xtask_trace',
            'ph': 's', 'pid': context['parent_pid'],
            'tid': context['parent_pid'], 'ts': timestamp,
            'id': flow_id,
            'args': {'parent_span_id': context['parent_span_id']},
        },
        {
            'name': 'xtask-execute-to-command', 'cat': 'xtask_trace',
            'ph': 'f', 'pid': os.getpid(), 'tid': os.getpid(),
            'ts': timestamp, 'id': flow_id, 'bp': 'e',
        },
        {
            'name': 'xtask.command', 'cat': 'xtask_trace', 'ph': 'E',
            'pid': os.getpid(), 'tid': os.getpid(), 'ts': timestamp + 1,
        },
    ]
    fragment = os.path.join(context['trace_dir'], 'fragment-workload.jsonl')
    with open(fragment, 'w') as output:
        output.write(''.join(json.dumps(event) + '\n' for event in events))
    sys.exit(subprocess.call(['cargo-xtask', 'xtask', 'leaf']))

print(json.dumps(args), flush=True)
sys.stdout.write(sys.stdin.read())
sys.stderr.write('child stderr\n')
sys.exit(73)
"#,
        )
        .unwrap();
        fs::set_permissions(&xtask, fs::Permissions::from_mode(0o755)).unwrap();
        let nix_script = format!(
            "#!/usr/bin/env python3\nimport os, sys\nassert sys.argv[1:] == ['build', '.#xtask', '--no-link', '--print-out-paths']\nassert os.environ.get('JAUNDER_TRACED_RUN_CONTEXT')\nassert os.getcwd() == os.environ['JAUNDER_TEST_ROOT']\nprint({:?})\n",
            store.display().to_string()
        );
        for (name, script) in [
            (
                "git",
                "#!/usr/bin/env bash\nprintf '%s\\n' \"$JAUNDER_TEST_ROOT\"\n",
            ),
            ("nix", nix_script.as_str()),
        ] {
            let file = bin.join(name);
            fs::write(&file, script).unwrap();
            fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        Self {
            dir,
            path: std::env::join_paths(paths).unwrap(),
        }
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(self.dir.path())
            .env("PATH", &self.path)
            .env("JAUNDER_TEST_ROOT", self.dir.path())
            .env_remove("JAUNDER_TRACED_RUN_CONTEXT");
        command
    }

    fn traces(&self) -> Vec<serde_json::Value> {
        fs::read_dir(self.dir.path().join(".xtask/run"))
            .unwrap()
            .map(Result::unwrap)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".trace.json"))
            .map(|entry| serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap())
            .collect()
    }
}

#[test]
fn direct_launcher_preserves_arguments_stdin_streams_status_and_later_roots() {
    use std::io::Write;
    let fixture = Fixture::new();
    for _ in 0..2 {
        let mut child = fixture
            .command("cargo-xtask")
            .args(["xtask", "", "two words", "--inner", "xtask", "a\nb"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"stdin bytes\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(73));
        assert_eq!(
            output.stdout,
            b"[\"\", \"two words\", \"--inner\", \"xtask\", \"a\\nb\"]\nstdin bytes\n"
        );
        assert_eq!(output.stderr, b"child stderr\n");
    }
    let traces = fixture.traces();
    assert_eq!(
        traces.len(),
        2,
        "later independent launches create fresh roots"
    );
    for trace in traces {
        let spans: Vec<_> = trace["traceEvents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["ph"] == "X")
            .collect();
        assert_eq!(spans.len(), 3);
        for name in ["cargo-xtask", "xtask-acquire", "xtask-execute"] {
            assert!(
                spans.iter().any(|event| event["name"] == name),
                "missing {name}"
            );
        }
    }
}

#[test]
fn wrapped_nested_launcher_keeps_one_root_and_each_launcher_span() {
    let fixture = Fixture::new();
    let output = fixture
        .command(env!("CARGO_BIN_EXE_traced-run"))
        .args(["--name", "outer", "--", "cargo-xtask", "xtask", "nested"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(73));
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        fs::read(summary["stdout"]["path"].as_str().unwrap()).unwrap(),
        b"[\"leaf\"]\n"
    );
    let traces = fixture.traces();
    assert_eq!(traces.len(), 1);
    let spans: Vec<_> = traces[0]["traceEvents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["ph"] == "X")
        .collect();
    assert_eq!(spans.len(), 7);
    let workload_execute = spans
        .iter()
        .find(|event| event["name"] == "xtask-execute" && event["args"]["package"] == "storage")
        .unwrap();
    let workload_root = spans
        .iter()
        .find(|event| {
            event["name"] == "cargo-xtask"
                && event["args"]["test_selector"] == "site_config_primitives_round_trip"
        })
        .unwrap();
    for span in [workload_execute, workload_root] {
        assert_eq!(span["args"]["subcommand"], "test-local");
        assert_eq!(span["args"]["package"], "storage");
        assert_eq!(
            span["args"]["test_selector"],
            "site_config_primitives_round_trip"
        );
    }
    assert_eq!(
        workload_execute["args"]["parent_span_id"],
        workload_root["args"]["span_id"]
    );
    let events = traces[0]["traceEvents"].as_array().unwrap();
    assert_eq!(workload_execute["pid"], workload_root["pid"]);
    assert_eq!(workload_execute["tid"], workload_root["tid"]);
    assert!(
        !events.iter().any(|event| {
            event["name"] == "xtask.command" || event["name"] == "xtask-execute-to-command"
        }),
        "folded wrappers and arrows are presentation-only omissions"
    );
    assert!(
        events.iter().any(|event| event["ph"] == "s"),
        "unfolded nested-launch flows remain"
    );
    let outer = spans.iter().find(|event| event["name"] == "outer").unwrap();
    let child = spans
        .iter()
        .find(|event| event["args"]["parent_span_id"] == outer["args"]["span_id"])
        .unwrap();
    assert_eq!(child["name"], "cargo-xtask");
    for name in ["xtask-acquire", "xtask-execute"] {
        assert!(spans.iter().any(|event| {
            event["name"] == name && event["args"]["parent_span_id"] == child["args"]["span_id"]
        }));
    }
}

#[test]
fn runner_merges_monotonic_fragments_from_nested_instrumentation() {
    let fixture = Fixture::new();
    let program = fixture.dir.path().join("write-fragment.py");
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
    let output = fixture
        .command(env!("CARGO_BIN_EXE_traced-run"))
        .args(["--name", "outer", "--"])
        .arg(program)
        .output()
        .unwrap();
    assert!(output.status.success());
    let traces = fixture.traces();
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

#[test]
fn discovery_and_nix_failures_are_already_supervised() {
    for program in ["git", "nix"] {
        let fixture = Fixture::new();
        fs::write(
            fixture.dir.path().join("bin").join(program),
            "#!/usr/bin/env bash\nprintf 'early failure\\n' >&2\nexit 71\n",
        )
        .unwrap();
        let output = fixture
            .command("cargo-xtask")
            .arg("xtask")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(71));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"early failure\n");
        let traces = fixture.traces();
        assert_eq!(traces.len(), 1);
        let spans: Vec<_> = traces[0]["traceEvents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["ph"] == "X")
            .collect();
        let launcher = spans
            .iter()
            .find(|event| event["name"] == "cargo-xtask")
            .unwrap();
        if program == "nix" {
            let acquisition = spans
                .iter()
                .find(|event| event["name"] == "xtask-acquire")
                .unwrap();
            assert_eq!(
                acquisition["args"]["parent_span_id"],
                launcher["args"]["span_id"]
            );
            assert!(!spans.iter().any(|event| event["name"] == "xtask-execute"));
        }
    }
}

#[test]
fn generic_runner_does_not_inject_launcher_state_by_basename() {
    let fixture = Fixture::new();
    let program = fixture.dir.path().join("cargo-xtask");
    fs::write(&program, "#!/usr/bin/env python3\nimport os\nassert 'JAUNDER_CARGO_XTASK_INNER' not in os.environ\nassert os.environ.get('JAUNDER_TRACED_RUN_CONTEXT')\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let output = fixture
        .command(env!("CARGO_BIN_EXE_traced-run"))
        .args(["--launcher", "--name", "generic", "--"])
        .arg(program)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn malformed_inner_entry_is_rejected_before_discovery() {
    let fixture = Fixture::new();
    for args in [
        vec!["--inner"],
        vec!["--inner", "wrong"],
        vec!["wrong"],
        vec![],
    ] {
        let output = fixture.command("cargo-xtask").args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cargo-xtask:"));
    }
    assert!(!fixture.dir.path().join(".xtask/run").exists());
}
