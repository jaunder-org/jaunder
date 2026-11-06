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
    use std::{cell::RefCell, io};
    use tempfile::tempdir;

    #[test]
    fn staging_runs_in_order_and_aborts_at_the_failed_operation() {
        for failure in [None, Some("removing"), Some("creating")] {
            let root = tempdir().unwrap();
            let site = root.path().join("site");
            let operations = RefCell::new(Vec::new());
            let result = prepare_staging_with(
                &site,
                |_| {
                    operations.borrow_mut().push("removing");
                    if failure == Some("removing") {
                        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "blocked"));
                    }
                    Ok(())
                },
                |path| {
                    operations.borrow_mut().push("creating");
                    if failure == Some("creating") {
                        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "blocked"));
                    }
                    fs::create_dir_all(path)
                },
                || {
                    operations.borrow_mut().push("staging");
                    fs::write(site.join("index.html"), b"entry").unwrap();
                },
            );
            if let Some(operation) = failure {
                let error = result.unwrap_err();
                assert_eq!(error.operation, operation);
                assert_eq!(error.path, site);
                assert_eq!(
                    error.to_string(),
                    format!("{operation} staging directory {}: blocked", site.display())
                );
                assert_eq!(
                    std::error::Error::source(&error).unwrap().to_string(),
                    "blocked"
                );
                assert!(!site.exists());
            } else {
                result.unwrap();
                assert_eq!(fs::read(site.join("index.html")).unwrap(), b"entry");
            }
            let expected = match failure {
                Some("removing") => vec!["removing"],
                Some(_) => vec!["removing", "creating"],
                None => vec!["removing", "creating", "staging"],
            };
            assert_eq!(*operations.borrow(), expected);
        }
    }

    #[test]
    fn staging_replaces_stale_outputs_and_tolerates_an_absent_directory() {
        let root = tempdir().unwrap();
        let site = root.path().join("site");
        for _ in 0..2 {
            prepare_staging_with(
                &site,
                |path| fs::remove_dir_all(path),
                |path| fs::create_dir_all(path),
                || {
                    assert!(!site.join("stale.js").exists());
                    fs::write(site.join("index.html"), b"new entry").unwrap();
                },
            )
            .unwrap();
            assert_eq!(fs::read(site.join("index.html")).unwrap(), b"new entry");
            fs::write(site.join("stale.js"), b"old chunk").unwrap();
        }
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
    fn vite_staging_reports_source_and_destination_when_copying_fails() {
        let root = tempdir().unwrap();
        let site = tempdir().unwrap();
        fs::write(root.path().join("index.html"), b"entry").unwrap();
        fs::create_dir(site.path().join("index.html")).unwrap();
        let error = stage_vite_output(root.path(), site.path()).unwrap_err();
        assert!(error.to_string().starts_with(&format!(
            "copying {} to {}:",
            root.path().join("index.html").display(),
            site.path().join("index.html").display()
        )));
        assert!(site.path().join("index.html").is_dir());
    }

    #[test]
    fn absent_public_assets_leave_the_empty_local_site_unchanged() {
        let root = tempdir().unwrap();
        let site = tempdir().unwrap();
        stage_public_tree(&root.path().join("absent"), site.path()).unwrap();
        assert_eq!(fs::read_dir(site.path()).unwrap().count(), 0);
    }

    #[test]
    fn public_assets_stage_for_the_no_bundle_local_case() {
        let public = tempdir().unwrap();
        let site = tempdir().unwrap();
        fs::write(public.path().join("favicon.ico"), b"favicon").unwrap();
        fs::create_dir(public.path().join("nested")).unwrap();
        fs::write(public.path().join("nested/asset.txt"), b"nested asset").unwrap();
        stage_public_tree(public.path(), site.path()).unwrap();
        assert_eq!(
            fs::read(site.path().join("nested/asset.txt")).unwrap(),
            b"nested asset"
        );
        assert_eq!(
            fs::read(site.path().join("favicon.ico")).unwrap(),
            b"favicon"
        );
    }
}
