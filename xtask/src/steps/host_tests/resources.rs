//! Best-effort boundary evidence for native auxiliary test links. Resource
//! probes do not change Cargo's arguments or verdict; the owning runner applies
//! only the approved test-debug capacity experiment. Snapshots are not peak
//! measurements; missing tools/files are recorded rather than inferred healthy.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use xshell::Shell;

use crate::result::StepResult;
use crate::sh::step_with_env;

const OWNED_STORAGE_PATHS: &[&str] = &["target", "/nix/store"];

/// Persist each boundary separately, so a failed/killed child cannot discard
/// its pre-snapshot. The existing always-uploaded diagnostics path owns these.
pub(super) fn run(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult {
    run_with_storage(sh, name, program, args, OWNED_STORAGE_PATHS)
}

// The private path seam keeps unit fixtures local; production always supplies
// exactly OWNED_STORAGE_PATHS, never discovered/user-supplied roots.
fn run_with_storage(
    sh: &Shell,
    name: &str,
    program: &str,
    args: &[&str],
    storage_paths: &[&str],
) -> StepResult {
    let id = format!("{}-{}", timestamp(), std::process::id());
    let directory = sh
        .current_dir()
        .join(".xtask/diagnostics/auxiliary-host-tests")
        .join(id);
    let before = capture_warning(sh, &directory, name, "pre", storage_paths);
    // cargo test selects the test profile (default debug=true). Reduce only
    // this child's debug bytes; do not change dev defaults, jobs or cache policy.
    // This is a capacity experiment, not an established SIGBUS remedy.
    let env = [("CARGO_PROFILE_TEST_DEBUG".to_owned(), "0".to_owned())];
    let mut result = step_with_env(sh, name, program, args, &env);
    let after = capture_warning(sh, &directory, name, "post", storage_paths);
    for warning in [before, after].into_iter().flatten() {
        result.detail = Some(match result.detail {
            Some(detail) => format!("{detail}\n{warning}"),
            None => warning,
        });
    }
    result
}

fn capture_warning(
    sh: &Shell,
    directory: &Path,
    name: &str,
    phase: &str,
    storage_paths: &[&str],
) -> Option<String> {
    capture(sh, directory, name, phase, storage_paths)
        .err()
        .map(|error| {
            let warning = format!(
                "missing resource evidence ({phase}, {}): {error}",
                directory.display()
            );
            // Report immediately, including a failed pre-write before Cargo starts.
            eprintln!("{name}: {warning}");
            warning
        })
}

fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn capture(
    sh: &Shell,
    directory: &Path,
    name: &str,
    phase: &str,
    storage_paths: &[&str],
) -> std::io::Result<()> {
    let snapshot = snapshot(sh, storage_paths);
    std::fs::create_dir_all(directory)?;
    let path = directory.join(format!("{name}-{phase}.json"));
    // One small file per boundary, closed before continuing to the child.
    std::fs::write(path, format!("{snapshot:#}\n"))
}

fn probe(sh: &Shell, program: &str, args: &[&str]) -> Value {
    match sh.cmd(program).args(args).quiet().ignore_status().output() {
        Ok(output) => json!({
            "program": program, "args": args,
            "ok": output.status.success(), "exit_code": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
            "missing_evidence": !output.status.success(),
        }),
        Err(error) => json!({"program": program, "args": args, "ok": false,
            "exit_code": null, "missing_evidence": true, "error": error.to_string()}),
    }
}

fn read(path: &Path) -> Value {
    match std::fs::read_to_string(path) {
        Ok(text) => json!({"path": path, "ok": true, "text": text}),
        Err(error) => json!({"path": path, "ok": false,
            "missing_evidence": true, "error": error.to_string()}),
    }
}

fn memory(text: &str) -> Value {
    let mut fields = serde_json::Map::new();
    for key in [
        "MemTotal",
        "MemFree",
        "MemAvailable",
        "SwapTotal",
        "SwapFree",
    ] {
        let value = text
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}:")));
        fields.insert(
            key.into(),
            match value {
                Some(value) => json!({"value": value.trim(), "ok": true}),
                None => json!({"ok": false, "missing_evidence": true}),
            },
        );
    }
    Value::Object(fields)
}

fn cgroup_directory(membership: &str) -> Option<PathBuf> {
    let relative = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?;
    let relative = Path::new(relative).strip_prefix("/").ok()?;
    // Do not escape the known cgroup mount when a namespace reports ../ paths.
    if relative
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(Path::new("/sys/fs/cgroup").join(relative))
}

fn snapshot(sh: &Shell, storage_paths: &[&str]) -> Value {
    let filesystems: Vec<_> = [".", "xtask/target", "tools/target", "/tmp", "/nix"]
        .into_iter()
        .map(|path| {
            json!({"path": path,
                "bytes": probe(sh, "df", &["-B1", "--output=size,used,avail", "--", path]),
                "inodes": probe(sh, "df", &["--output=itotal,iused,iavail", "--", path]),
            })
        })
        .collect();
    // Retain the original auxiliary totals alongside the approved fixed-path
    // root-target/store totals. No path discovery, contents or inventory.
    let targets: Vec<_> = ["xtask/target", "tools/target"]
        .into_iter()
        .map(|path| probe(sh, "du", &["-s", "-B1", "--", path]))
        .collect();
    let owned_storage: Vec<_> = storage_paths
        .iter()
        .copied()
        .map(|path| probe(sh, "timeout", &["30s", "du", "-s", "-x", "-B1", "--", path]))
        .collect();
    let memory = match std::fs::read_to_string("/proc/meminfo") {
        Ok(text) => memory(&text),
        Err(error) => json!({"ok": false, "missing_evidence": true, "error": error.to_string()}),
    };
    let cgroup = match std::fs::read_to_string("/proc/self/cgroup") {
        Ok(text) => match cgroup_directory(&text) {
            Some(directory) => json!({"directory": directory, "files": ([
                "memory.current", "memory.max", "memory.events", "memory.peak",
                "memory.swap.current", "memory.swap.max",
            ].map(|file| read(&directory.join(file))))}),
            None => json!({"ok": false, "missing_evidence": true,
                "error": "no usable unified cgroup membership (v1/namespace unsupported)"}),
        },
        Err(error) => json!({"ok": false, "missing_evidence": true, "error": error.to_string()}),
    };
    let time = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(time) => json!({"ok": true, "unix_ns": time.as_nanos().to_string()}),
        Err(error) => json!({"ok": false, "missing_evidence": true, "error": error.to_string()}),
    };
    json!({"timestamp": time, "filesystems": filesystems,
        "native_target_allocated_bytes": targets, "owned_storage_allocated_bytes": owned_storage,
        "memory": memory, "cgroup": cgroup,
        "limitations": "pre/post only, not child peak RSS; GNU df/du/timeout and Linux proc/cgroup expected; fixed root target/store totals bounded to 30s each; target overrides and build scratch are not traversed"})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_run(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult {
        assert_eq!(OWNED_STORAGE_PATHS, ["target", "/nix/store"]);
        let store = sh.current_dir().join("fixture-nix-store");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("fixture"), "store output").unwrap();
        run_with_storage(sh, name, program, args, &["target", "fixture-nix-store"])
    }

    #[test]
    fn child_only_test_debug_override_preserves_other_inherited_settings() {
        let directory = tempfile::tempdir().unwrap();
        let sh = Shell::new().unwrap();
        sh.change_dir(directory.path());
        let _test_debug = sh.push_env("CARGO_PROFILE_TEST_DEBUG", "2");
        let _dev_debug = sh.push_env("CARGO_PROFILE_DEV_DEBUG", "2");
        let _jobs = sh.push_env("CARGO_BUILD_JOBS", "7");
        let result = fixture_run(
            &sh,
            "env-fixture",
            "sh",
            &[
                "-c",
                "test \"$CARGO_PROFILE_TEST_DEBUG\" = 0 && test \"$CARGO_PROFILE_DEV_DEBUG\" = 2 && test \"$CARGO_BUILD_JOBS\" = 7",
            ],
        );
        assert!(result.ok, "{:?}", result.detail);
        assert_eq!(sh.var("CARGO_PROFILE_TEST_DEBUG").unwrap(), "2");
    }

    #[test]
    fn bounded_probe_timeout_is_explicit_missing_evidence() {
        let sh = Shell::new().unwrap();
        let timed_out = probe(&sh, "timeout", &["0.01s", "sleep", "1"]);
        assert_eq!(timed_out["exit_code"], 124);
        assert_eq!(timed_out["ok"], false);
        assert_eq!(timed_out["missing_evidence"], true);
    }

    #[test]
    fn probe_records_actual_failure_exit_and_missing_executable() {
        let sh = Shell::new().unwrap();
        let failed = probe(&sh, "false", &[]);
        assert_eq!(failed["exit_code"], 1);
        assert_eq!(failed["missing_evidence"], true);
        let absent = probe(&sh, "/jaunder-nonexistent-resource-probe", &[]);
        assert_eq!(absent["exit_code"], Value::Null);
        assert_eq!(absent["missing_evidence"], true);
    }

    #[test]
    fn memory_reports_missing_fields_instead_of_zero() {
        let fields = memory("MemAvailable: 123 kB\nSwapFree: 0 kB\n");
        assert_eq!(fields["MemAvailable"]["value"], "123 kB");
        assert_eq!(fields["SwapFree"]["value"], "0 kB");
        assert_eq!(fields["MemTotal"]["missing_evidence"], true);
    }

    #[test]
    fn cgroup_path_is_membership_owned_and_cannot_escape_mount() {
        assert_eq!(
            cgroup_directory("0::/user.slice/job\n"),
            Some(PathBuf::from("/sys/fs/cgroup/user.slice/job"))
        );
        assert_eq!(
            cgroup_directory("0::/\n"),
            Some(PathBuf::from("/sys/fs/cgroup"))
        );
        assert_eq!(cgroup_directory("0::/../../other\n"), None);
        assert_eq!(cgroup_directory("4:memory:/job\n"), None);
    }

    #[test]
    fn real_boundary_capture_survives_failed_child_and_keeps_both_verdicts() {
        let directory = tempfile::tempdir().unwrap();
        let sh = Shell::new().unwrap();
        sh.change_dir(directory.path());
        assert!(!fixture_run(&sh, "failed", "false", &[]).ok);
        assert!(fixture_run(&sh, "passed", "true", &[]).ok);
        let root = directory
            .path()
            .join(".xtask/diagnostics/auxiliary-host-tests");
        let mut snapshots = 0;
        for entry in std::fs::read_dir(root).unwrap() {
            for entry in std::fs::read_dir(entry.unwrap().path()).unwrap() {
                let snapshot: Value =
                    serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap();
                assert_eq!(
                    snapshot["native_target_allocated_bytes"][0]["missing_evidence"],
                    true
                );
                let totals = snapshot["owned_storage_allocated_bytes"]
                    .as_array()
                    .unwrap();
                assert_eq!(totals.len(), 2);
                for (total, path) in totals.iter().zip(["target", "fixture-nix-store"]) {
                    assert_eq!(total["program"], "timeout");
                    assert_eq!(total["args"][0], "30s");
                    assert_eq!(total["args"][6], path);
                }
                snapshots += 1;
            }
        }
        assert_eq!(snapshots, 4);
    }

    #[test]
    fn capture_write_failure_is_missing_evidence_not_a_changed_child_verdict() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(".xtask"), "not a directory").unwrap();
        let sh = Shell::new().unwrap();
        sh.change_dir(directory.path());
        let passed = fixture_run(&sh, "passed", "true", &[]);
        let failed = fixture_run(&sh, "failed", "false", &[]);
        assert!(passed.ok);
        assert!(!failed.ok);
        assert!(passed.detail.unwrap().contains("missing resource evidence"));
        assert!(failed.detail.unwrap().contains("missing resource evidence"));
    }
}
