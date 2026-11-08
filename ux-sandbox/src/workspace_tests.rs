//! Real filesystem and COTS lock regressions for publication and cleanup boundaries.
use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum LockOperation {
    Acquire,
    Unlock,
}
impl Lock {
    pub(super) fn fail_once(&mut self, operation: LockOperation) -> io::Result<()> {
        if self.failure == Some(operation) {
            self.failure = None;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "one-shot lock operation failure",
            ));
        }
        Ok(())
    }
}

#[test]
fn lock_operation_errors_preserve_source_and_close_the_owned_descriptor() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("lock");
    let mut lock = Lock::open(&path).unwrap();
    lock.failure = Some(LockOperation::Acquire);
    let error = lock.acquire(&path).err().unwrap();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(error.to_string().contains(&path.display().to_string()));
    let mut lock = Lock::try_exclusive(&path).unwrap().unwrap();
    lock.failure = Some(LockOperation::Unlock);
    drop(lock);
    assert!(
        Lock::try_exclusive(&path).unwrap().is_some(),
        "closing the owned file still releases a lock when explicit unlock reports failure"
    );
}

#[test]
fn filesystem_inspection_and_metadata_errors_retain_the_failing_path() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, "sentinel").unwrap();
    let nested = file.join("child");
    let error = regular(&nested).unwrap_err();
    assert!(format!("{error:#}").contains(&nested.display().to_string()));
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::NotADirectory
    );
    let missing = root.path().join("missing");
    let error = read_profile(&missing).unwrap_err();
    assert!(error.to_string().contains("missing/.sandbox.json"));
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::NotFound
    );
    assert!(read_server(&missing).unwrap().is_none());
    assert!(
        discard(&missing)
            .unwrap_err()
            .to_string()
            .contains("workspace disappeared")
    );
    assert_eq!(fs::read_to_string(file).unwrap(), "sentinel");
}

#[test]
fn vanished_control_root_is_not_recreated_during_validation() {
    let root = tempfile::tempdir().unwrap();
    let paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
    let control = paths.metadata.parent().unwrap();
    fs::remove_dir(control).unwrap();
    let error = paths.validate().unwrap_err();
    assert!(error.to_string().contains(&control.display().to_string()));
    assert!(!control.exists());
}

#[test]
fn failed_publication_and_restoration_keep_old_for_next_recovery() {
    let root = tempfile::tempdir().unwrap();
    let mut paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
    populated(&paths.current, "current");
    populated(&paths.new, "new");
    paths.fail_publication = true;
    paths.fail_restoration = true;
    let error = paths.publish().unwrap_err();
    assert!(error.to_string().contains("restoring old also failed"));
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(!paths.current.exists());
    assert_eq!(
        fs::read_to_string(paths.old.join("marker")).unwrap(),
        "current"
    );
    assert_eq!(fs::read_to_string(paths.new.join("marker")).unwrap(), "new");
    paths.recover().unwrap();
    assert_eq!(
        fs::read_to_string(paths.current.join("marker")).unwrap(),
        "current"
    );
    assert!(!paths.new.exists());
    assert!(!paths.old.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn unreadable_regular_server_metadata_retains_io_source() {
    // procfs exposes a regular file whose bytes cannot be read as metadata.
    let path = Path::new("/proc/self/mem");
    let error = read_server(path).err().unwrap();
    assert!(error.to_string().contains("reading /proc/self/mem"));
    assert!(error.downcast_ref::<io::Error>().is_some());
}

#[tokio::test]
async fn cleanup_retries_io_error_without_releasing_lease_or_metadata() {
    let root = tempfile::tempdir().unwrap();
    let paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
    let lease = Lock::try_exclusive(&paths.lease).unwrap().unwrap();
    let exclusive = Lock::try_exclusive(&paths.ownership).unwrap().unwrap();
    let mut server = Server::new(paths.clone(), lease, exclusive, "abc".into());
    server.ready().unwrap();
    server.stopping().unwrap();
    fs::remove_file(&paths.ownership).unwrap();
    fs::create_dir(&paths.ownership).unwrap();
    let cleanup = server.cleanup();
    tokio::pin!(cleanup);
    assert!(
        tokio::time::timeout(Duration::from_millis(60), &mut cleanup)
            .await
            .is_err()
    );
    assert!(Lock::try_exclusive(&paths.lease).unwrap().is_none());
    assert!(read_server(&paths.metadata).unwrap().unwrap().state == State::Stopping);
    fs::remove_dir(&paths.ownership).unwrap();
    cleanup.await.unwrap();
    assert!(!paths.metadata.exists());
    assert!(Lock::try_exclusive(&paths.lease).unwrap().is_none());
}

#[test]
fn workspace_open_requires_a_validated_name() {
    use crate::sandbox_name::SandboxName;
    let open: fn(&Path, &SandboxName) -> anyhow::Result<Named> = Named::open;
    let root = tempfile::tempdir().unwrap();
    let name: SandboxName = "writing-flow_2".parse().unwrap();
    let paths = open(root.path(), &name).unwrap();
    assert_eq!(
        paths.current,
        root.path().join(".xtask/sandboxes/writing-flow_2")
    );
}

fn populated(path: &Path, marker: &str) {
    fs::create_dir(path).unwrap();
    fs::write(path.join("marker"), marker).unwrap();
    write_profile(path, Profile::Empty).unwrap();
}

#[test]
fn recovery_covers_all_current_new_old_combinations() {
    for bits in 0..8 {
        let root = tempfile::tempdir().unwrap();
        let paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
        for (bit, path, marker) in [
            (1, &paths.current, "current"),
            (2, &paths.new, "new"),
            (4, &paths.old, "old"),
        ] {
            if bits & bit != 0 {
                populated(path, marker);
            }
        }
        paths.recover().unwrap();
        assert!(!paths.new.exists() && !paths.old.exists());
        let expected = if bits & 1 != 0 {
            Some("current")
        } else if bits & 4 != 0 {
            Some("old")
        } else {
            None
        };
        assert_eq!(
            fs::read_to_string(paths.current.join("marker"))
                .ok()
                .as_deref(),
            expected,
            "state {bits}"
        );
    }
}

#[test]
fn recovery_refuses_to_move_an_externally_owned_old_workspace() {
    let root = tempfile::tempdir().unwrap();
    let paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
    populated(&paths.old, "old");
    let runtime = Lock::runtime(&paths.old).unwrap();
    assert!(paths.recover().is_err());
    assert!(!paths.current.exists());
    assert_eq!(fs::read_to_string(paths.old.join("marker")).unwrap(), "old");
    drop(runtime);
    paths.recover().unwrap();
    assert_eq!(
        fs::read_to_string(paths.current.join("marker")).unwrap(),
        "old"
    );
}

#[test]
fn failed_second_rename_restores_old_without_publishing_partial_state() {
    let root = tempfile::tempdir().unwrap();
    let mut paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
    populated(&paths.current, "current");
    populated(&paths.new, "new");
    paths.fail_publication = true;
    let error = paths.publish().unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        fs::read_to_string(paths.current.join("marker")).unwrap(),
        "current"
    );
    assert!(!paths.old.exists());
    assert!(paths.new.exists());
    paths.recover().unwrap();
    assert!(!paths.new.exists());
}

#[test]
fn successful_replacement_publishes_new_profile_and_retires_old_generation() {
    let root = tempfile::tempdir().unwrap();
    let mut paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
    populated(&paths.current, "current");
    populated(&paths.new, "new");
    write_profile(&paths.new, Profile::Demo).unwrap();
    paths.publish().unwrap();
    assert_eq!(
        fs::read_to_string(paths.current.join("marker")).unwrap(),
        "new"
    );
    assert_eq!(read_profile(&paths.current).unwrap(), Profile::Demo);
    assert!(!paths.new.exists());
    assert!(!paths.old.exists());
}

#[test]
fn profile_schema_and_reset_selection_remain_legacy_compatible() {
    let root = tempfile::tempdir().unwrap();
    let paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
    assert!(paths.profile(None, true).is_err());
    assert_eq!(paths.profile(None, false).unwrap(), Profile::Empty);
    populated(&paths.current, "current");
    fs::write(
        paths.current.join(".sandbox.json"),
        r#"{"version":1,"profile":"demo"}"#,
    )
    .unwrap();
    assert_eq!(paths.profile(None, false).unwrap(), Profile::Demo);
    assert_eq!(paths.profile(None, true).unwrap(), Profile::Demo);
    assert_eq!(
        paths.profile(Some(Profile::Standard), true).unwrap(),
        Profile::Standard
    );
    assert!(paths.profile(Some(Profile::Empty), false).is_err());
    fs::write(
        paths.current.join(".sandbox.json"),
        r#"{"version":2,"profile":"demo"}"#,
    )
    .unwrap();
    assert!(read_profile(&paths.current).is_err());
}

#[test]
fn root_control_workspace_and_reset_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    for relative in [
        ".xtask",
        ".xtask/sandboxes",
        ".xtask/sandboxes/.locks",
        ".xtask/sandboxes/writing",
        ".xtask/sandboxes/.writing.reset-new",
        ".xtask/sandboxes/.writing.reset-old",
        ".xtask/sandboxes/.locks/writing.server.lock",
        ".xtask/sandboxes/.locks/writing.workspace.lock",
        ".xtask/sandboxes/.locks/writing.server.json",
        ".xtask/sandboxes/writing/.sandbox.json",
        ".xtask/sandboxes/writing/runtime.lock",
        ".xtask/sandboxes/writing/jaunder.db",
    ] {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("untouched"), "data").unwrap();
        let link = root.path().join(relative);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(external.path(), link).unwrap();
        assert!(
            Named::open(root.path(), &"writing".parse().unwrap()).is_err(),
            "{relative}"
        );
        assert_eq!(
            fs::read_to_string(external.path().join("untouched")).unwrap(),
            "data"
        );
    }
}

#[test]
fn legacy_ready_and_stopping_metadata_keep_exact_version_one_shape() {
    let root = tempfile::tempdir().unwrap();
    let paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
    for (state, literal) in [(State::Ready, "ready"), (State::Stopping, "stopping")] {
        fs::write(
            &paths.metadata,
            format!(r#"{{"version":1,"state":"{literal}","executable_digest":"abc"}}"#),
        )
        .unwrap();
        let decoded = read_server(&paths.metadata).unwrap().unwrap();
        assert!(decoded.state == state);
        assert_eq!(decoded.executable_digest, "abc");
        write_json(&paths.metadata, &decoded).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&paths.metadata).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"version":1,"state":literal,"executable_digest":"abc"})
        );
    }
}

#[tokio::test]
async fn stopping_lease_survives_exclusive_cleanup_barrier_until_commands_leave() {
    let root = tempfile::tempdir().unwrap();
    let paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
    let lease = Lock::try_exclusive(&paths.lease).unwrap().unwrap();
    let exclusive = Lock::try_exclusive(&paths.ownership).unwrap().unwrap();
    let mut server = Server::new(paths.clone(), lease, exclusive, "abc".into());
    server.ready().unwrap();
    let command = Lock::shared(&paths.ownership).unwrap();
    server.stopping().unwrap();
    assert!(read_server(&paths.metadata).unwrap().unwrap().state == State::Stopping);
    let cleanup = server.cleanup();
    tokio::pin!(cleanup);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut cleanup)
            .await
            .is_err()
    );
    assert!(Lock::try_exclusive(&paths.lease).unwrap().is_none());
    assert!(paths.metadata.exists());
    drop(command);
    cleanup.await.unwrap();
    assert!(!paths.metadata.exists());
    assert!(Lock::try_exclusive(&paths.lease).unwrap().is_none());
}
