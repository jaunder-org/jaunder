//! Execute the evaluated shared devShell hooks without realizing their tool matrix.
//! Provisioning is intercepted only to observe its startup environment; Git,
//! Bash, Node and filesystem behavior run for real.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

fn hooks() -> &'static BTreeMap<String, String> {
    static HOOKS: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    HOOKS.get_or_init(|| {
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        // Select the Git-backed flake so ignored build/test outputs are not
        // copied into Nix's source snapshot. Project only the runtime hooks,
        // without forcing the unrelated shell tool derivations.
        let output = Command::new("nix")
            .args([
                "eval", "--impure", "--json", ".#devShells", "--apply",
                "all: let shells = all.${builtins.currentSystem}; in builtins.mapAttrs (_: shell: shell.shellHook) { inherit (shells) default ci mutants; }",
            ])
            .current_dir(checkout)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    })
}

fn owned_fixture() -> tempfile::TempDir {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let artifacts = checkout.join(".xtask/issue-1811");
    fs::create_dir_all(&artifacts).unwrap();
    tempfile::tempdir_in(artifacts).unwrap()
}

fn repository(parent: &Path, name: &str) -> PathBuf {
    let root = parent.join(name);
    fs::create_dir(&root).unwrap();
    let output = xtask::git::at(&root)
        .args(["init", "--quiet"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    root
}

fn enter(hook: &str, cwd: &Path, parent: &Path, record: &Path, body: &str) -> Output {
    let script = format!(
        "devtool() {{ printf '%s\\n' \"$TMPDIR\" > \"$JAUNDER_TEST_PROVISION_RECORD\"; }}\n\
         eval \"$JAUNDER_TEST_SHELL_HOOK\"\n{body}"
    );
    let mut command = Command::new("bash");
    // Fixture Git discovery must not inherit hook redirects or escape to the
    // real checkout when deliberately testing a directory outside any repo.
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(key);
    }
    command
        .args(["--noprofile", "--norc", "-c", &script])
        .current_dir(cwd)
        .env("GIT_CEILING_DIRECTORIES", parent)
        .env("TMPDIR", parent)
        .env("JAUNDER_TEST_SHELL_HOOK", hook)
        .env("JAUNDER_TEST_PROVISION_RECORD", record)
        .env("JAUNDER_TEST_PARENT", parent)
        .output()
        .unwrap()
}

#[test]
fn shared_shell_entry_replaces_inherited_tmpdir_before_provisioning() {
    let owned = owned_fixture();
    for (name, hook) in hooks() {
        let root = repository(owned.path(), &format!("{name} checkout"));
        let nested = root.join("nested directory");
        fs::create_dir(&nested).unwrap();
        let record = owned.path().join(format!("{name}-provision"));
        let output = enter(
            hook,
            &nested,
            owned.path(),
            &record,
            "printf 'shell=%s\\n' \"$TMPDIR\"\nnode -p 'require(\"os\").tmpdir()'\nbash -c 'printenv TMPDIR; bash -c \"printenv TMPDIR\"'",
        );
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = root.join(".xtask/tmp");
        assert!(expected.is_absolute());
        assert!(expected.is_dir(), "{name}: shell must create TMPDIR");
        assert_eq!(
            fs::read_dir(&expected).unwrap().count(),
            0,
            "usability probe must be removed by its startup owner"
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("shell={}\n{0}\n{0}\n{0}\n", expected.display())
        );
        assert_eq!(
            fs::read_to_string(record).unwrap(),
            format!("{}\n", expected.display())
        );
        println!(
            "{name}: evaluated hook creates checkout-local TMPDIR before provisioning; direct/child/grandchild inherit it"
        );
    }
}

#[test]
fn shell_entry_binding_survives_cd_and_allows_later_command_override() {
    for (name, hook) in hooks() {
        let owned = owned_fixture();
        let a = repository(owned.path(), "checkout A");
        let b = repository(owned.path(), "checkout B");
        let override_dir = owned.path().join("override");
        fs::create_dir(&override_dir).unwrap();
        let record = owned.path().join("provision");
        let output = enter(
            hook,
            &a,
            owned.path(),
            &record,
            r#"
TMPDIR="$JAUNDER_TEST_PARENT/override" node -p 'require("os").tmpdir()'
printenv TMPDIR
cd "$JAUNDER_TEST_PARENT/checkout B"
printenv TMPDIR
"#,
        );
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let a_tmp = a.join(".xtask/tmp");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "{}\n{}\n{}\n",
                override_dir.display(),
                a_tmp.display(),
                a_tmp.display()
            )
        );
        assert!(
            !b.join(".xtask/tmp").exists(),
            "cd must not create a new temp root"
        );
        let output = enter(hook, &b, owned.path(), &record, "printenv TMPDIR");
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{}\n", b.join(".xtask/tmp").display())
        );
        assert_eq!(
            fs::read_to_string(record).unwrap(),
            format!("{}\n", b.join(".xtask/tmp").display())
        );
        println!("{name}: command override wins; cd retains A; fresh entry binds to B");
    }
}

#[test]
fn invalid_checkout_or_temp_directory_stops_startup_and_command() {
    use std::os::unix::fs::PermissionsExt;

    for (name, hook) in hooks() {
        for failure in ["no-checkout", "blocked", "unusable"] {
            let owned = owned_fixture();
            let root = if failure == "no-checkout" {
                let root = owned.path().join("not a repository");
                fs::create_dir(&root).unwrap();
                root
            } else {
                repository(owned.path(), "checkout")
            };
            let tmpdir = root.join(".xtask/tmp");
            if failure == "blocked" {
                fs::create_dir(root.join(".xtask")).unwrap();
                fs::write(&tmpdir, "caller-owned obstruction").unwrap();
            } else if failure == "unusable" {
                fs::create_dir_all(&tmpdir).unwrap();
                fs::set_permissions(&tmpdir, fs::Permissions::from_mode(0o500)).unwrap();
            }
            let record = owned.path().join("provision");
            // No errexit: the hook itself must stop the shell, not merely return
            // a failure that the caller could overwrite by running its command.
            let output = enter(
                hook,
                &root,
                owned.path(),
                &record,
                "printf 'command ran\\n' > \"$JAUNDER_TEST_PARENT/command-ran\"",
            );
            if failure == "unusable" {
                fs::set_permissions(&tmpdir, fs::Permissions::from_mode(0o700)).unwrap();
            }
            assert_eq!(
                output.status.code(),
                Some(1),
                "{name}/{failure}: startup must fail with exit 1"
            );
            let stderr = String::from_utf8(output.stderr).unwrap();
            let diagnostic = if failure == "no-checkout" {
                "jaunder: cannot resolve an absolute Git checkout root for TMPDIR"
            } else {
                "jaunder: cannot create or use TMPDIR"
            };
            assert!(stderr.contains(diagnostic), "{name}/{failure}: {stderr}");
            assert!(
                !record.exists(),
                "{name}/{failure}: provisioning must not run"
            );
            assert!(
                !owned.path().join("command-ran").exists(),
                "{name}/{failure}: command must not run or fall back"
            );
            if failure == "blocked" {
                assert_eq!(
                    fs::read_to_string(&tmpdir).unwrap(),
                    "caller-owned obstruction"
                );
            }
            println!(
                "{name}/{failure}: visible nonzero failure before provisioning or command; no fallback"
            );
        }
    }
}
