use anyhow::{Context, Result, bail};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub fn open_lease(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening host-global lease {}", path.display()))
}

pub fn validate_lease_file(file: &File, _: &Path) -> Result<()> {
    if !file.metadata()?.is_file() {
        bail!("host-global lease is not a regular file");
    }
    Ok(())
}

pub fn global_lease_path() -> Result<PathBuf> {
    Ok(std::env::temp_dir().join("jaunder-production-baseline.lock"))
}

// Preserve the existing non-Unix behavior: POSIX permission modes do not apply.
pub fn restrict_evidence_workspace(_: &Path) -> Result<()> {
    Ok(())
}

pub fn restrict_workflow_error(_: &Path) -> Result<()> {
    Ok(())
}
