//! The PR cleanup script against real Git in disposable repositories. gh and
//! cargo are fakes; never clean this checkout. The `pr-land`/`pr-cleanup`
//! fixture proofs are in `tools/dev/tests/test_pr_frontdoors.py`.

use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Command, Output},
};

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MERGE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn snapshot() -> Value {
    json!({"number":731,"state":"MERGED","baseRefName":"main","headRefName":"issue-731-example","headRefOid":HEAD,
        "mergedAt":"2026-11-08T00:00:00Z","mergeCommit":{"oid":MERGE}})
}
// Hook-exported repository redirects override -C. Scrub them on every real Git
// construction, and isolate identity/config so fixture commits cannot run hooks
// or signing configuration inherited from the managed checkout.
fn native_git(dir: &std::path::Path, args: &[&str]) -> Output {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "user.name=Cleanup fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_TEMPLATE_DIR");
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(variable);
    }
    command.env("LC_ALL", "C").output().unwrap()
}
fn native_git_ok(dir: &std::path::Path, args: &[&str]) -> String {
    let output = native_git(dir, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn disposable_native_git_cleanup_fetches_detaches_and_preserves_local_main() {
    use std::os::unix::fs::PermissionsExt;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("pr-cleanup-native-")
        .tempdir_in(root.join(".xtask/tmp"))
        .unwrap();
    let upstream = fixture.path().join("upstream");
    let checkout = fixture.path().join("checkout");
    native_git_ok(
        fixture.path(),
        &["init", "--initial-branch=main", upstream.to_str().unwrap()],
    );
    std::fs::write(upstream.join("base.txt"), "base\n").unwrap();
    native_git_ok(&upstream, &["add", "base.txt"]);
    native_git_ok(&upstream, &["commit", "-m", "fixture base"]);
    let base = native_git_ok(&upstream, &["rev-parse", "HEAD"]);
    native_git_ok(
        fixture.path(),
        &[
            "clone",
            "--no-hardlinks",
            upstream.to_str().unwrap(),
            checkout.to_str().unwrap(),
        ],
    );
    native_git_ok(&checkout, &["checkout", "-b", "issue-731-example"]);
    std::fs::write(checkout.join("feature.txt"), "feature\n").unwrap();
    native_git_ok(&checkout, &["add", "feature.txt"]);
    native_git_ok(&checkout, &["commit", "-m", "fixture feature"]);
    let head = native_git_ok(&checkout, &["rev-parse", "HEAD"]);
    native_git_ok(&checkout, &["branch", "--set-upstream-to=origin/main"]);
    // Only this filesystem-local upstream advances; checkout's local main and
    // origin/main must remain at base until the actual cleanup script fetches.
    native_git_ok(
        &upstream,
        &["fetch", checkout.to_str().unwrap(), "issue-731-example"],
    );
    native_git_ok(
        &upstream,
        &["merge", "--no-ff", "-m", "fixture merge", "FETCH_HEAD"],
    );
    let merged = native_git_ok(&upstream, &["rev-parse", "HEAD"]);
    assert_eq!(
        native_git_ok(&checkout, &["rev-parse", "origin/main"]),
        base
    );
    let mut pr = snapshot();
    pr["headRefOid"] = json!(head);
    pr["mergeCommit"]["oid"] = json!(merged);
    let pr_file = fixture.path().join("pr.json");
    std::fs::write(&pr_file, pr.to_string()).unwrap();
    let bin = fixture.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for (tool, script) in [
        (
            "gh",
            "#!/usr/bin/env bash\nset -euo pipefail\n[[ \"$*\" == 'pr view 731 --repo jaunder-org/jaunder --json number,state,baseRefName,headRefName,headRefOid,mergedAt,mergeCommit' ]]\njq -c . \"$PR_FIXTURE\"\n",
        ),
        (
            "cargo",
            "#!/usr/bin/env bash\nset -euo pipefail\n[[ \"$*\" == clean ]]\nprintf 'clean\\n' >> \"$CARGO_MARKER\"\n",
        ),
    ] {
        let path = bin.join(tool);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let marker = fixture.path().join("cargo-called");
    // No fake git is installed. The script runs only inside this fresh checkout;
    // invalid redirects are also confined to the fixture if scrubbing regresses.
    let mut command = Command::new("bash");
    command.env("LC_ALL", "C");
    command
        .arg(root.join("scripts/pr-cleanup.sh"))
        .args(["731", &head, "unmanaged"])
        .current_dir(&checkout)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("PR_FIXTURE", &pr_file)
        .env("CARGO_MARKER", &marker)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_TEMPLATE_DIR");
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(variable);
    }
    command
        .env("GIT_DIR", fixture.path().join("nonexistent"))
        .env("GIT_INDEX_FILE", fixture.path().join("foreign-index"));
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["outcome"], "cleaned");
    assert_eq!(report["head_sha"], head);
    assert_eq!(report["merge_commit"], merged);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Deleted branch issue-731-example"));
    assert_eq!(native_git_ok(&checkout, &["branch", "--show-current"]), "");
    assert_eq!(native_git_ok(&checkout, &["rev-parse", "HEAD"]), merged);
    assert_eq!(
        native_git_ok(&checkout, &["rev-parse", "origin/main"]),
        merged
    );
    assert_eq!(
        native_git_ok(&checkout, &["rev-parse", "refs/heads/main"]),
        base
    );
    assert_eq!(
        native_git(
            &checkout,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/issue-731-example"
            ]
        )
        .status
        .code(),
        Some(1)
    );
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "clean\n");
}
