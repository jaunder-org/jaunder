//! Passive, bounded observations for the existing test-local owner.
//!
//! Start and completion publish independently through the trace owner's atomic
//! no-overwrite writer. Only generated identity, fixed labels, clocks and observed
//! outcome enter the records. An explicit parent is an observing owner's context,
//! not a global root. Private worktree bundles have no pruning or age expiry.
//! This recorder has no execution, cleanup, signal or drop-completion authority.

use std::{
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// No drop completion: disappearance cannot establish a workload outcome.
/// The owner supplies only its observed result, never caller-controlled text.
pub(crate) struct Receipt {
    directory: PathBuf,
    id: String,
    parent_id: Option<String>,
    started: Instant,
    utc_unix_ms: u128,
}

impl Receipt {
    pub(crate) fn start(root: &Path, parent: Option<&Self>) -> Result<Self, &'static str> {
        let started = Instant::now();
        let utc_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "clock-unavailable")?
            .as_millis();
        let bundles = root.join(".xtask/workflow-traces");
        std::fs::create_dir_all(&bundles).map_err(|_| "directory-create")?;
        // Set privacy at creation, not after publishing potentially readable data.
        let mut builder = tempfile::Builder::new();
        builder.prefix("run-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let directory = builder
            .tempdir_in(bundles)
            .map_err(|_| "directory-create")?
            .keep();
        let id = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("invocation-id")?
            .to_owned();
        let receipt = Self {
            directory,
            id,
            parent_id: parent.map(|parent| parent.id.clone()),
            started,
            utc_unix_ms,
        };
        receipt.publish("start.json", "start", "unknown", None)?;
        Ok(receipt)
    }

    pub(crate) fn complete(self, ok: bool, exit_code: i32) -> Result<(), &'static str> {
        self.publish(
            "completion.json",
            "completion",
            if ok { "success" } else { "failure" },
            Some(exit_code),
        )
    }

    fn publish(
        &self,
        file: &str,
        observation: &str,
        outcome: &str,
        exit_code: Option<i32>,
    ) -> Result<(), &'static str> {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "invocation_id": self.id,
            "parent_id": self.parent_id,
            "stage": "test-local",
            "observation": observation,
            "utc_unix_ms": self.utc_unix_ms,
            "elapsed_ms": self.started.elapsed().as_millis(),
            "outcome": outcome,
            "exit_code": exit_code,
            "capture_health": "available",
        }))
        .map_err(|_| "serialization")?;
        crate::invocation_trace::persist_file(&self.directory, &self.directory.join(file), &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_publishes_start_before_completion_without_caller_data() {
        let root = tempfile::tempdir().unwrap();
        let receipt = Receipt::start(root.path(), None).unwrap();
        let start: serde_json::Value =
            serde_json::from_slice(&std::fs::read(receipt.directory.join("start.json")).unwrap())
                .unwrap();
        assert_eq!(start["stage"], "test-local");
        assert_eq!(start["outcome"], "unknown");
        assert_eq!(start["capture_health"], "available");
        assert!(!receipt.directory.join("completion.json").exists());
    }

    #[test]
    fn nested_parallel_and_incomplete_observations_remain_separate() {
        let root = tempfile::tempdir().unwrap();
        let parent = Receipt::start(root.path(), None).unwrap();
        let children = std::thread::scope(|scope| {
            let first = scope.spawn(|| Receipt::start(root.path(), Some(&parent)).unwrap());
            let second = scope.spawn(|| Receipt::start(root.path(), Some(&parent)).unwrap());
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_ne!(children[0].id, children[1].id);
        for child in children {
            let start: serde_json::Value =
                serde_json::from_slice(&std::fs::read(child.directory.join("start.json")).unwrap())
                    .unwrap();
            assert_eq!(start["parent_id"], parent.id);
            child.complete(true, 0).unwrap();
        }
        let directory = parent.directory.clone();
        drop(parent);
        assert!(directory.join("start.json").exists());
        assert!(
            !directory.join("completion.json").exists(),
            "missing terminal evidence is unknown, even on drop"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(directory.join("start.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn unavailable_and_partial_publication_preserve_start() {
        let root = tempfile::tempdir().unwrap();
        let blocked = root.path().join("blocked");
        std::fs::write(&blocked, "secret-shaped-IO-error").unwrap();
        assert!(matches!(
            Receipt::start(&blocked, None),
            Err("directory-create")
        ));
        let receipt = Receipt::start(root.path(), None).unwrap();
        let directory = receipt.directory.clone();
        // A participant cannot overwrite an existing terminal publication.
        std::fs::create_dir(directory.join("completion.json")).unwrap();
        assert_eq!(receipt.complete(false, 19), Err("file-rename"));
        assert!(directory.join("start.json").is_file());
        assert!(directory.join("completion.json").is_dir());
        assert_eq!(
            std::fs::read_dir(&directory).unwrap().count(),
            2,
            "failed temporary publication is not a terminal record"
        );
    }

    #[test]
    fn owner_completion_retains_only_bounded_observed_outcome() {
        let root = tempfile::Builder::new()
            .prefix("password-SUPERSECRET-")
            .tempdir()
            .unwrap();
        for (ok, code, outcome) in [
            (true, 0, "success"),
            (false, 17, "failure"),
            (false, 130, "failure"),
        ] {
            let receipt = Receipt::start(root.path(), None).unwrap();
            let directory = receipt.directory.clone();
            receipt.complete(ok, code).unwrap();
            let bytes = std::fs::read(directory.join("completion.json")).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("SUPERSECRET"));
            let completed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(completed["outcome"], outcome);
            assert_eq!(completed["exit_code"], code);
            assert_eq!(completed["observation"], "completion");
            assert_eq!(completed["capture_health"], "available");
            let keys: Vec<_> = completed
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                [
                    "capture_health",
                    "elapsed_ms",
                    "exit_code",
                    "invocation_id",
                    "observation",
                    "outcome",
                    "parent_id",
                    "schema_version",
                    "stage",
                    "utc_unix_ms"
                ]
            );
        }
    }
}
