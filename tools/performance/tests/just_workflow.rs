#![cfg(unix)]

use anyhow::{Context, Result};
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

const NONCE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct WorkflowFixture {
    directory: TempDir,
    log: PathBuf,
    outputs: PathBuf,
    selection: PathBuf,
    parameters: PathBuf,
    path: String,
}

impl WorkflowFixture {
    fn new(failed_status: Option<&str>) -> Result<Self> {
        let directory = tempfile::tempdir().context("temporary Just checkout")?;
        let fake_bin = directory.path().join("bin");
        let outputs = directory.path().join("outputs");
        fs::create_dir_all(&fake_bin).context("create fake bin")?;
        fs::create_dir_all(&outputs).context("create fake producer outputs")?;
        let log = directory.path().join("calls.log");
        let selection = directory.path().join("selection.json");
        let parameters = directory.path().join("parameters.json");
        let producers = [
            ("performance-storage-sqlite", "storage", "sqlite", None),
            (
                "performance-browser-sqlite-chromium",
                "browser",
                "sqlite",
                Some("chromium"),
            ),
            ("performance-storage-postgres", "storage", "postgres", None),
            (
                "performance-browser-postgres-chromium",
                "browser",
                "postgres",
                Some("chromium"),
            ),
        ];
        let producer_json = producers
            .iter()
            .map(|(name, producer, backend, browser)| {
                serde_json::json!({
                    "name": name,
                    "installable": format!("/fake/{name}.drv^out"),
                    "identity": outputs.join(name).to_string_lossy().to_string(),
                    "producer": producer,
                    "backend": backend,
                    "browser": browser,
                })
            })
            .collect::<Vec<_>>();
        fs::write(
            &selection,
            serde_json::json!({
                "profile": "small",
                "selection": {
                    "backends": ["sqlite", "postgres"],
                    "browsers": ["chromium"],
                    "storage": true,
                    "browser": true,
                    "count_overrides": {"posts": null, "authors": null, "revisions": null},
                },
                "producers": producer_json,
            })
            .to_string(),
        )
        .context("write Nix selection fixture")?;

        for (name, _, _, _) in producers {
            let root = outputs.join(name).join("performance");
            fs::create_dir_all(root.join("fragments")).context("create fragment directory")?;
            fs::write(
                root.join("producer-status-v1.json"),
                serde_json::json!({
                    "schema_version": 1,
                    "ok": failed_status != Some(name),
                    "detail": "injected producer status",
                })
                .to_string(),
            )
            .context("write producer status fixture")?;
            fs::write(root.join("fragments/placeholder.json"), "{}")
                .context("fragment placeholder")?;
        }

        write_executable(
            &fake_bin.join("nix"),
            "#!/usr/bin/env bash\nset -euo pipefail\nif [[ ${1-} == eval ]]; then\n  printf '%s\\n' \"$JAUNDER_PERF_PARAMETERS\" > \"$JAUNDER_TEST_PARAMETERS\"\n  jq --argjson params \"$JAUNDER_PERF_PARAMETERS\" '.profile = $params.profile' \"$JAUNDER_TEST_SELECTION\"\n  exit 0\nfi\n[[ ${1-} == build ]] || exit 90\ninstallable=\"${!#}\"\nname=$(basename \"${installable%^out}\" .drv)\nprintf 'build:%s\\n' \"$name\" >> \"$JAUNDER_TEST_LOG\"\nif [[ $JAUNDER_TEST_MODE == build-failure && $name == performance-browser-sqlite-chromium ]]; then exit 73; fi\nprintf '%s\\n' \"$JAUNDER_TEST_OUTPUTS/$name\"\n",
        )?;
        write_executable(
            &fake_bin.join("cargo"),
            &format!(
                "#!/usr/bin/env bash\nset -euo pipefail\ncase \" $* \" in\n  *' nonce '*) printf '%s\\n' '{NONCE}' ;;\n  *' validate-selection '*) printf 'preflight\\n' >> $JAUNDER_TEST_LOG; [[ $JAUNDER_TEST_MODE != preflight-failure ]] ;;\n  *' validate-producer '*)\n    root=\"${{!#}}\"\n    name=$(basename \"$(dirname \"$root\")\")\n    printf 'validate:%s\\n' \"$name\" >> \"$JAUNDER_TEST_LOG\"\n    jq -e '.ok == true' \"$root/producer-status-v1.json\" >/dev/null\n    ;;\n  *) echo \"unexpected cargo command: $*\" >&2; exit 91 ;;\nesac\n"
            ),
        )?;
        let path = format!(
            "{}:{}",
            fake_bin.display(),
            env::var("PATH").context("test PATH")?
        );
        Ok(Self {
            directory,
            log,
            outputs,
            selection,
            parameters,
            path,
        })
    }

    fn run(&self, mode: &str, arguments: &[&str]) -> Result<Output> {
        let mut command = Command::new("just");
        command
            .arg("--justfile")
            .arg(repo_root()?.join("Justfile"))
            .arg("--working-directory")
            .arg(self.directory.path())
            .arg("perf")
            .args(arguments)
            .env("PATH", &self.path)
            .env("JAUNDER_TEST_LOG", &self.log)
            .env("JAUNDER_TEST_OUTPUTS", &self.outputs)
            .env("JAUNDER_TEST_SELECTION", &self.selection)
            .env("JAUNDER_TEST_PARAMETERS", &self.parameters)
            .env("JAUNDER_TEST_MODE", mode);
        command
            .output()
            .context("run native Just workflow against fake external commands")
    }

    fn run_root(&self) -> PathBuf {
        self.directory.path().join(".xtask/performance").join(NONCE)
    }

    fn parameters(&self) -> Result<serde_json::Value> {
        serde_json::from_slice(
            &fs::read(&self.parameters).context("Nix selector parameters were captured")?,
        )
        .context("Nix selector parameters are JSON")
    }

    fn calls(&self) -> Result<Vec<String>> {
        Ok(fs::read_to_string(&self.log)
            .context("read command order log")?
            .lines()
            .map(str::to_owned)
            .collect())
    }
}

fn repo_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("performance crate is under tools/ at repository root")
        .map(Path::to_owned)
}

fn write_executable(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents).context("write fake command")?;
    let mut permissions = fs::metadata(path)
        .context("read fake command mode")?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).context("make fake command executable")
}

#[test]
fn native_recipe_defaults_forward_the_medium_profile_and_unset_selectors() -> Result<()> {
    let fixture = WorkflowFixture::new(None)?;
    let output = fixture.run("build-failure", &[])?;
    assert!(
        !output.status.success(),
        "injected later build failure should be visible"
    );
    let parameters = fixture.parameters()?;
    assert_eq!(parameters["profile"], "medium");
    assert_eq!(parameters["backends"], "sqlite,postgres");
    assert_eq!(parameters["browsers"], "");
    assert_eq!(parameters["workload"], "all");
    assert_eq!(parameters["posts"], "");
    assert_eq!(parameters["authors"], "");
    assert_eq!(parameters["revisions"], "");
    Ok(())
}

#[test]
fn native_storage_parameters_and_count_overrides_reach_nix_selection() -> Result<()> {
    let fixture = WorkflowFixture::new(None)?;
    let output = fixture.run(
        "build-failure",
        &[
            "small",
            "sqlite,postgres",
            "",
            "storage",
            "120",
            "12",
            "777",
        ],
    )?;
    assert!(!output.status.success(), "later build failure is injected");
    let parameters = fixture.parameters()?;
    assert_eq!(parameters["profile"], "small");
    assert_eq!(parameters["backends"], "sqlite,postgres");
    assert_eq!(parameters["browsers"], "");
    assert_eq!(parameters["workload"], "storage");
    assert_eq!(parameters["posts"], "120");
    assert_eq!(parameters["authors"], "12");
    assert_eq!(parameters["revisions"], "777");
    Ok(())
}

#[test]
fn rejected_selection_stops_before_any_producer_build() -> Result<()> {
    let fixture = WorkflowFixture::new(None)?;
    let output = fixture.run(
        "preflight-failure",
        &["small", "sqlite", "", "storage", "100", "12", "777"],
    )?;
    assert!(
        !output.status.success(),
        "rejected preflight must fail Just"
    );
    assert_eq!(fixture.calls()?, ["preflight"]);
    assert!(
        !fixture
            .run_root()
            .join("producers/performance-storage-sqlite")
            .exists()
    );
    Ok(())
}

#[test]
fn later_nix_build_failure_preserves_completed_producer_and_stops_sequence() -> Result<()> {
    let fixture = WorkflowFixture::new(None)?;
    let output = fixture.run("build-failure", &["small", "sqlite,postgres", "", "all"])?;
    assert!(
        !output.status.success(),
        "failure must stop the Just recipe"
    );
    assert!(
        fixture
            .run_root()
            .join("producers/performance-storage-sqlite/performance/producer-status-v1.json")
            .is_file()
    );
    assert!(
        !fixture
            .run_root()
            .join("producers/performance-browser-sqlite-chromium/performance")
            .exists()
    );
    assert_eq!(
        fixture.calls()?,
        [
            "preflight",
            "build:performance-storage-sqlite",
            "validate:performance-storage-sqlite",
            "build:performance-browser-sqlite-chromium",
        ]
    );
    Ok(())
}

#[test]
fn later_producer_status_failure_preserves_evidence_and_stops_sequence() -> Result<()> {
    let fixture = WorkflowFixture::new(Some("performance-browser-sqlite-chromium"))?;
    let output = fixture.run("status-failure", &["small", "sqlite,postgres", "", "all"])?;
    assert!(
        !output.status.success(),
        "failed producer status must stop Just"
    );
    assert!(
        fixture
            .run_root()
            .join("producers/performance-storage-sqlite/performance/producer-status-v1.json")
            .is_file()
    );
    assert!(
        fixture
            .run_root()
            .join(
                "producers/performance-browser-sqlite-chromium/performance/producer-status-v1.json"
            )
            .is_file()
    );
    assert!(
        !fixture
            .run_root()
            .join("producers/performance-storage-postgres/performance")
            .exists()
    );
    assert_eq!(
        fixture.calls()?,
        [
            "preflight",
            "build:performance-storage-sqlite",
            "validate:performance-storage-sqlite",
            "build:performance-browser-sqlite-chromium",
            "validate:performance-browser-sqlite-chromium",
        ]
    );
    Ok(())
}
