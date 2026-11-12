use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

pub fn open_lease(path: &Path) -> Result<File> {
    // Use the OS constant, not a manually maintained table of numeric values.
    // O_NOFOLLOW keeps an attacker-controlled link from becoming the lock inode.
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().try_into()?)
        .open(path)
        .with_context(|| format!("opening host-global lease {}", path.display()))
}

pub fn validate_lease_file(file: &File, path: &Path) -> Result<()> {
    let metadata = file
        .metadata()
        .with_context(|| format!("inspecting host-global lease {}", path.display()))?;
    if !metadata.is_file() || metadata.uid() != rustix::process::geteuid().as_raw() {
        bail!("host-global lease is not a regular file owned by this user");
    }
    Ok(())
}

pub fn global_lease_path() -> Result<PathBuf> {
    let uid = rustix::process::geteuid().as_raw();
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && private_runtime_directory(path, uid))
        .unwrap_or_else(|| std::env::temp_dir().join(format!("jaunder-production-baseline-{uid}")));
    ensure_private_runtime_directory(&runtime, uid)?;
    Ok(runtime.join("jaunder-production-baseline.lock"))
}

fn private_runtime_directory(path: &Path, uid: u32) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o777 == 0o700
}

fn ensure_private_runtime_directory(path: &Path, uid: u32) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .context("restricting host lease runtime directory")?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!("creating host lease runtime directory {}", path.display())
            });
        }
    }
    if private_runtime_directory(path, uid) {
        Ok(())
    } else {
        bail!("host lease runtime directory is not caller-owned mode 0700")
    }
}

pub fn restrict_evidence_workspace(path: &Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .context("restricting evidence staging workspace")
}

pub fn restrict_workflow_error(path: &Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::production_baseline::RunLease;
    use std::os::unix::fs::symlink;

    #[test]
    fn host_global_lease_refuses_a_symlink_without_truncating_its_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let path = temp.path().join("production-baseline.lock");
        fs::write(&target, b"must remain intact").unwrap();
        symlink(&target, &path).unwrap();

        assert!(RunLease::acquire_at(&path).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"must remain intact");
    }

    #[test]
    fn private_runtime_directory_requires_owner_mode_and_no_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime");
        let uid = rustix::process::geteuid().as_raw();
        ensure_private_runtime_directory(&path, uid).unwrap();
        assert!(private_runtime_directory(&path, uid));
        assert!(ensure_private_runtime_directory(&path, uid).is_ok());
        assert!(!private_runtime_directory(&path, uid.wrapping_add(1)));

        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ensure_private_runtime_directory(&path, uid).is_err());
        let link = temp.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(!private_runtime_directory(&link, uid));
        assert!(ensure_private_runtime_directory(&link, uid).is_err());
    }

    #[test]
    fn evidence_permissions_are_restricted() {
        let temp = tempfile::tempdir().unwrap();
        restrict_evidence_workspace(temp.path()).unwrap();
        assert_eq!(fs::metadata(temp.path()).unwrap().mode() & 0o777, 0o700);
        let path = temp.path().join("workflow-error.txt");
        fs::write(&path, "error").unwrap();
        restrict_workflow_error(&path).unwrap();
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
    }
}
