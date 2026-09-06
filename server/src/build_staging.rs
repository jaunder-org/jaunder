use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use csr_bundle::{Manifest, Role};

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

/// Staging failure for a verified CSR bundle.
#[derive(Debug)]
pub struct BundleStageError(String);

impl fmt::Display for BundleStageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BundleStageError {}

/// Copy the verified bundle and public assets, then verify the exact staged result.
///
/// # Errors
///
/// Returns an error when a public asset would replace a reserved bundle path, a copy
/// fails, or the final staged inventory, digests, or shell disagree with the manifest.
pub fn stage_bundle(
    root: &Path,
    site: &Path,
    public_src: &Path,
    manifest: &Manifest,
) -> Result<(), BundleStageError> {
    reject_public_collisions(public_src, manifest)?;
    copy_file(&root.join("index.html"), &site.join("index.html"))?;
    for asset in &manifest.assets {
        copy_file(&root.join(&asset.path), &site.join(&asset.path))?;
        for representation in asset.representations.values() {
            if representation.path != asset.path {
                copy_file(
                    &root.join(&representation.path),
                    &site.join(&representation.path),
                )?;
            }
        }
    }
    if public_src.is_dir() {
        copy_tree(public_src, site)?;
    }
    verify_staged_bundle(site, manifest)
}

fn reject_public_collisions(
    public_src: &Path,
    manifest: &Manifest,
) -> Result<(), BundleStageError> {
    if !public_src.is_dir() {
        return Ok(());
    }
    let mut reserved = BTreeSet::from(["index.html", "manifest.json"]);
    for asset in &manifest.assets {
        reserved.insert(asset.path.as_str());
        for representation in asset.representations.values() {
            reserved.insert(representation.path.as_str());
        }
    }
    reject_public_collisions_below(public_src, Path::new(""), &reserved)
}

fn reject_public_collisions_below(
    directory: &Path,
    relative: &Path,
    reserved: &BTreeSet<&str>,
) -> Result<(), BundleStageError> {
    for entry in fs::read_dir(directory)
        .map_err(|error| BundleStageError(format!("reading {}: {error}", directory.display())))?
    {
        let entry = entry.map_err(|error| {
            BundleStageError(format!("reading {}: {error}", directory.display()))
        })?;
        let path = entry.path();
        let child_relative = relative.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| BundleStageError(format!("stating {}: {error}", path.display())))?;
        if file_type.is_dir() {
            reject_public_collisions_below(&path, &child_relative, reserved)?;
        } else {
            let relative = child_relative.to_str().ok_or_else(|| {
                BundleStageError(format!(
                    "public asset path is not UTF-8: {}",
                    child_relative.display()
                ))
            })?;
            if reserved.contains(relative) {
                return Err(BundleStageError(format!(
                    "public asset {relative} overwrites a reserved CSR bundle path"
                )));
            }
        }
    }
    Ok(())
}

fn verify_staged_bundle(site: &Path, manifest: &Manifest) -> Result<(), BundleStageError> {
    manifest.verify_bundle(site).map_err(|error| {
        BundleStageError(format!("invalid staged bundle {}: {error}", site.display()))
    })?;
    let shell = fs::read_to_string(site.join("index.html"))
        .map_err(|error| BundleStageError(format!("reading staged index.html: {error}")))?;
    let glue = manifest
        .role(Role::Glue)
        .map_err(|error| BundleStageError(format!("verified glue role missing: {error}")))?;
    let wasm = manifest
        .role(Role::Wasm)
        .map_err(|error| BundleStageError(format!("verified WASM role missing: {error}")))?;
    validate_shell(
        &shell,
        &format!("/{}", glue.path),
        &format!("/{}", wasm.path),
    )
}

fn validate_shell(shell: &str, glue: &str, wasm: &str) -> Result<(), BundleStageError> {
    for (role, url) in [("glue", glue), ("WASM", wasm)] {
        if shell.matches(url).count() != 1 {
            return Err(BundleStageError(format!(
                "staged shell must contain exactly one {role} URL"
            )));
        }
    }
    let early_fetch = shell
        .find("window.__jaunderWasmFetch = fetch")
        .ok_or_else(|| BundleStageError("staged shell is missing the early WASM fetch".into()))?;
    let stylesheet = shell
        .find(r#"<link rel="stylesheet" href="/style/jaunder.css" />"#)
        .ok_or_else(|| BundleStageError("staged shell is missing the base stylesheet".into()))?;
    let import = shell
        .find("import {initMeasured}")
        .ok_or_else(|| BundleStageError("staged shell is missing the glue import".into()))?;
    let mark = shell.find("performance.mark").ok_or_else(|| {
        BundleStageError("staged shell is missing the initialization mark".into())
    })?;
    let init = shell
        .find("initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl)")
        .ok_or_else(|| {
            BundleStageError("staged shell is missing the WASM fallback initializer".into())
        })?;
    if early_fetch < stylesheet && stylesheet < import && import < mark && mark < init {
        Ok(())
    } else {
        Err(BundleStageError(
            "staged shell does not preserve early-fetch, fallback, and initialization ordering"
                .into(),
        ))
    }
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
