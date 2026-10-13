//! Production-feature application identity for out-of-process regression tests.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempPath;

const RETAINED_PREFIX: &str = "jaunder-native-application-";

pub(super) struct PreparedApplication {
    file: TempPath,
    digest: [u8; 32],
}

impl PreparedApplication {
    pub(super) fn prepare(sh: &xshell::Shell, env: &[(String, String)]) -> Result<Self> {
        let mut command = sh
            .cmd("cargo")
            .args([
                "build",
                "--locked",
                "--package",
                "jaunder",
                "--bin",
                "jaunder",
                "--message-format=json",
            ])
            .quiet()
            .ignore_status();
        for (key, value) in env {
            command = command.env(key, value);
        }
        let output = command
            .output()
            .context("build production application fixture")?;
        if !output.status.success() {
            bail!(
                "application fixture build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let artifacts =
            std::str::from_utf8(&output.stdout).context("application artifact encoding")?;
        let executable = executable_from_artifacts(&sh.current_dir(), artifacts)?;
        Self::retain(&executable)
    }

    fn retain(executable: &Path) -> Result<Self> {
        let source = std::fs::read(executable).context("read Cargo application executable")?;
        let parent = executable
            .parent()
            .context("application profile directory")?;
        let objects = parent.join("deps");
        for entry in std::fs::read_dir(&objects).context("application object directory")? {
            let entry = entry.context("inspect application object directory")?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(RETAINED_PREFIX)
            {
                bail!(
                    "unsettled application fixture object exists: {}",
                    entry.path().display()
                );
            }
        }
        let file = tempfile::Builder::new()
            .prefix(RETAINED_PREFIX)
            .tempfile_in(objects)
            .context("reserve owned application object")?;
        std::fs::copy(executable, file.path()).context("retain application object")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o700))
                .context("private executable fixture permissions")?;
        }
        let retained = std::fs::read(file.path()).context("verify retained application")?;
        if Sha256::digest(&source) != Sha256::digest(&retained) {
            bail!("retained application bytes differ from Cargo artifact");
        }
        Ok(Self {
            file: file.into_temp_path(),
            digest: Sha256::digest(&source).into(),
        })
    }

    pub(super) fn path(&self) -> &Path {
        self.file.as_ref()
    }

    pub(super) fn close(self) -> Result<()> {
        let identity = std::fs::read(&self.file)
            .context("read retained application after test compilation")
            .and_then(|bytes| {
                if <[u8; 32]>::from(Sha256::digest(bytes)) != self.digest {
                    bail!("retained application changed during the test lane");
                }
                Ok(())
            });
        let cleanup = self
            .file
            .close()
            .context("remove owned application fixture object");
        match (identity, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(identity), Err(cleanup)) => {
                Err(identity.context(format!("application cleanup also failed: {cleanup:#}")))
            }
        }
    }
}

pub(super) fn executable_from_artifacts(root: &Path, output: &str) -> Result<PathBuf> {
    let records = output
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("decode application Cargo artifacts")?;
    let artifacts = records
        .iter()
        .filter(|record| record["reason"] == "compiler-artifact")
        .collect::<Vec<_>>();
    for (crate_name, forbidden) in [("host", "cheap-kdf"), ("storage", "test-utils")] {
        let source = root.join(crate_name).join("src/lib.rs");
        let matching = artifacts
            .iter()
            .filter(|record| record["target"]["src_path"].as_str().map(Path::new) == Some(&source))
            .collect::<Vec<_>>();
        if matching.is_empty() {
            bail!("missing application {crate_name} feature evidence");
        }
        for record in matching {
            let features = record["features"]
                .as_array()
                .context("application feature schema")?;
            if features.iter().any(|feature| feature == forbidden) {
                bail!("application enables forbidden {crate_name}/{forbidden}");
            }
            if features.iter().any(|feature| !feature.is_string()) {
                bail!("invalid application feature evidence");
            }
        }
    }
    let source = root.join("server/src/main.rs");
    let package_url = url::Url::from_file_path(root.join("server"))
        .map_err(|()| anyhow::anyhow!("absolute application package path required"))?;
    let package_prefix = format!("path+{package_url}#jaunder@");
    let executables = artifacts
        .iter()
        .filter(|record| {
            record["target"]["src_path"].as_str().map(Path::new) == Some(&source)
                && record["target"]["name"] == "jaunder"
                && record["target"]["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .map(|record| {
            let package = record["package_id"]
                .as_str()
                .context("application package identity")?;
            if !package.starts_with(&package_prefix) {
                bail!("unexpected application package identity");
            }
            record["executable"]
                .as_str()
                .context("application executable schema")
        })
        .collect::<Result<Vec<_>>>()?;
    if executables.len() != 1 {
        bail!("expected exactly one application executable");
    }
    let executable = PathBuf::from(executables[0]);
    if !executable.is_absolute() {
        bail!("application executable must be absolute");
    }
    Ok(executable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn artifacts() -> Vec<Value> {
        vec![
            json!({"reason":"compiler-artifact","target":{"src_path":"/repo/host/src/lib.rs"},"features":["default","sqlx"]}),
            json!({"reason":"compiler-artifact","target":{"src_path":"/repo/storage/src/lib.rs"},"features":["default","sqlx"]}),
            json!({"reason":"compiler-artifact","package_id":"path+file:///repo/server#jaunder@0.1.0","target":{"src_path":"/repo/server/src/main.rs","name":"jaunder","kind":["bin"]},"executable":"/repo/target/debug/jaunder"}),
        ]
    }

    fn encode(records: &[Value]) -> String {
        records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn retained_object_lifetime_preserves_original_and_rejects_unsettled_predecessors() {
        let directory = tempfile::tempdir().expect("fixture profile directory");
        std::fs::create_dir(directory.path().join("deps")).expect("object directory");
        let original = directory.path().join("jaunder");
        std::fs::write(&original, b"controlled executable bytes").expect("original bytes");
        let retained = PreparedApplication::retain(&original).expect("owned copy");
        let path = retained.path().to_owned();
        assert_eq!(
            std::fs::read(&path).expect("retained bytes"),
            std::fs::read(&original).expect("original bytes")
        );
        assert!(
            PreparedApplication::retain(&original).is_err(),
            "an unsettled object cannot join another run"
        );
        assert!(
            path.exists(),
            "collision rejection cannot remove the prior object"
        );
        retained.close().expect("owned copy cleanup");
        assert!(!path.exists());
        assert_eq!(
            std::fs::read(&original).expect("original retained"),
            b"controlled executable bytes"
        );
        let next = PreparedApplication::retain(&original).expect("subsequent run after cleanup");
        let changed = next.path().to_owned();
        std::fs::write(&changed, b"unexpected changed bytes").expect("controlled identity failure");
        assert!(next.close().is_err(), "changed identity cannot be accepted");
        assert!(
            !changed.exists(),
            "identity failure still closes owned object"
        );
        let final_run = PreparedApplication::retain(&original).expect("clean final run");
        final_run.close().expect("subsequent cleanup");
    }

    #[test]
    fn production_application_selection_requires_unique_identity_and_feature_evidence() {
        let root = Path::new("/repo");
        let records = artifacts();
        assert_eq!(
            executable_from_artifacts(root, &encode(&records)).expect("production artifact"),
            Path::new("/repo/target/debug/jaunder")
        );
        assert!(executable_from_artifacts(root, "").is_err());
        assert!(executable_from_artifacts(root, "not JSON").is_err());
        let mut ambiguous = records.clone();
        ambiguous.push(records[2].clone());
        assert!(executable_from_artifacts(root, &encode(&ambiguous)).is_err());
        for (index, feature) in [(0, "cheap-kdf"), (1, "test-utils")] {
            let mut unified = records.clone();
            unified[index]["features"] = json!([feature]);
            assert!(executable_from_artifacts(root, &encode(&unified)).is_err());
        }
        let mut missing = records.clone();
        missing.remove(0);
        assert!(executable_from_artifacts(root, &encode(&missing)).is_err());
        for value in [json!(null), json!("relative/jaunder")] {
            let mut invalid = records.clone();
            invalid[2]["executable"] = value;
            assert!(executable_from_artifacts(root, &encode(&invalid)).is_err());
        }
        let mut foreign = records.clone();
        foreign[2]["package_id"] = json!("registry+https://example.test#jaunder@0.1.0");
        assert!(executable_from_artifacts(root, &encode(&foreign)).is_err());
        let mut invalid = records;
        invalid[0]["features"] = json!([42]);
        assert!(executable_from_artifacts(root, &encode(&invalid)).is_err());
    }
}
