use xshell::Shell;

use crate::compile_cache;
use crate::result::{CommandResult, StepResult};
use crate::sh::step_with_env;

#[path = "application_fixture.rs"]
mod application_fixture;

pub fn run(sh: &Shell, result: &mut CommandResult, nextest_args: &[String]) {
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
    let application =
        match application_fixture::PreparedApplication::prepare(&sh.current_dir(), &env) {
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
