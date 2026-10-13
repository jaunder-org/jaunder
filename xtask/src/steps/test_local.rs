use xshell::Shell;

use crate::compile_cache;
use crate::result::{CommandResult, StepResult};
use crate::sh::step_with_env;

#[path = "application_fixture.rs"]
mod application_fixture;

pub fn run(sh: &Shell, result: &mut CommandResult, nextest_args: &[String]) {
    run_with_producer(
        sh,
        result,
        nextest_args,
        sh.var_os("JAUNDER_CSR_BUNDLE_DIR"),
        |sh, result| super::build_csr::run(sh, result, false),
    );
}

// The collaborator owns only subprocess execution; input validation stays real.
fn run_with_producer(
    sh: &Shell,
    result: &mut CommandResult,
    nextest_args: &[String],
    explicit_root: Option<std::ffi::OsString>,
    produce: impl FnOnce(&Shell, &mut CommandResult),
) {
    let _build_lock = match super::host_server::host_artifact_build_lock() {
        Ok(lock) => lock,
        Err(error) => {
            result.push(
                StepResult::fail("test-local-csr-prerequisite").detail(format!(
                    "cannot acquire host artifact build lock: {error:#}"
                )),
            );
            return;
        }
    };
    let input = match explicit_root {
        Some(root) => sh.current_dir().join(root),
        None => {
            let producer_start = result.steps.len();
            produce(sh, result);
            if result.steps[producer_start..]
                .iter()
                .any(|step| !step.ok && !step.skipped)
            {
                return;
            }
            sh.current_dir().join("target/site")
        }
    };
    let root = match super::csr_prerequisite::validate(&input) {
        Ok(root) => root,
        Err(error) => {
            result
                .push(StepResult::fail("test-local-csr-prerequisite").detail(format!("{error:#}")));
            return;
        }
    };
    result.push(
        StepResult::ok("test-local-csr-prerequisite")
            .detail(format!("verified CSR bundle {}", root.display())),
    );
    // The shared producer can replace target/site; retain ownership until the
    // wrapped nextest command has finished all consumer compilation.
    let _bundle_env = sh.push_env("JAUNDER_CSR_BUNDLE_DIR", &root);
    let args = args_for(nextest_args);
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let (mut env, cache_detail) = compile_cache::cargo_compile_env();
    // Integration/libtest executables cannot dispatch internal application modes.
    // Prepare the real demo executable explicitly for every local product lane.
    let build = step_with_env(
        sh,
        "image-worker-fixture",
        "cargo",
        &[
            "build",
            "--locked",
            "-p",
            "test-support",
            "--bin",
            "test-support",
            "--target-dir",
            "target",
        ],
        &env,
    );
    let built = build.ok;
    result.push(build);
    if !built {
        return;
    }
    let start = std::time::Instant::now();
    let application = match application_fixture::PreparedApplication::prepare(sh, &env) {
        Ok(application) => {
            let mut step = StepResult::ok("application-fixture");
            step.duration_ms = start.elapsed().as_millis();
            result.push(step);
            application
        }
        Err(error) => {
            let mut step = StepResult::fail("application-fixture").detail(format!("{error:#}"));
            step.duration_ms = start.elapsed().as_millis();
            result.push(step);
            return;
        }
    };
    env.push((
        "JAUNDER_APPLICATION_BIN".into(),
        application.path().display().to_string(),
    ));
    env.push((
        "JAUNDER_IMAGE_WORKER_BIN".into(),
        sh.current_dir()
            .join("target/debug/test-support")
            .display()
            .to_string(),
    ));
    let mut step = step_with_env(sh, "test-local", "cargo", &refs, &env);
    if let Some(cache_detail) = cache_detail {
        step.detail = Some(match step.detail.take() {
            Some(command_detail) => format!("{command_detail}\n{cache_detail}"),
            None => cache_detail,
        });
    }
    result.push(step);
    match application.close() {
        Ok(()) => result.push(StepResult::ok("application-fixture-cleanup")),
        Err(error) => result
            .push(StepResult::fail("application-fixture-cleanup").detail(format!("{error:#}"))),
    }
}

pub fn args_for(nextest_args: &[String]) -> Vec<String> {
    let mut args = [
        "run",
        "--locked",
        "--quiet",
        "--manifest-path",
        "tools/Cargo.toml",
        "-p",
        "devtool",
        "--",
        "pg",
        "run",
        "--",
        "cargo",
        "nextest",
        "run",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    if nextest_args.is_empty() {
        args.push("--workspace".to_string());
    } else {
        args.extend(nextest_args.iter().cloned());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    static ORCHESTRATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    #[cfg(unix)]
    fn test_local_missing_explicit_bundle_blocks_consumer_without_fallback() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let _serial = ORCHESTRATION.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let cargo = temp.path().join("cargo");
        let calls = temp.path().join("calls");
        fs::write(&cargo, "#!/bin/sh\nprintf 'consumer\\n' >> \"$CALLS\"\n").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
        let sh = Shell::new().unwrap();
        sh.set_var("PATH", temp.path());
        sh.set_var("CALLS", &calls);
        sh.set_var("JAUNDER_CSR_BUNDLE_DIR", temp.path().join("missing"));
        let mut result = CommandResult::new("test-local");

        run(&sh, &mut result, &[]);

        assert!(
            !result.ok,
            "missing explicit CSR input must fail before compilation"
        );
        assert!(
            !calls.exists(),
            "neither fallback producer nor consumer may run"
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_local_producer_failure_blocks_consumer() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let _serial = ORCHESTRATION.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let cargo = temp.path().join("cargo");
        fs::write(&cargo, "#!/bin/sh\nif [ \"$1\" = producer ]; then echo 'producer failed' >&2; exit 9; fi\nexit 0\n").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
        let sh = Shell::new().unwrap();
        sh.set_var("PATH", temp.path());
        let mut result = CommandResult::new("test-local");
        run_with_producer(&sh, &mut result, &[], None, |sh, result| {
            result.push(step_with_env(
                sh,
                "controlled-producer",
                "cargo",
                &["producer"],
                &[],
            ));
        });
        assert!(
            !result.ok,
            "producer failure must block test compilation and execution"
        );
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].name, "controlled-producer");
    }

    // flock is the Linux subprocess probe for the Rust File lock contract.
    #[test]
    #[cfg(target_os = "linux")]
    fn test_local_resolves_before_consumers_and_retains_ownership_and_arguments() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let _serial = ORCHESTRATION.lock().unwrap();
        if std::env::var_os("JAUNDER_TEST_LOCAL_CHECKOUT_LOCK").is_none() {
            // A private process/cwd keeps the release observer independent of
            // other callers and parallel subprocess creation in the auxiliary suite.
            drop(super::super::host_server::host_artifact_build_lock().unwrap());
            let checkout_lock = std::env::current_dir()
                .unwrap()
                .join(".xtask/host-artifacts.lock");
            let isolated = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "steps::test_local::tests::test_local_resolves_before_consumers_and_retains_ownership_and_arguments",
                    "--nocapture",
                ])
                .current_dir(isolated.path())
                .env("JAUNDER_TEST_LOCAL_CHECKOUT_LOCK", checkout_lock)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "isolated lifecycle test failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        for explicit in [false, true] {
            for failure in ["none", "worker", "application", "nextest"] {
                for prior_failure in [false, true] {
                    let temp = tempfile::tempdir().unwrap();
                    let fixture = temp.path().join("fixture");
                    super::super::csr_prerequisite::tests::write_bundle(&fixture);
                    let profile = temp.path().join("target/debug");
                    fs::create_dir_all(profile.join("deps")).unwrap();
                    let original = profile.join("jaunder");
                    fs::write(&original, b"#!/bin/sh\nexit 0\n").unwrap();
                    fs::set_permissions(&original, fs::Permissions::from_mode(0o700)).unwrap();
                    let records = [
                        serde_json::json!({"reason":"compiler-artifact","target":{"src_path":temp.path().join("host/src/lib.rs")},"features":["default","sqlx"]}),
                        serde_json::json!({"reason":"compiler-artifact","target":{"src_path":temp.path().join("storage/src/lib.rs")},"features":["default","sqlx"]}),
                        serde_json::json!({"reason":"compiler-artifact","package_id":format!("path+{}#jaunder@0.1.0", url::Url::from_file_path(temp.path().join("server")).unwrap()),"target":{"src_path":temp.path().join("server/src/main.rs"),"name":"jaunder","kind":["bin"]},"executable":original}),
                    ];
                    fs::write(
                        temp.path().join("artifacts"),
                        records
                            .iter()
                            .map(serde_json::Value::to_string)
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )
                    .unwrap();
                    let cargo = temp.path().join("cargo");
                    // Only sanitized fixture copying and observations; no Cargo,
                    // PostgreSQL, wasm or application process is launched here.
                    fs::write(
                        &cargo,
                        r#"#!/bin/sh
flock -n -E 73 "$LOCK" -c true
status=$?
if [ "$status" -ne 73 ]; then echo "expected lock contention, got $status" >&2; exit 42; fi
if [ "$1" = producer ]; then
    printf 'producer\n' >> "$CALLS"
    mkdir -p target
    cp -R "$FIXTURE" target/site
    printf '%s\n' "$@" > "$OBSERVED-producer"
    exit 0
fi
if [ "$1" = build ]; then
    if [ "$3" = -p ]; then stage=worker; else stage=application; fi
else
    stage=nextest
fi
printf '%s\n' "$stage" >> "$CALLS"
printf '%s\n' "$PWD" "$JAUNDER_CSR_BUNDLE_DIR" "$RUSTC_WRAPPER" "$CARGO_INCREMENTAL" "$SCCACHE_BASEDIRS" "$@" > "$OBSERVED-$stage"
if [ "$stage" = nextest ]; then
    test -x "$JAUNDER_APPLICATION_BIN" || exit 43
    cmp "$JAUNDER_APPLICATION_BIN" "$ORIGINAL" || exit 44
    printf '%s\n' "$JAUNDER_APPLICATION_BIN" "$JAUNDER_IMAGE_WORKER_BIN" > "$RETAINED"
fi
if [ "$FAIL_STAGE" = "$stage" ]; then echo "controlled $stage failure" >&2; exit 8; fi
if [ "$stage" = application ]; then cat "$ARTIFACTS"; fi
"#,
                    )
                    .unwrap();
                    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
                    let sh = Shell::new().unwrap();
                    sh.change_dir(temp.path());
                    sh.set_var(
                        "PATH",
                        format!(
                            "{}:{}",
                            temp.path().display(),
                            std::env::var("PATH").unwrap()
                        ),
                    );
                    sh.set_var(
                        "LOCK",
                        std::env::current_dir()
                            .unwrap()
                            .join(".xtask/host-artifacts.lock"),
                    );
                    sh.set_var("FIXTURE", &fixture);
                    sh.set_var("CALLS", temp.path().join("calls"));
                    sh.set_var("OBSERVED", temp.path().join("observed"));
                    sh.set_var("FAIL_STAGE", failure);
                    sh.set_var("ARTIFACTS", temp.path().join("artifacts"));
                    sh.set_var("ORIGINAL", &original);
                    sh.set_var("RETAINED", temp.path().join("retained"));
                    let args = if explicit {
                        strings(&["-p", "jaunder", "--no-fail-fast"])
                    } else {
                        vec![]
                    };
                    let mut result = CommandResult::new("test-local");
                    if prior_failure {
                        result.push(StepResult::fail("unrelated-earlier-step"));
                    }
                    run_with_producer(
                        &sh,
                        &mut result,
                        &args,
                        explicit.then(|| std::ffi::OsString::from("fixture/.")),
                        |sh, result| {
                            result.push(step_with_env(
                                sh,
                                "controlled-producer",
                                "cargo",
                                &["producer"],
                                &[],
                            ))
                        },
                    );
                    assert_eq!(
                        result.ok,
                        failure == "none" && !prior_failure,
                        "{:?}",
                        result.steps
                    );
                    let stages: &[&str] = match failure {
                        "worker" => &["worker"],
                        "application" => &["worker", "application"],
                        _ => &["worker", "application", "nextest"],
                    };
                    let mut calls = if explicit {
                        String::new()
                    } else {
                        "producer\n".into()
                    };
                    for stage in stages {
                        calls.push_str(stage);
                        calls.push('\n');
                    }
                    assert_eq!(
                        fs::read_to_string(temp.path().join("calls")).unwrap(),
                        calls
                    );
                    if !explicit {
                        assert_eq!(
                            fs::read_to_string(temp.path().join("observed-producer")).unwrap(),
                            "producer\n"
                        );
                    }
                    let root = if explicit {
                        fixture
                    } else {
                        temp.path().join("target/site")
                    };
                    let (cache_env, _) = compile_cache::cargo_compile_env();
                    let basedirs = cache_env
                        .iter()
                        .find(|(key, _)| key == "SCCACHE_BASEDIRS")
                        .unwrap()
                        .1
                        .as_str();
                    for stage in stages {
                        let observed =
                            fs::read_to_string(temp.path().join(format!("observed-{stage}")))
                                .unwrap();
                        let lines = observed.lines().collect::<Vec<_>>();
                        assert_eq!(lines[0], temp.path().to_str().unwrap());
                        assert_eq!(lines[1], root.canonicalize().unwrap().to_str().unwrap());
                        assert_eq!(&lines[2..5], &["sccache", "0", basedirs]);
                        let expected = match *stage {
                            "worker" => strings(&[
                                "build",
                                "--locked",
                                "-p",
                                "test-support",
                                "--bin",
                                "test-support",
                                "--target-dir",
                                "target",
                            ]),
                            "application" => strings(&[
                                "build",
                                "--locked",
                                "--package",
                                "jaunder",
                                "--bin",
                                "jaunder",
                                "--message-format=json",
                            ]),
                            _ => args_for(&args),
                        };
                        assert_eq!(
                            &lines[5..],
                            expected.iter().map(String::as_str).collect::<Vec<_>>()
                        );
                    }
                    if failure != "none" {
                        let name = match failure {
                            "worker" => "image-worker-fixture",
                            "application" => "application-fixture",
                            _ => "test-local",
                        };
                        let failed = result.steps.iter().find(|step| step.name == name).unwrap();
                        assert!(!failed.ok);
                        assert!(
                            failed
                                .detail
                                .as_ref()
                                .unwrap()
                                .contains(&format!("controlled {failure} failure"))
                        );
                    }
                    if stages.contains(&"nextest") {
                        let retained = fs::read_to_string(temp.path().join("retained")).unwrap();
                        let paths = retained.lines().collect::<Vec<_>>();
                        assert_ne!(paths[0], original.to_str().unwrap());
                        assert_eq!(
                            std::path::Path::new(paths[0]).parent().unwrap(),
                            profile.join("deps")
                        );
                        assert!(
                            !std::path::Path::new(paths[0]).exists(),
                            "owned fixture must close after nextest success or failure"
                        );
                        assert_eq!(paths[1], profile.join("test-support").to_str().unwrap());
                        let cleanup = result.steps.last().unwrap();
                        assert_eq!(cleanup.name, "application-fixture-cleanup");
                        assert!(cleanup.ok);
                    } else {
                        assert!(!temp.path().join("retained").exists());
                    }
                    assert_eq!(fs::read(&original).unwrap(), b"#!/bin/sh\nexit 0\n");
                    assert_eq!(fs::read_dir(profile.join("deps")).unwrap().count(), 0);
                    // A cooperative caller can take the checkout lock as soon
                    // as the completed consumer releases it.
                    let peer_path = std::env::var_os("JAUNDER_TEST_LOCAL_CHECKOUT_LOCK").unwrap();
                    let peer = fs::File::open(peer_path).unwrap();
                    peer.lock().unwrap();
                    let lock = fs::File::open(
                        std::env::current_dir()
                            .unwrap()
                            .join(".xtask/host-artifacts.lock"),
                    )
                    .unwrap();
                    lock.try_lock()
                        .expect("ownership must settle after success or failure");
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_local_corrupt_explicit_or_produced_bundle_blocks_consumers() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let _serial = ORCHESTRATION.lock().unwrap();
        for explicit in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let input = temp.path().join("target/site");
            fs::create_dir_all(&input).unwrap();
            fs::write(input.join("manifest.json"), b"corrupt").unwrap();
            let cargo = temp.path().join("cargo");
            fs::write(&cargo, "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"$CALLS\"\n").unwrap();
            fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
            let sh = Shell::new().unwrap();
            sh.change_dir(temp.path());
            sh.set_var("PATH", temp.path());
            sh.set_var("CALLS", temp.path().join("calls"));
            let mut result = CommandResult::new("test-local");
            run_with_producer(
                &sh,
                &mut result,
                &[],
                explicit.then(|| input.clone().into_os_string()),
                |sh, result| {
                    result.push(step_with_env(
                        sh,
                        "controlled-producer",
                        "cargo",
                        &["producer"],
                        &[],
                    ))
                },
            );
            assert!(!result.ok);
            assert!(
                result
                    .steps
                    .last()
                    .unwrap()
                    .detail
                    .as_ref()
                    .unwrap()
                    .contains("invalid manifest JSON")
            );
            if explicit {
                assert!(
                    !temp.path().join("calls").exists(),
                    "explicit invalid input has no fallback"
                );
            } else {
                assert_eq!(
                    fs::read_to_string(temp.path().join("calls")).unwrap(),
                    "producer\n"
                );
            }
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn test_local_defaults_to_workspace_nextest_under_devtool_pg_run() {
        assert_eq!(
            args_for(&[]),
            strings(&[
                "run",
                "--locked",
                "--quiet",
                "--manifest-path",
                "tools/Cargo.toml",
                "-p",
                "devtool",
                "--",
                "pg",
                "run",
                "--",
                "cargo",
                "nextest",
                "run",
                "--workspace",
            ])
        );
    }

    #[test]
    fn test_local_forwards_explicit_nextest_args_without_workspace_default() {
        assert_eq!(
            args_for(&strings(&[
                "-p",
                "storage",
                "site_config_primitives_round_trip",
            ])),
            strings(&[
                "run",
                "--locked",
                "--quiet",
                "--manifest-path",
                "tools/Cargo.toml",
                "-p",
                "devtool",
                "--",
                "pg",
                "run",
                "--",
                "cargo",
                "nextest",
                "run",
                "-p",
                "storage",
                "site_config_primitives_round_trip",
            ])
        );
    }
}
