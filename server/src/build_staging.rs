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
        // cov:ignore-start: A ReadDir item error requires concurrent filesystem mutation; rustix exposes no deterministic item-error injection seam.
        let entry = entry.map_err(|error| {
            BundleStageError(format!("reading {}: {error}", directory.display()))
        })?;
        // cov:ignore-stop
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

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs, io,
        path::{Path, PathBuf},
    };

    use csr_bundle::{Asset, Manifest, Representation, Role};

    use super::{
        copy_file, prepare_staging_with, reject_public_collisions, reject_public_collisions_below,
        stage_bundle, stage_public_tree, validate_shell,
    };

    fn shell(glue: &str, wasm: &str) -> String {
        format!(
            r#"<script>const __jaunderWasmUrl = "{wasm}"; window.__jaunderWasmFetch = fetch(__jaunderWasmUrl);</script>
<link rel="stylesheet" href="/style/jaunder.css" />
<script type="module">import {{initMeasured}} from "{glue}"; performance.mark("jaunder.module.before_init"); initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl);</script>"#
        )
    }

    fn bundle_asset(role: Role, extension: &str, bytes: &[u8]) -> Asset {
        let digest = csr_bundle::digest(bytes);
        let path = format!("pkg/{digest}.{extension}");
        let gzip = format!("{path}.gz");
        let brotli = format!("{path}.br");
        let representations = BTreeMap::from([
            (
                "identity".into(),
                Representation {
                    path: path.clone(),
                    sha256: digest.clone(),
                },
            ),
            (
                "gzip".into(),
                Representation {
                    path: gzip,
                    sha256: csr_bundle::digest(b"gzip"),
                },
            ),
            (
                "br".into(),
                Representation {
                    path: brotli,
                    sha256: csr_bundle::digest(b"brotli"),
                },
            ),
        ]);
        Asset {
            role: Some(role),
            path,
            sha256: digest,
            representations,
        }
    }

    fn write_bundle(root: &std::path::Path) -> Manifest {
        let manifest = Manifest {
            version: csr_bundle::VERSION,
            assets: vec![
                bundle_asset(Role::Glue, "js", b"glue"),
                bundle_asset(Role::Wasm, "wasm", b"wasm"),
            ],
        };
        for asset in &manifest.assets {
            for representation in asset.representations.values() {
                let bytes = match std::path::Path::new(&representation.path).extension() {
                    Some(extension) if extension == "gz" => b"gzip".as_slice(),
                    Some(extension) if extension == "br" => b"brotli".as_slice(),
                    Some(extension) if extension == "js" => b"glue".as_slice(),
                    _ => b"wasm".as_slice(),
                };
                let path = root.join(&representation.path);
                fs::create_dir_all(path.parent().expect("bundle parent")).expect("create parent");
                fs::write(path, bytes).expect("write representation");
            }
        }
        let glue = manifest.role(Role::Glue).expect("glue role");
        let wasm = manifest.role(Role::Wasm).expect("WASM role");
        fs::write(
            root.join("index.html"),
            shell(&format!("/{}", glue.path), &format!("/{}", wasm.path)),
        )
        .expect("write shell");
        manifest
    }

    #[test]
    fn stages_verified_bundle_and_nested_public_assets() {
        let bundle = tempfile::tempdir().expect("bundle root");
        let site = tempfile::tempdir().expect("site root");
        let public = tempfile::tempdir().expect("public root");
        let manifest = write_bundle(bundle.path());
        let stylesheet = public.path().join("style/jaunder.css");
        fs::create_dir_all(stylesheet.parent().expect("stylesheet parent"))
            .expect("create stylesheet parent");
        fs::write(&stylesheet, "body {}").expect("write stylesheet");

        stage_bundle(bundle.path(), site.path(), public.path(), &manifest).expect("stage bundle");

        manifest
            .verify_bundle(site.path())
            .expect("staged bundle verifies");
        assert_eq!(
            fs::read_to_string(site.path().join("style/jaunder.css")).unwrap(),
            "body {}"
        );
    }

    #[test]
    fn rejects_nested_public_bundle_collision() {
        let bundle = tempfile::tempdir().expect("bundle root");
        let public = tempfile::tempdir().expect("public root");
        let manifest = write_bundle(bundle.path());
        let glue = manifest.role(Role::Glue).expect("glue role");
        let collision = public.path().join(&glue.path);
        fs::create_dir_all(collision.parent().expect("collision parent"))
            .expect("create collision parent");
        fs::write(collision, "replacement").expect("write collision");

        let error = reject_public_collisions(public.path(), &manifest).expect_err("collision");

        assert!(
            error
                .to_string()
                .contains("overwrites a reserved CSR bundle path")
        );
    }

    #[test]
    fn shell_requires_unique_urls_and_required_order() {
        let glue = "/pkg/glue.js";
        let wasm = "/pkg/module.wasm";
        validate_shell(&shell(glue, wasm), glue, wasm).expect("valid shell");

        let duplicate = format!("{}\n{glue}", shell(glue, wasm));
        assert!(
            validate_shell(&duplicate, glue, wasm)
                .expect_err("duplicate glue URL")
                .to_string()
                .contains("exactly one glue URL")
        );
        let misordered = format!(
            r#"<link rel="stylesheet" href="/style/jaunder.css" />window.__jaunderWasmFetch = fetch import {{initMeasured}} performance.mark initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl) {glue} {wasm}"#
        );
        assert!(
            validate_shell(&misordered, glue, wasm)
                .expect_err("misordered shell")
                .to_string()
                .contains("does not preserve")
        );
    }

    #[test]
    fn public_tree_ignores_missing_source_and_copies_nested_assets() {
        let source = tempfile::tempdir().expect("source root");
        let destination = tempfile::tempdir().expect("destination root");
        let nested = source.path().join("nested/asset");
        fs::create_dir_all(nested.parent().expect("nested parent")).expect("create nested parent");
        fs::write(&nested, "asset").expect("write asset");

        stage_public_tree(source.path(), destination.path()).expect("copy public tree");
        stage_public_tree(&source.path().join("missing"), destination.path())
            .expect("ignore missing source");

        assert_eq!(
            fs::read_to_string(destination.path().join("nested/asset")).unwrap(),
            "asset"
        );
    }
    #[test]
    fn staging_preparation_recreates_then_stages() {
        let site = tempfile::tempdir().expect("temp dir");
        let path = site.path().join("site");
        let mut staged = false;

        prepare_staging_with(&path, |_| Ok(()), |_| Ok(()), || staged = true)
            .expect("prepare staging");

        assert!(staged);
    }

    #[test]
    fn staging_preparation_reports_remove_and_create_failures() {
        let path = Path::new("/injected/site");
        let remove_error = prepare_staging_with(
            path,
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "remove denied",
                ))
            },
            |_| unreachable!("remove failure must stop before creation"),
            || unreachable!("remove failure must stop before staging"),
        )
        .expect_err("remove failure");
        assert_eq!(
            remove_error.to_string(),
            "removing staging directory /injected/site: remove denied"
        );
        assert_eq!(
            std::error::Error::source(&remove_error)
                .map(ToString::to_string)
                .as_deref(),
            Some("remove denied")
        );

        let create_error = prepare_staging_with(
            path,
            |_| Err(io::Error::new(io::ErrorKind::NotFound, "absent")),
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "create denied",
                ))
            },
            || unreachable!("create failure must stop before staging"),
        )
        .expect_err("create failure");
        assert_eq!(
            create_error.to_string(),
            "creating staging directory /injected/site: create denied"
        );
    }

    #[test]
    fn staging_handles_missing_public_tree_and_missing_shell_positions() {
        let bundle = tempfile::tempdir().expect("bundle");
        let site = tempfile::tempdir().expect("site");
        let manifest = write_bundle(bundle.path());
        stage_bundle(
            bundle.path(),
            site.path(),
            &bundle.path().join("missing"),
            &manifest,
        )
        .expect("missing public tree is permitted");

        for required in [
            "window.__jaunderWasmFetch = fetch",
            r#"<link rel="stylesheet" href="/style/jaunder.css" />"#,
            "import {initMeasured}",
            "performance.mark",
            "initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl)",
        ] {
            let error =
                validate_shell(required, "glue", "wasm").expect_err("missing required position");
            assert!(
                error.to_string().contains("missing") || error.to_string().contains("exactly one")
            );
        }

        let shell_with_unique_urls = shell("/pkg/glue.js", "/pkg/module.wasm");
        for required in [
            "performance.mark",
            "initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl)",
        ] {
            let incomplete = shell_with_unique_urls.replacen(required, "", 1);
            let error = validate_shell(&incomplete, "/pkg/glue.js", "/pkg/module.wasm")
                .expect_err("unique URLs do not excuse a missing required shell marker");
            assert!(error.to_string().contains("missing"));
        }
    }
    #[test]
    fn staging_reports_missing_tree_and_public_collision() {
        let manifest = Manifest {
            version: csr_bundle::VERSION,
            assets: Vec::new(),
        };
        let missing = Path::new("/definitely-missing-jaunder-public-tree");
        let error = reject_public_collisions_below(missing, Path::new(""), &BTreeSet::new())
            .expect_err("missing tree");
        assert!(error.to_string().contains("reading"));

        let bundle = tempfile::tempdir().expect("bundle");
        let site = tempfile::tempdir().expect("site");
        fs::write(bundle.path().join("index.html"), "shell").expect("shell");
        let error = stage_bundle(bundle.path(), site.path(), bundle.path(), &manifest)
            .expect_err("public shell collision");
        assert!(error.to_string().contains("overwrites"));
    }

    #[test]
    fn staging_reports_copy_and_invalid_staged_bundle_failures() {
        let destination = tempfile::tempdir().expect("destination");
        let error = copy_file(
            Path::new("/definitely-missing-jaunder-staging-source"),
            &destination.path().join("copied"),
        )
        .expect_err("missing source copy");
        assert!(error.to_string().contains("copying"));

        let bundle = tempfile::tempdir().expect("bundle");
        let site = tempfile::tempdir().expect("site");
        let public = tempfile::tempdir().expect("public");
        let manifest = write_bundle(bundle.path());
        let glue = manifest.role(Role::Glue).expect("glue role");
        fs::write(bundle.path().join(&glue.path), "mutated").expect("mutate verified source");
        let error = stage_bundle(bundle.path(), site.path(), public.path(), &manifest)
            .expect_err("invalid staged bundle");
        assert!(error.to_string().contains("invalid staged bundle"));
    }
    #[cfg(unix)]
    #[test]
    fn public_collision_rejection_reports_non_utf8_asset_path() {
        use std::os::unix::ffi::OsStringExt;

        let public = tempfile::tempdir().expect("public");
        let non_utf8 = PathBuf::from(std::ffi::OsString::from_vec(vec![0xff]));
        fs::write(public.path().join(&non_utf8), "asset").expect("write non-UTF8 asset");

        let error = reject_public_collisions_below(public.path(), Path::new(""), &BTreeSet::new())
            .expect_err("non-UTF8 public asset must be rejected");

        assert!(error.to_string().contains("public asset path is not UTF-8"));
    }
    #[test]
    fn staging_propagates_missing_declared_representation_copy_failure() {
        let bundle = tempfile::tempdir().expect("bundle");
        let site = tempfile::tempdir().expect("site");
        let public = tempfile::tempdir().expect("public");
        let mut manifest = write_bundle(bundle.path());
        manifest.assets[0].representations.insert(
            "missing".into(),
            Representation {
                path: "pkg/absent.js".into(),
                sha256: csr_bundle::digest(b"absent"),
            },
        );

        let error = stage_bundle(bundle.path(), site.path(), public.path(), &manifest)
            .expect_err("missing declared representation must fail staging");

        assert!(error.to_string().contains("copying"));
    }
}
