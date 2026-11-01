use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn launcher_inherits_stdin_while_captured_mode_keeps_null_stdin() {
    for launcher in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_traced-run"));
        command
            .env_remove("JAUNDER_TRACED_RUN_CONTEXT")
            .current_dir(fixture.path());
        if launcher {
            command.arg("--launcher");
        }
        let mut child = command
            .args(["--name", "stdin-contract", "--", "cat"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The input is small enough to fit in the pipe even when captured mode
        // deliberately gives the supervised child null stdin.
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"interactive input\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        if launcher {
            assert_eq!(output.stdout, b"interactive input\n");
        } else {
            let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let parked = summary["stdout"]["path"].as_str().unwrap();
            assert!(std::fs::read(parked).unwrap().is_empty());
        }
    }
}

#[test]
fn named_captured_errors_are_stdout_json_and_launcher_errors_are_stderr() {
    for launcher in [false, true] {
        for (name, program, kind) in [
            ("spawn-error", "/definitely/not/a/real/program", "spawn"),
            ("refusal", "bash", "shell_refused"),
            ("invalid name", "true", "usage"),
        ] {
            let fixture = tempfile::tempdir().unwrap();
            let mut command = Command::new(env!("CARGO_BIN_EXE_traced-run"));
            command
                .env_remove("JAUNDER_TRACED_RUN_CONTEXT")
                .current_dir(fixture.path());
            if launcher {
                command.arg("--launcher");
            }
            let output = command
                .args(["--name", name, "--", program])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(64), "{launcher}: {name}");
            if launcher {
                assert!(output.stdout.is_empty(), "{name}");
                assert!(!output.stderr.is_empty(), "{name}");
            } else {
                assert!(output.stderr.is_empty(), "{name}: {:?}", output.stderr);
                let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(error["kind"], kind, "{name}");
                assert!(!error["error"].as_str().unwrap().is_empty());
                assert_eq!(error.as_object().unwrap().len(), 2);
            }
        }
    }
}
