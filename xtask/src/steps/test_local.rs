use xshell::Shell;

use crate::compile_cache;
use crate::result::CommandResult;
use crate::sh::step_with_env;

pub fn run(sh: &Shell, result: &mut CommandResult, nextest_args: &[String]) {
    let args = args_for(nextest_args);
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let (env, cache_detail) = compile_cache::cargo_compile_env();
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
