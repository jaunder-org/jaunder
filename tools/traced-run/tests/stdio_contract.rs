use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn captured_child_gets_null_stdin() {
    let fixture = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_traced-run"))
        .current_dir(fixture.path())
        .args(["--", "cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The input is small enough to fit in the pipe even though the supervised
    // child deliberately gets null stdin.
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"interactive input\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let parked = summary["stdout"]["path"].as_str().unwrap();
    assert!(std::fs::read(parked).unwrap().is_empty());
}

#[test]
fn runner_errors_are_stdout_json() {
    for (program, kind) in [
        ("/definitely/not/a/real/program", "spawn"),
        ("bash", "shell_refused"),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_traced-run"))
            .current_dir(fixture.path())
            .args(["--", program])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64), "{program}");
        assert!(output.stderr.is_empty(), "{program}: {:?}", output.stderr);
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["kind"], kind, "{program}");
        assert!(!error["error"].as_str().unwrap().is_empty());
        assert_eq!(error.as_object().unwrap().len(), 2);
    }
}

#[test]
fn help_and_malformed_cli_are_distinct() {
    let help = Command::new(env!("CARGO_BIN_EXE_traced-run"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .starts_with("Usage: traced-run")
    );
    for args in [
        vec!["true"],
        vec!["--"],
        vec!["--timeout", "soon", "--", "true"],
        vec!["--name", "label", "--", "true"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_traced-run"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64), "{args:?}");
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["kind"], "usage", "{args:?}");
    }
}
