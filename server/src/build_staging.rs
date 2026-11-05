use std::fmt;
use std::fs;
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

#[derive(Debug)]
pub struct BundleStageError(String);

impl fmt::Display for BundleStageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BundleStageError {}

/// Copy Vite's output tree into the embedded site, excluding build-only output.
/// The ordinary Vite manifest is consumed by the build script, not served; the
/// independently verified system-theme inventory is staged in its private tree.
///
/// # Errors
///
/// Returns an error when `index.html` is absent or a tree copy fails.
pub fn stage_vite_output(root: &Path, site: &Path) -> Result<(), BundleStageError> {
    if !root.join("index.html").is_file() {
        return Err(BundleStageError(format!(
            "Vite output {} has no index.html",
            root.display()
        )));
    }
    for entry in fs::read_dir(root)
        .map_err(|error| BundleStageError(format!("reading {}: {error}", root.display())))?
    {
        let entry = entry
            .map_err(|error| BundleStageError(format!("reading {}: {error}", root.display())))?;
        if matches!(
            entry.file_name().to_str(),
            Some(".vite" | "system-artifacts")
        ) {
            continue;
        }
        let source = entry.path();
        let destination = site.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| BundleStageError(format!("stating {}: {error}", source.display())))?
            .is_dir()
        {
            copy_tree(&source, &destination)?;
        } else {
            copy_file(&source, &destination)?;
        }
    }
    Ok(())
}

fn copy_file(src: &Path, dst: &Path) -> Result<(), BundleStageError> {
    let parent = dst
        .parent()
        .ok_or_else(|| BundleStageError(format!("staged path {} has no parent", dst.display())))?;
    fs::create_dir_all(parent)
        .map_err(|error| BundleStageError(format!("creating {}: {error}", parent.display())))?;
    fs::copy(src, dst).map_err(|error| {
        BundleStageError(format!(
            "copying {} to {}: {error}",
            src.display(),
            dst.display()
        ))
    })?;
    Ok(())
}

/// Copy public assets for an intentionally empty local CSR staging directory.
///
/// # Errors
///
/// Returns an error when copying the public tree fails.
pub fn stage_public_tree(src: &Path, dst: &Path) -> Result<(), BundleStageError> {
    if src.is_dir() {
        copy_tree(src, dst)?;
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), BundleStageError> {
    fs::create_dir_all(dst)
        .map_err(|error| BundleStageError(format!("creating {}: {error}", dst.display())))?;
    for entry in fs::read_dir(src)
        .map_err(|error| BundleStageError(format!("reading {}: {error}", src.display())))?
    {
        let entry = entry
            .map_err(|error| BundleStageError(format!("reading {}: {error}", src.display())))?;
        let path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| BundleStageError(format!("stating {}: {error}", path.display())))?
            .is_dir()
        {
            copy_tree(&path, &dst_path)?;
        } else {
            copy_file(&path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, io};
    use tempfile::tempdir;

    #[test]
    fn build_script_staging_cleanup_failure_aborts_before_create_or_copy() {
        let created = Cell::new(false);
        let copied = Cell::new(false);
        let site = Path::new("/injected/out/site");
        let error = prepare_staging_with(
            site,
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected remove failure",
                ))
            },
            |_| {
                created.set(true);
                Ok(())
            },
            || copied.set(true),
        )
        .unwrap_err();
        assert_eq!(error.operation, "removing");
        assert_eq!(error.path, site);
        assert!(!created.get());
        assert!(!copied.get());
    }

    #[test]
    fn stages_vite_outputs_and_sidecars_but_not_build_only_metadata() {
        let root = tempdir().unwrap();
        let site = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".vite")).unwrap();
        fs::create_dir_all(root.path().join("assets")).unwrap();
        fs::create_dir_all(root.path().join("system-artifacts")).unwrap();
        for (path, bytes) in [
            (
                "index.html",
                b"<script src=\"/assets/app.js\"></script>".as_slice(),
            ),
            ("assets/app.js", b"js".as_slice()),
            ("assets/app.js.br", b"br".as_slice()),
            ("assets/client.wasm", b"wasm".as_slice()),
            ("assets/client.wasm.gz", b"gz".as_slice()),
            (".vite/manifest.json", b"build-only".as_slice()),
            ("system-artifacts/inventory.txt", b"private".as_slice()),
        ] {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        stage_vite_output(root.path(), site.path()).unwrap();
        for path in [
            "index.html",
            "assets/app.js",
            "assets/app.js.br",
            "assets/client.wasm",
            "assets/client.wasm.gz",
        ] {
            assert!(site.path().join(path).is_file(), "missing {path}");
        }
        assert!(!site.path().join(".vite/manifest.json").exists());
        assert!(!site.path().join("system-artifacts/inventory.txt").exists());
    }

    #[test]
    fn vite_staging_requires_the_html_entry() {
        let root = tempdir().unwrap();
        let site = tempdir().unwrap();
        assert!(
            stage_vite_output(root.path(), site.path())
                .unwrap_err()
                .to_string()
                .contains("no index.html")
        );
    }

    #[test]
    fn public_assets_stage_for_the_no_bundle_local_case() {
        let public = tempdir().unwrap();
        let site = tempdir().unwrap();
        fs::write(public.path().join("favicon.ico"), b"favicon").unwrap();
        stage_public_tree(public.path(), site.path()).unwrap();
        assert_eq!(
            fs::read(site.path().join("favicon.ico")).unwrap(),
            b"favicon"
        );
    }
}
