//! Filesystem admission for a theme repository.
//!
//! This is the sole adapter from an author-controlled directory to the closed
//! Theme Package boundary. It admits only root package inputs and the closed
//! `assets/` tree, then delegates every package and stylesheet rule to
//! [`crate::theme_package`].

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags, fstat, openat, statat};
use thiserror::Error;

use crate::theme_package::{
    CompiledThemeRevision, ThemePackageError, ThemePackageLimits, export_theme_package,
    percent_encode_asset_path, validate_theme_package,
};

/// Why a repository directory cannot be admitted as a Theme Package.
#[derive(Debug, Error)]
pub enum ThemeRepositoryError {
    #[error("repository entry `{path}` is not a regular file or directory")]
    EntryType { path: PathBuf },
    #[error("repository entry `{path}` has no UTF-8 name")]
    NonUtf8Name { path: PathBuf },
    #[error("repository entry `{path}` exceeds the per-file byte limit")]
    FileLimit { path: PathBuf },
    #[error("repository exceeds the file-count limit")]
    FileCountLimit,
    #[error("repository exceeds the expanded byte limit")]
    TotalLimit,
    #[error("repository I/O failed at `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("repository stylesheet `{path}` failed validation: {source}")]
    Stylesheet {
        path: PathBuf,
        #[source]
        source: ThemePackageError,
    },
    #[error(transparent)]
    Package(#[from] ThemePackageError),
}

/// A repository that has passed filesystem admission, package validation, and CSS compilation.
///
/// Its fields stay private so package or compiled-preview bytes cannot be obtained
/// from unchecked directory contents.
#[derive(Debug)]
pub struct AcceptedThemeRepository {
    package_bytes: Vec<u8>,
    revision: CompiledThemeRevision,
}

impl AcceptedThemeRepository {
    /// Returns the canonical deterministic portable archive.
    #[must_use]
    pub fn package_bytes(&self) -> &[u8] {
        &self.package_bytes
    }

    /// Returns the compiled package revision for a trusted local preview.
    #[must_use]
    pub fn revision(&self) -> &CompiledThemeRevision {
        &self.revision
    }
}

/// Admits a theme repository through the existing Theme Package validator and compiler.
///
/// Root support files are ignored. The only consumed root files are `theme.json`
/// and `style.css`; `assets/`, when present, is recursively closed to regular files.
/// CSS asset references compile to deterministic package-local paths.
///
/// # Errors
///
/// Returns an error for filesystem admission, package validation, or CSS compilation failures.
pub fn accept_theme_repository(
    root: &Path,
) -> Result<AcceptedThemeRepository, ThemeRepositoryError> {
    accept_theme_repository_with_limits(root, ThemePackageLimits::default())
}

fn accept_theme_repository_with_limits(
    root: &Path,
    limits: ThemePackageLimits,
) -> Result<AcceptedThemeRepository, ThemeRepositoryError> {
    let root_dir = open_directory(CWD, root, root)?;
    let manifest = read_required(&root_dir, root, "theme.json", limits.max_file_bytes)?;
    let css = read_required(&root_dir, root, "style.css", limits.max_file_bytes)?;
    let mut total = add_bytes(0, manifest.len(), limits)?;
    total = add_bytes(total, css.len(), limits)?;
    let mut files = 2;
    if files > limits.max_files {
        return Err(ThemeRepositoryError::FileCountLimit);
    }

    let mut assets = BTreeMap::new();
    let asset_root = root.join("assets");
    match statat(
        root_dir.fd().map_err(|source| io_error(root, source))?,
        "assets",
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Ok(metadata) => {
            if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
                return Err(ThemeRepositoryError::EntryType { path: asset_root });
            }
            let assets_dir = open_directory(
                root_dir.fd().map_err(|source| io_error(root, source))?,
                Path::new("assets"),
                &asset_root,
            )?;
            collect_assets(
                &assets_dir,
                &asset_root,
                Path::new("assets"),
                &mut assets,
                &mut total,
                &mut files,
                limits,
            )?;
        }
        Err(error) if io::Error::from(error).kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error(&asset_root, source)), // cov:ignore: deterministic host tests cannot make statat fail for a present assets entry without changing it between lookup and stat.
    }

    let source = export_theme_package(&manifest, &css, &assets)?;
    let validated = validate_theme_package(&source, limits)?;
    let package_bytes = validated.export_archive()?;
    let asset_urls = validated
        .asset_paths()
        .map(|path| {
            (
                path.to_owned(),
                format!("/theme-assets/{}", percent_encode_asset_path(path)),
            )
        })
        .collect();
    let revision = validated.compile(&asset_urls, limits).map_err(|source| {
        ThemeRepositoryError::Stylesheet {
            path: root.join("style.css"),
            source,
        }
    })?;
    Ok(AcceptedThemeRepository {
        package_bytes,
        revision,
    })
}

fn collect_assets(
    directory: &Dir,
    display_directory: &Path,
    relative_directory: &Path,
    assets: &mut BTreeMap<String, Vec<u8>>,
    total: &mut usize,
    files: &mut usize,
    limits: ThemePackageLimits,
) -> Result<(), ThemeRepositoryError> {
    let mut directory = Dir::read_from(
        directory
            .fd()
            .map_err(|source| io_error(display_directory, source))?,
    )
    .map_err(|source| io_error(display_directory, source))?;
    while let Some(entry) = directory.read() {
        let entry = entry.map_err(|source| io_error(display_directory, source))?;
        let raw_name = entry.file_name();
        if raw_name.to_bytes() == b"." || raw_name.to_bytes() == b".." {
            continue;
        }
        let entry_path = display_directory.join(std::ffi::OsStr::from_bytes(raw_name.to_bytes()));
        let name = raw_name
            .to_str()
            .map_err(|_| ThemeRepositoryError::NonUtf8Name {
                path: entry_path.clone(),
            })?;
        let metadata = statat(
            directory
                .fd()
                .map_err(|source| io_error(display_directory, source))?,
            raw_name,
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|source| io_error(&entry_path, source))?;
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        if file_type.is_dir() {
            let child = open_directory(
                directory
                    .fd()
                    .map_err(|source| io_error(display_directory, source))?,
                Path::new(name),
                &entry_path,
            )?;
            collect_assets(
                &child,
                &entry_path,
                &relative_directory.join(name),
                assets,
                total,
                files,
                limits,
            )?;
            continue; // cov:ignore: llvm-cov records the recursive call but omits this exercised directory-entry loop continuation.
        }
        if !file_type.is_file() {
            return Err(ThemeRepositoryError::EntryType { path: entry_path });
        }
        *files = files
            .checked_add(1)
            .ok_or(ThemeRepositoryError::FileCountLimit)?;
        if *files > limits.max_files {
            return Err(ThemeRepositoryError::FileCountLimit);
        }
        let bytes = read_regular(
            directory
                .fd()
                .map_err(|source| io_error(display_directory, source))?,
            raw_name,
            &entry_path,
            limits.max_file_bytes,
        )?;
        *total = add_bytes(*total, bytes.len(), limits)?;
        let asset_path = relative_directory.join(name);
        let asset_name = asset_path
            .to_str()
            .ok_or(ThemeRepositoryError::NonUtf8Name { path: entry_path })?
            .replace(std::path::MAIN_SEPARATOR, "/");
        assets.insert(asset_name, bytes);
    }
    Ok(())
}

fn read_required(
    root: &Dir,
    root_path: &Path,
    name: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, ThemeRepositoryError> {
    read_regular(
        root.fd().map_err(|source| io_error(root_path, source))?,
        name,
        &root_path.join(name),
        max_bytes,
    )
}

fn open_directory<Fd: std::os::fd::AsFd>(
    parent: Fd,
    name: &Path,
    display_path: &Path,
) -> Result<Dir, ThemeRepositoryError> {
    let descriptor = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|source| io_error(display_path, source))?;
    Dir::new(descriptor).map_err(|source| io_error(display_path, source))
}

fn read_regular<Fd: std::os::fd::AsFd>(
    parent: Fd,
    name: impl rustix::path::Arg + Copy,
    display_path: &Path,
    max_bytes: usize,
) -> Result<Vec<u8>, ThemeRepositoryError> {
    let metadata = statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|source| io_error(display_path, source))?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file() {
        return Err(ThemeRepositoryError::EntryType {
            path: display_path.to_owned(),
        });
    }
    if u64::try_from(metadata.st_size).unwrap_or(u64::MAX) > max_bytes as u64 {
        return Err(ThemeRepositoryError::FileLimit {
            path: display_path.to_owned(),
        });
    }
    let descriptor = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|source| io_error(display_path, source))?;
    let opened = fstat(&descriptor).map_err(|source| io_error(display_path, source))?;
    if !FileType::from_raw_mode(opened.st_mode).is_file() {
        // cov:ignore-start: a non-file descriptor after a successful regular-file stat requires replacing the entry between statat and openat.
        return Err(ThemeRepositoryError::EntryType {
            path: display_path.to_owned(),
        });
        // cov:ignore-stop
    }
    let opened_len = u64::try_from(opened.st_size).unwrap_or(u64::MAX);
    if opened_len > max_bytes as u64 {
        // cov:ignore-start: a larger descriptor after the pre-open size check requires a concurrent file replacement or resize.
        return Err(ThemeRepositoryError::FileLimit {
            path: display_path.to_owned(),
        });
        // cov:ignore-stop
    }
    let file = fs::File::from(descriptor);
    let capacity = usize::try_from(opened_len).unwrap_or(max_bytes);
    let mut bytes = Vec::with_capacity(capacity);
    let read_limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(display_path, source))?;
    if bytes.len() > max_bytes {
        // cov:ignore-start: exceeding the bounded read after both size checks requires the file to grow while it is read.
        return Err(ThemeRepositoryError::FileLimit {
            path: display_path.to_owned(),
        });
        // cov:ignore-stop
    }
    Ok(bytes)
}

fn add_bytes(
    total: usize,
    bytes: usize,
    limits: ThemePackageLimits,
) -> Result<usize, ThemeRepositoryError> {
    let total = total
        .checked_add(bytes)
        .ok_or(ThemeRepositoryError::TotalLimit)?;
    if total > limits.max_expanded_bytes {
        Err(ThemeRepositoryError::TotalLimit)
    } else {
        Ok(total)
    }
}

fn io_error(path: &Path, source: impl Into<io::Error>) -> ThemeRepositoryError {
    ThemeRepositoryError::Io {
        path: path.to_owned(),
        source: source.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str =
        r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#;

    fn repository() -> tempfile::TempDir {
        let repository = tempfile::tempdir().expect("repository");
        fs::write(repository.path().join("theme.json"), MANIFEST).expect("manifest");
        fs::write(
            repository.path().join("style.css"),
            "body { color: black; }",
        )
        .expect("css");
        repository
    }

    #[test]
    fn accepts_root_support_files_and_emits_stable_package_bytes() {
        let repository = repository();
        fs::write(repository.path().join("README.md"), "support").expect("support file");
        fs::create_dir(repository.path().join(".github")).expect("support directory");
        let first = accept_theme_repository(repository.path()).expect("first acceptance");
        let second = accept_theme_repository(repository.path()).expect("second acceptance");
        assert_eq!(first.package_bytes(), second.package_bytes());
        assert!(!first.package_bytes().is_empty());
    }

    #[test]
    fn rejects_limits_below_the_required_root_member_count() {
        let repository = repository();
        let limits = ThemePackageLimits {
            max_files: 1,
            ..ThemePackageLimits::default()
        };

        assert!(matches!(
            accept_theme_repository_with_limits(repository.path(), limits),
            Err(ThemeRepositoryError::FileCountLimit)
        ));
    }

    #[test]
    fn rejects_directory_valued_required_members() {
        let repository = repository();
        let style = repository.path().join("style.css");
        fs::remove_file(&style).expect("stylesheet");
        fs::create_dir(&style).expect("stylesheet directory");

        assert!(matches!(
            accept_theme_repository(repository.path()),
            Err(ThemeRepositoryError::EntryType { path }) if path == style
        ));
    }

    #[test]
    fn rejects_required_members_larger_than_the_file_limit() {
        let repository = repository();
        let manifest = repository.path().join("theme.json");
        let limits = ThemePackageLimits {
            max_file_bytes: MANIFEST.len() - 1,
            ..ThemePackageLimits::default()
        };

        assert!(matches!(
            accept_theme_repository_with_limits(repository.path(), limits),
            Err(ThemeRepositoryError::FileLimit { path }) if path == manifest
        ));
    }

    #[test]
    fn rejects_missing_and_undeclared_assets() {
        let missing = repository();
        fs::write(
            missing.path().join("theme.json"),
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{"assets/logo.png":"image/png"},"defaults":{}}"#,
        )
        .expect("manifest");
        assert!(accept_theme_repository(missing.path()).is_err());

        let undeclared = repository();
        fs::create_dir(undeclared.path().join("assets")).expect("assets");
        fs::write(undeclared.path().join("assets/logo.png"), b"not a png").expect("asset");
        assert!(matches!(
            accept_theme_repository(undeclared.path()),
            Err(ThemeRepositoryError::Package(ThemePackageError::UndeclaredAsset(path))) if path == "assets/logo.png"
        ));
    }

    #[test]
    fn compiles_declared_assets_to_percent_encoded_local_urls() {
        let repository = repository();
        fs::write(
            repository.path().join("theme.json"),
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{"assets/pixel ?#.avif":"image/avif"},"defaults":{}}"#,
        )
        .expect("manifest");
        fs::write(
            repository.path().join("style.css"),
            r#"body { background-image: url("assets/pixel ?#.avif"); }"#,
        )
        .expect("css");
        fs::create_dir(repository.path().join("assets")).expect("assets");
        fs::write(
            repository.path().join("assets/pixel ?#.avif"),
            include_bytes!("theme_package/fixtures/one-pixel.avif"),
        )
        .expect("asset");
        let accepted = accept_theme_repository(repository.path()).expect("accepted repository");
        assert!(
            std::str::from_utf8(accepted.revision().css().bytes())
                .expect("compiled stylesheet")
                .contains("/theme-assets/assets/pixel%20%3F%23.avif")
        );
    }

    #[test]
    fn counts_actual_member_bytes_once_and_includes_required_root_members() {
        let repository = repository();
        let manifest = r#"{"defaults":{},"assets":{"assets/pixel.avif":"image/avif"},"style_contract":1,"name":"Paper","schema":1}"#;
        let css = "body { background-image: url(assets/pixel.avif); }";
        let asset = include_bytes!("theme_package/fixtures/one-pixel.avif");
        fs::write(repository.path().join("theme.json"), manifest).expect("manifest");
        fs::write(repository.path().join("style.css"), css).expect("css");
        fs::create_dir(repository.path().join("assets")).expect("assets");
        fs::write(repository.path().join("assets/pixel.avif"), asset).expect("asset");
        let total = manifest.len() + css.len() + asset.len();
        let limits = ThemePackageLimits {
            max_expanded_bytes: total,
            max_files: 3,
            ..ThemePackageLimits::default()
        };
        let accepted =
            accept_theme_repository_with_limits(repository.path(), limits).expect("exact limit");
        let package = validate_theme_package(accepted.package_bytes(), limits).expect("package");
        assert_eq!(
            package.canonical_manifest(),
            br#"{"assets":{"assets/pixel.avif":"image/avif"},"defaults":{},"name":"Paper","schema":1,"style_contract":1}"#
        );

        assert!(matches!(
            accept_theme_repository_with_limits(
                repository.path(),
                ThemePackageLimits {
                    max_expanded_bytes: total - 1,
                    ..limits
                }
            ),
            Err(ThemeRepositoryError::TotalLimit)
        ));
        assert!(matches!(
            accept_theme_repository_with_limits(
                repository.path(),
                ThemePackageLimits {
                    max_files: 2,
                    ..limits
                }
            ),
            Err(ThemeRepositoryError::FileCountLimit)
        ));
    }

    #[test]
    fn rejects_manifest_asset_paths_that_cannot_be_safe_package_members() {
        let repository = repository();
        fs::write(
            repository.path().join("theme.json"),
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{"assets/../escaped.png":"image/png"},"defaults":{}}"#,
        )
        .expect("manifest");
        assert!(matches!(
            accept_theme_repository(repository.path()),
            Err(ThemeRepositoryError::Package(
                ThemePackageError::Manifest(_) | ThemePackageError::Member(_)
            ))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_root_nested_symlinks_and_non_regular_asset_entries() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;

        let root_link = repository();
        symlink("theme.json", root_link.path().join("assets")).expect("root symlink");
        assert!(matches!(
            accept_theme_repository(root_link.path()),
            Err(ThemeRepositoryError::EntryType { .. })
        ));

        let nested_link = repository();
        fs::create_dir(nested_link.path().join("assets")).expect("assets");
        fs::create_dir(nested_link.path().join("assets/nested")).expect("nested assets");
        symlink(
            "../../theme.json",
            nested_link.path().join("assets/nested/link"),
        )
        .expect("nested symlink");
        assert!(matches!(
            accept_theme_repository(nested_link.path()),
            Err(ThemeRepositoryError::EntryType { .. })
        ));

        let special = repository();
        fs::create_dir(special.path().join("assets")).expect("assets");
        let _socket = UnixListener::bind(special.path().join("assets/socket")).expect("socket");
        assert!(matches!(
            accept_theme_repository(special.path()),
            Err(ThemeRepositoryError::EntryType { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_asset_names() {
        use std::os::unix::ffi::OsStringExt;

        let repository = repository();
        let assets = repository.path().join("assets");
        let name = std::ffi::OsString::from_vec(b"invalid-\xFF.avif".to_vec());
        let path = assets.join(&name);
        fs::create_dir(&assets).expect("assets");
        fs::write(&path, b"asset").expect("asset");

        assert!(matches!(
            accept_theme_repository(repository.path()),
            Err(ThemeRepositoryError::NonUtf8Name { path: error_path }) if error_path == path
        ));
    }

    #[cfg(unix)]
    fn deny_read_access(path: &Path) -> fs::Permissions {
        use std::os::unix::fs::PermissionsExt;

        let original = fs::metadata(path).expect("permission target").permissions();
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).expect("deny read access");
        original
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unreadable_assets_directory() {
        let repository = repository();
        let assets = repository.path().join("assets");
        fs::create_dir(&assets).expect("assets");
        let original_permissions = deny_read_access(&assets);

        let result = accept_theme_repository(repository.path());

        fs::set_permissions(&assets, original_permissions).expect("restore assets permissions");
        assert!(matches!(
            result,
            Err(ThemeRepositoryError::Io { path, source })
                if path == assets && source.kind() == io::ErrorKind::PermissionDenied
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unreadable_nested_assets_directory() {
        let repository = repository();
        let assets = repository.path().join("assets");
        let nested = assets.join("nested");
        fs::create_dir(&assets).expect("assets");
        fs::create_dir(&nested).expect("nested assets");
        let original_permissions = deny_read_access(&nested);

        let result = accept_theme_repository(repository.path());

        fs::set_permissions(&nested, original_permissions).expect("restore nested permissions");
        assert!(matches!(
            result,
            Err(ThemeRepositoryError::Io { path, source })
                if path == nested && source.kind() == io::ErrorKind::PermissionDenied
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unreadable_asset_files_after_metadata_lookup() {
        let repository = repository();
        let assets = repository.path().join("assets");
        let asset = assets.join("asset.txt");
        fs::create_dir(&assets).expect("assets");
        fs::write(&asset, "asset").expect("asset");
        let original_permissions = deny_read_access(&asset);

        let result = accept_theme_repository(repository.path());

        fs::set_permissions(&asset, original_permissions).expect("restore asset permissions");
        assert!(matches!(
            result,
            Err(ThemeRepositoryError::Io { path, source })
                if path == asset && source.kind() == io::ErrorKind::PermissionDenied
        ));
    }
}
