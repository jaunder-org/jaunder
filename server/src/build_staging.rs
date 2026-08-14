use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct StageError {
    pub operation: &'static str,
    pub path: PathBuf,
    source: io::Error,
}

impl fmt::Display for StageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} staging directory {}: {}",
            self.operation,
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for StageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Removes and recreates the staging directory before invoking `stage`.
///
/// # Errors
///
/// Returns a [`StageError`] when removing the stale directory or creating its
/// replacement fails.
pub fn prepare_staging_with(
    site_dir: &Path,
    remove: impl FnOnce(&Path) -> io::Result<()>,
    create: impl FnOnce(&Path) -> io::Result<()>,
    stage: impl FnOnce(),
) -> Result<(), StageError> {
    match remove(site_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(StageError {
                operation: "removing",
                path: site_dir.to_path_buf(),
                source,
            });
        }
    }
    create(site_dir).map_err(|source| StageError {
        operation: "creating",
        path: site_dir.to_path_buf(),
        source,
    })?;
    stage();
    Ok(())
}
