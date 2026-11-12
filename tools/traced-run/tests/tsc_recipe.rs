//! Execute the maintained tsc owner through Just, substituting only fixture leaves.
//! These isolated fixtures are not PATH interception or the real workflow proof.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn recipe_fixture(root: &Path, recorder: &str) -> PathBuf {
    let source = include_str!("../../../Justfile");
    assert!(
        source.contains("static := \"just --justfile recipes/static.just --working-directory .\"")
    );
    assert!(source.contains("{{static}} tsc {{quote(mode)}}"));
    let recipe = source
        .split_once("tsc mode=\"verify\":")
        .unwrap()
        .1
        .split_once("\n# The host builds the adapter")
        .unwrap()
        .0;
    let setup = root.join("setup");
    std::fs::write(
        &setup,
        r#"#!/usr/bin/env bash
printf 'setup %s\n' "$*" >> order
printf 'setup-out\n'
printf 'setup-err\n' >&2
if [ "$EXPECT_START" = yes ]; then
  test -n "$(find .xtask/run -name start.jsonl -print -quit)" || exit 99
fi
exit "$SETUP_CODE"
"#,
    )
    .unwrap();
    let request = root.join("request");
    std::fs::write(
        &request,
        r#"#!/usr/bin/env bash
printf 'request %s\n' "$*" >> order
printf 'request-out\n'
printf 'request-err\n' >&2
if [ "$INTERRUPT_OWNER" = yes ]; then kill -TERM "$PPID"; fi
exit "$REQUEST_CODE"
"#,
    )
    .unwrap();
    let recipe = recipe
        .replace(
            "setup-e2e-node-modules",
            &format!("bash '{}'", setup.display()),
        )
        .replace("traced-run", recorder);
    let file = root.join("Justfile");
    std::fs::write(
        &file,
        format!(
            "static := \"bash '{}'\"\ntsc mode=\"verify\":{recipe}",
            request.display()
        ),
    )
    .unwrap();
    file
}

fn run(
    root: &Path,
    mode: &str,
    setup: i32,
    request: i32,
    recorder: &str,
    expect_start: bool,
    interrupt: bool,
) -> Output {
    let justfile = recipe_fixture(root, recorder);
    Command::new("just")
        .args(["--justfile"])
        .arg(justfile)
        .args(["--working-directory"])
        .arg(root)
        .args(["tsc", mode])
        .env("E2E_TYPES_NODE_MODULES", "types-pin")
        .env("E2E_PLAYWRIGHT_TEST", "playwright-pin")
        .env("SETUP_CODE", setup.to_string())
        .env("REQUEST_CODE", request.to_string())
        .env("EXPECT_START", if expect_start { "yes" } else { "no" })
        .env("INTERRUPT_OWNER", if interrupt { "yes" } else { "no" })
        .env_remove("JAUNDER_TRACED_RUN_CONTEXT")
        .output()
        .unwrap()
}

fn fragments(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root.join(".xtask/run"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "d"))
        .collect()
}

#[test]
fn public_just_owner_records_before_setup_and_preserves_mode_arguments_streams_and_status() {
    for (mode, setup, request, expected, boundary) in [
        ("verify", 0, 0, 0, "none"),
        ("fix", 0, 17, 17, "request"),
        ("verify", 19, 0, 19, "setup"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let output = run(
            root.path(),
            mode,
            setup,
            request,
            env!("CARGO_BIN_EXE_traced-run"),
            true,
            false,
        );
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
        let order = std::fs::read_to_string(root.path().join("order")).unwrap();
        if setup == 0 {
            assert_eq!(
                order,
                if mode == "fix" {
                    "setup types-pin playwright-pin\nrequest tsc fix\n"
                } else {
                    "setup types-pin playwright-pin\nrequest tsc verify\n"
                }
            );
            assert_eq!(output.stdout, b"setup-out\nrequest-out\n");
        } else {
            assert_eq!(order, "setup types-pin playwright-pin\n");
            assert_eq!(output.stdout, b"setup-out\n");
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("setup-err"));
        assert_eq!(stderr.contains("request-err"), setup == 0);
        let records = fragments(root.path());
        assert_eq!(records.len(), 1);
        let finish: serde_json::Value =
            serde_json::from_slice(&std::fs::read(records[0].join("finish.jsonl")).unwrap())
                .unwrap();
        assert_eq!(finish["args"]["exit_code"], expected);
        assert_eq!(finish["args"]["boundary"], boundary);
    }
}

#[test]
fn recorder_unavailable_or_failed_never_gates_work_or_replaces_its_saved_result() {
    for failure in ["missing", "start-sink", "malformed-handle", "finish-sink"] {
        for status in [0, 17] {
            let root = tempfile::tempdir().unwrap();
            let recorder = match failure {
                "missing" => "/missing/recorder".to_owned(),
                "malformed-handle" => {
                    let path = root.path().join("bad-recorder");
                    std::fs::write(&path, "#!/usr/bin/env bash\nif [ \"$1\" = record-start ]; then printf '../invalid\\n'; else exit 64; fi\n").unwrap();
                    format!("bash '{}'", path.display())
                }
                _ => env!("CARGO_BIN_EXE_traced-run").to_owned(),
            };
            if failure == "start-sink" {
                std::fs::create_dir(root.path().join(".xtask")).unwrap();
                std::fs::write(root.path().join(".xtask/run"), "blocked").unwrap();
            }
            // A fixed fixture recorder delegates to the real CLI, then blocks publication.
            let recorder = if failure == "finish-sink" {
                let path = root.path().join("blocked-finish");
                std::fs::write(&path, format!("#!/usr/bin/env bash\nif [ \"$1\" = record-finish ]; then mkdir \".xtask/run/$2.trace.json\"; fi\nexec '{}' \"$@\"\n", env!("CARGO_BIN_EXE_traced-run"))).unwrap();
                format!("bash '{}'", path.display())
            } else {
                recorder
            };
            let output = run(root.path(), "verify", 0, status, &recorder, false, false);
            assert_eq!(output.status.code(), Some(status), "{failure}: {output:?}");
            assert_eq!(
                std::fs::read_to_string(root.path().join("order")).unwrap(),
                "setup types-pin playwright-pin\nrequest tsc verify\n"
            );
            assert_eq!(output.stdout, b"setup-out\nrequest-out\n");
        }
    }
}

#[test]
fn interrupted_owner_leaves_start_unknown_without_new_signal_handling() {
    let root = tempfile::tempdir().unwrap();
    let output = run(
        root.path(),
        "verify",
        0,
        0,
        env!("CARGO_BIN_EXE_traced-run"),
        true,
        true,
    );
    assert!(!output.status.success());
    let records = fragments(root.path());
    assert_eq!(records.len(), 1);
    assert!(!records[0].join("finish.jsonl").exists());
    let begin: serde_json::Value =
        serde_json::from_slice(&std::fs::read(records[0].join("start.jsonl")).unwrap()).unwrap();
    assert_eq!(begin["args"]["outcome"], "unknown");
    assert!(begin["args"].get("signal").is_none());
}
