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
        for (name, script) in [
            (
                "git",
                "#!/usr/bin/env bash\nprintf '%s\\n' \"$JAUNDER_TEST_ROOT\"\n",
            ),
            (
                "nix",
                r#"#!/usr/bin/env python3
import json, os, subprocess, sys
assert sys.argv[1:4] == ['run', '.#xtask', '--']
assert os.environ.get('JAUNDER_TRACED_RUN_CONTEXT')
assert 'JAUNDER_CARGO_XTASK_INNER' not in os.environ
args = sys.argv[4:]
if args == ['nested']:
    sys.exit(subprocess.call(['cargo-xtask', 'xtask', 'leaf']))
print(json.dumps(args), flush=True)
sys.stdout.write(sys.stdin.read())
sys.stderr.write('child stderr\n')
sys.exit(73)
"#,
            ),
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
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0]["name"], "cargo-xtask");
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
    assert_eq!(spans.len(), 3);
    let outer = spans.iter().find(|event| event["name"] == "outer").unwrap();
    let child = spans
        .iter()
        .find(|event| event["args"]["parent_span_id"] == outer["args"]["span_id"])
        .unwrap();
    assert_eq!(child["name"], "cargo-xtask");
    assert!(
        spans
            .iter()
            .any(|event| event["args"]["parent_span_id"] == child["args"]["span_id"])
    );
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
        assert!(
            traces[0]["traceEvents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["ph"] == "X" && event["name"] == "cargo-xtask")
        );
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
