use xshell::Shell;

use crate::compile_cache;
use crate::result::{CommandResult, StepResult};
use crate::sh::step_with_env;

pub fn run(sh: &Shell, result: &mut CommandResult, nextest_args: &[String]) {
    run_with_producer(
        sh,
        result,
        nextest_args,
        sh.var_os("JAUNDER_CSR_BUNDLE_DIR"),
        |sh, result| {
            let traced = crate::invocation_trace::active();
            let phase = traced.then(|| crate::invocation_trace::phase("test-local-csr-bundle"));
            let started = (!traced).then(std::time::Instant::now);
            let resolve = || -> anyhow::Result<()> {
                let admission =
                    traced.then(|| crate::invocation_trace::phase("xtask.csr-admission"));
                let admitted = crate::host_tooling::admit_untracked_inputs(
                    &sh.current_dir(),
                    ".#csrBundle.sourceAdmits",
                );
                if let Some(admission) = admission {
                    admission.finish(admitted.is_ok());
                }
                admitted?;
                let root = sh.current_dir().join(".xtask/csr-bundle");
                sh.create_dir(root.parent().unwrap())?;
                let build = traced.then(|| crate::invocation_trace::phase("xtask.csr-build"));
                let built = crate::nix_build::build_rooted(sh, "csrBundle", &root);
                if let Some(build) = build {
                    build.finish(built.is_ok());
                }
                built?;
                Ok(())
            };
            let mut step = match resolve() {
                Ok(()) => StepResult::ok("test-local-csr-bundle"),
                Err(error) => {
                    StepResult::fail("test-local-csr-bundle").detail(format!("{error:#}"))
                }
            };
            if let Some(phase) = phase {
                phase.finish(step.ok);
            }
            if let Some(started) = started {
                step = step.with_duration(started.elapsed());
            }
            result.push(step);
        },
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
    let wait = crate::invocation_trace::active()
        .then(|| crate::invocation_trace::phase("xtask.artifact-lock-wait"));
    let lock = super::host_server::host_artifact_build_lock();
    if let Some(wait) = wait {
        wait.finish(lock.is_ok());
    }
    let _build_lock = match lock {
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
            sh.current_dir().join(".xtask/csr-bundle")
        }
    };
    let validation = crate::invocation_trace::active()
        .then(|| crate::invocation_trace::phase("test-local-csr-prerequisite"));
    let validated = super::csr_prerequisite::validate(&input);
    if let Some(validation) = validation {
        validation.finish(validated.is_ok());
    }
    let root = match validated {
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
    // Nix's ordinary checkout-local out-link remains rooted. Keep the shared
    // lock through compilation/execution so peers cannot replace the active root.
    let _bundle_env = sh.push_env("JAUNDER_CSR_BUNDLE_DIR", &root);
    let (env, cache_detail) = compile_cache::cargo_compile_env();
    // Storage and seeding tests need the application worker, not their test
    // harness executable. Build it even when nextest selects only storage.
    let worker_build = step_with_env(
        sh,
        "test-local-image-worker",
        "cargo",
        &["build", "--locked", "-p", "jaunder", "--bin", "jaunder"],
        &env,
    );
    let worker_ok = worker_build.ok;
    result.push(worker_build);
    if !worker_ok {
        return;
    }
    let executable = sh.current_dir().join("target/debug/jaunder");
    let _worker_env = sh.push_env("JAUNDER_TEST_EXECUTABLE", executable);
    let args = args_for(nextest_args);
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut step = step_with_env(sh, "test-local", "cargo", &refs, &env);
    if let Some(cache_detail) = cache_detail {
        step.detail = Some(match step.detail.take() {
            Some(command_detail) => format!("{command_detail}\n{cache_detail}"),
            None => cache_detail,
        });
    }
    result.push(step);
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
    fn implicit_bundle_is_nix_selected_and_rooted_through_both_consumers() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let _serial = ORCHESTRATION.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        assert!(
            crate::git::at(temp.path())
                .args(["init", "--quiet"])
                .status()
                .unwrap()
                .success()
        );
        fs::write(temp.path().join(".git/info/exclude"), "*\n").unwrap();
        let fixture = temp.path().join("bundle");
        super::super::csr_prerequisite::tests::write_bundle(&fixture);
        let calls = temp.path().join("calls");
        for (name, script) in [
            (
                "nix",
                r#"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$CALLS"
[ "${FAIL_BUILD:-no}" = no ] || exit 37
[ "$1" = build ] || exit 31
while [ "$1" != --out-link ]; do shift; done
ln -sfn "$FIXTURE" "$2"
printf '/nix/store/fixture\n'
"#,
            ),
            (
                "cargo",
                r#"#!/bin/sh
[ -L .xtask/csr-bundle ] || exit 32
[ "$JAUNDER_CSR_BUNDLE_DIR" = "$FIXTURE" ] || exit 33
printf 'consumer %s\n' "$1" >> "$CALLS"
"#,
            ),
        ] {
            let path = temp.path().join(name);
            fs::write(&path, script).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
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
        sh.set_var("FIXTURE", &fixture);
        sh.set_var("CALLS", &calls);
        assert!(sh.var_os("JAUNDER_CSR_BUNDLE_DIR").is_none());
        let mut result = CommandResult::new("test-local");
        run(&sh, &mut result, &[]);
        assert!(result.ok, "{:?}", result.steps);
        let observed = fs::read_to_string(&calls).unwrap();
        assert!(observed.contains(".#csrBundle\n"), "{observed}");
        assert!(
            observed.ends_with("consumer build\nconsumer run\n"),
            "{observed}"
        );
        assert!(!temp.path().join("target/site").exists());

        // A prior root must not become a fallback if realization fails.
        fs::write(&calls, "").unwrap();
        sh.set_var("FAIL_BUILD", "yes");
        let mut failed = CommandResult::new("test-local");
        run(&sh, &mut failed, &[]);
        assert!(!failed.ok);
        assert!(!fs::read_to_string(&calls).unwrap().contains("consumer"));

        fs::write(&calls, "").unwrap();
        sh.set_var("FAIL_BUILD", "no");
        fs::write(fixture.join("manifest.json"), "corrupt").unwrap();
        let mut corrupt = CommandResult::new("test-local");
        run(&sh, &mut corrupt, &[]);
        assert!(!corrupt.ok);
        assert!(!fs::read_to_string(calls).unwrap().contains("consumer"));
    }

    #[test]
    #[cfg(unix)]
    fn omitted_source_blocks_realization_and_consumers() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let _serial = ORCHESTRATION.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(
                crate::git::at(temp.path())
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "--quiet"]);
        fs::write(
            temp.path().join("flake.nix"),
            r#"{
          outputs = { self }: { packages = builtins.listToAttrs (map (name: {
            inherit name;
            value.csrBundle.sourceAdmits = path: kind: path == "new.rs";
          }) [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ]); };
        }"#,
        )
        .unwrap();
        git(&["add", "flake.nix"]);
        fs::write(temp.path().join("new.rs"), "omitted source").unwrap();
        for program in ["cargo", "nix"] {
            let path = temp.path().join(program);
            fs::write(
                &path,
                "#!/bin/sh\nprintf 'unexpected subprocess' >> \"$CALLS\"\n",
            )
            .unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let sh = Shell::new().unwrap();
        sh.change_dir(temp.path());
        sh.set_var("PATH", temp.path());
        sh.set_var("CALLS", temp.path().join("calls"));
        let mut result = CommandResult::new("test-local");
        // The production resolver must refuse before invoking either producer
        // or consumers, rather than accepting an available stale bundle.
        run(&sh, &mut result, &[]);
        assert!(!result.ok);
        assert!(result.steps[0].detail.as_ref().unwrap().contains("new.rs"));
        assert!(!temp.path().join("calls").exists());
    }

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
            for fail_consumer in [false, true] {
                for prior_failure in [false, true] {
                    let temp = tempfile::tempdir().unwrap();
                    let fixture = temp.path().join("fixture");
                    super::super::csr_prerequisite::tests::write_bundle(&fixture);
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
    mkdir -p .xtask
    cp -R "$FIXTURE" .xtask/csr-bundle
elif [ "$1" = build ]; then
    printf 'worker-build\n' >> "$CALLS"
else
    printf 'consumer\n' >> "$CALLS"
    printf '%s\n' "$JAUNDER_CSR_BUNDLE_DIR" "$RUSTC_WRAPPER" "$CARGO_INCREMENTAL" "$JAUNDER_TEST_EXECUTABLE" "$@" > "$OBSERVED"
    if [ "$FAIL_CONSUMER" = yes ]; then echo 'controlled consumer failure' >&2; exit 8; fi
fi
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
                    sh.set_var("FAIL_CONSUMER", if fail_consumer { "yes" } else { "no" });
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
                        !fail_consumer && !prior_failure,
                        "{:?}",
                        result.steps
                    );
                    assert_eq!(
                        fs::read_to_string(temp.path().join("calls")).unwrap(),
                        if explicit {
                            "worker-build\nconsumer\n"
                        } else {
                            "producer\nworker-build\nconsumer\n"
                        }
                    );
                    let root = if explicit {
                        fixture
                    } else {
                        temp.path().join(".xtask/csr-bundle")
                    };
                    let observed = fs::read_to_string(temp.path().join("observed")).unwrap();
                    let lines = observed.lines().collect::<Vec<_>>();
                    assert_eq!(lines[0], root.canonicalize().unwrap().to_str().unwrap());
                    assert_eq!(&lines[1..3], &["sccache", "0"]);
                    assert_eq!(
                        std::path::Path::new(lines[3]),
                        temp.path().join("target/debug/jaunder")
                    );
                    assert_eq!(
                        &lines[4..],
                        args_for(&args)
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                    );
                    if fail_consumer {
                        assert!(
                            result
                                .steps
                                .last()
                                .unwrap()
                                .detail
                                .as_ref()
                                .unwrap()
                                .contains("controlled consumer failure")
                        );
                    }
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
            let input = temp.path().join(".xtask/csr-bundle");
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
