use std::process::Command;

#[test]
fn run_forwarding_preserves_captured_output_and_child_status() {
    for (program, args, expected, bytes) in [
        ("printf", vec!["forwarded\\n"], 0, b"forwarded\n".as_slice()),
        ("false", vec![], 1, b"".as_slice()),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_devtool"))
            .current_dir(fixture.path())
            .args(["run", "--", program])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(expected));
        assert!(output.stderr.is_empty());
        let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(summary["exit_code"], expected);
        assert_eq!(summary["ok"], expected == 0);
        assert_eq!(
            std::fs::read(summary["stdout"]["path"].as_str().unwrap()).unwrap(),
            bytes
        );
    }
}

#[test]
fn run_forwarding_preserves_stdout_json_errors() {
    for (program, kind) in [
        ("bash", "shell_refused"),
        ("/definitely/not/a/real/program", "spawn"),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_devtool"))
            .current_dir(fixture.path())
            .args(["run", "--", program])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64));
        assert!(output.stderr.is_empty());
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["kind"], kind);
        assert!(error["error"].is_string());
    }
}
