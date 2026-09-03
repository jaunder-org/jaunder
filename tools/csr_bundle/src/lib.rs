//! The build-only contract for content-addressed CSR runtime bundles.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Current manifest schema version.
pub const VERSION: u32 = 1;

/// A logical runtime asset with a unique semantic role.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Glue,
    Wasm,
}

/// The bytes selected by content negotiation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Representation {
    pub path: String,
    pub sha256: String,
}

/// One logical runtime asset and every representation the server may serve.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    pub path: String,
    pub sha256: String,
    pub representations: BTreeMap<String, Representation>,
}

/// The sole naming contract between CSR bundle production and its consumers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("manifest version must be {VERSION}, found {0}")]
    Version(u32),
    #[error("unsafe bundle-relative path: {0}")]
    UnsafePath(String),
    #[error("duplicate manifest path: {0}")]
    DuplicatePath(String),
    #[error("duplicate role: {0:?}")]
    DuplicateRole(Role),
    #[error("missing required role: {0:?}")]
    MissingRole(Role),
    #[error("missing required representation {encoding:?} for {path}")]
    MissingRepresentation {
        path: String,
        encoding: &'static str,
    },
    #[error("invalid SHA-256 digest for {0}")]
    InvalidDigest(String),
    #[error("digest mismatch for {path}: manifest {expected}, actual {actual}")]
    DigestMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("missing bundle file: {0}")]
    MissingFile(String),
    #[error("unexpected bundle file: {0}")]
    UnexpectedFile(String),
    #[error("invalid manifest JSON: {0}")]
    InvalidManifest(String),
    #[error("asset path is not content-addressed: {0}")]
    NonContentAddressedPath(String),
    #[error("unsupported representation encoding: {0}")]
    UnsupportedRepresentation(String),
    #[error("I/O while reading bundle: {0}")]
    Io(#[from] std::io::Error),
}

impl Manifest {
    /// Serialize canonically: assets and representation names are sorted before JSON encoding.
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest cannot be serialized as JSON.
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut canonical = self.clone();
        canonical
            .assets
            .sort_by(|left, right| left.path.cmp(&right.path));
        serde_json::to_vec_pretty(&canonical)
    }

    /// Parse and validate schema-level invariants without accessing a bundle directory.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid JSON, an unsupported schema version, unsafe or
    /// non-content-addressed paths, duplicate paths or roles, invalid digests, or
    /// missing and unsupported representations.
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| Error::InvalidManifest(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Return the uniquely declared asset for a semantic role.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MissingRole`] when the role is absent.
    pub fn role(&self, role: Role) -> Result<&Asset, Error> {
        self.assets
            .iter()
            .find(|asset| asset.role == Some(role))
            .ok_or(Error::MissingRole(role))
    }

    /// Verify the manifest's paths, roles, digests, and exact `pkg/` inventory against `root`.
    ///
    /// # Errors
    ///
    /// Returns an error for any schema invariant failure, missing, extra, or
    /// digest-mismatched runtime file, or an I/O failure while walking the bundle.
    pub fn verify_bundle(&self, root: &Path) -> Result<(), Error> {
        self.validate()?;
        let mut declared = BTreeSet::new();
        for asset in &self.assets {
            declared.insert(asset.path.as_str());
            verify_file(root, &asset.path, &asset.sha256)?;
            for representation in asset.representations.values() {
                declared.insert(representation.path.as_str());
                verify_file(root, &representation.path, &representation.sha256)?;
            }
        }
        let pkg = root.join("pkg");
        if pkg.exists() {
            for path in files_below(&pkg)? {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| Error::UnsafePath(path.to_string_lossy().into_owned()))?
                    .to_string_lossy()
                    .into_owned();
                if !declared.contains(relative.as_str()) {
                    return Err(Error::UnexpectedFile(relative));
                }
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), Error> {
        if self.version != VERSION {
            return Err(Error::Version(self.version));
        }
        let mut paths = BTreeSet::new();
        let mut roles = BTreeSet::new();
        for asset in &self.assets {
            validate_path(&asset.path)?;
            validate_digest(&asset.sha256)?;
            validate_content_address(&asset.path, &asset.sha256)?;
            if let Some(role) = asset.role
                && !roles.insert(role)
            {
                return Err(Error::DuplicateRole(role));
            }
            let identity = asset.representations.get("identity").ok_or_else(|| {
                Error::MissingRepresentation {
                    path: asset.path.clone(),
                    encoding: "identity",
                }
            })?;
            if identity.path != asset.path {
                return Err(Error::MissingRepresentation {
                    path: asset.path.clone(),
                    encoding: "identity",
                });
            }
            if identity.sha256 != asset.sha256 {
                return Err(Error::DigestMismatch {
                    path: asset.path.clone(),
                    expected: asset.sha256.clone(),
                    actual: identity.sha256.clone(),
                });
            }
            for (encoding, representation) in &asset.representations {
                match encoding.as_str() {
                    "identity" => {}
                    "gzip" => {
                        if representation.path != format!("{}.gz", asset.path) {
                            return Err(Error::NonContentAddressedPath(
                                representation.path.clone(),
                            ));
                        }
                    }
                    "br" => {
                        if representation.path != format!("{}.br", asset.path) {
                            return Err(Error::NonContentAddressedPath(
                                representation.path.clone(),
                            ));
                        }
                    }
                    _ => return Err(Error::UnsupportedRepresentation(encoding.clone())),
                }
                validate_path(&representation.path)?;
                validate_digest(&representation.sha256)?;
                if !paths.insert(representation.path.as_str()) {
                    return Err(Error::DuplicatePath(representation.path.clone()));
                }
            }
        }
        for role in [Role::Glue, Role::Wasm] {
            let asset = self.role(role)?;
            for encoding in ["identity", "gzip", "br"] {
                if !asset.representations.contains_key(encoding) {
                    return Err(Error::MissingRepresentation {
                        path: asset.path.clone(),
                        encoding,
                    });
                }
            }
        }
        Ok(())
    }
}

/// SHA-256 lower-case hexadecimal digest of final served bytes.
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_digest(value: &str) -> Result<(), Error> {
    if value.len() == 64
        && value.bytes().all(|byte| {
            byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
        })
    {
        Ok(())
    } else {
        Err(Error::InvalidDigest(value.to_owned()))
    }
}

fn validate_path(value: &str) -> Result<(), Error> {
    let path = Path::new(value);
    if !value.starts_with("pkg/")
        || value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::UnsafePath(value.to_owned()));
    }
    Ok(())
}

fn validate_content_address(path: &str, digest: &str) -> Result<(), Error> {
    let filename = path
        .strip_prefix("pkg/")
        .ok_or_else(|| Error::UnsafePath(path.to_owned()))?;
    let Some(extension) = filename
        .strip_prefix(digest)
        .and_then(|tail| tail.strip_prefix('.'))
    else {
        return Err(Error::NonContentAddressedPath(path.to_owned()));
    };
    if !matches!(extension, "js" | "wasm") {
        return Err(Error::NonContentAddressedPath(path.to_owned()));
    }
    Ok(())
}

fn verify_file(root: &Path, relative: &str, expected: &str) -> Result<(), Error> {
    let path = root.join(relative);
    let bytes = fs::read(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::MissingFile(relative.to_owned())
        } else {
            Error::Io(error)
        }
    })?;
    let actual = digest(&bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(Error::DigestMismatch {
            path: relative.to_owned(),
            expected: expected.to_owned(),
            actual,
        })
    }
}

fn files_below(directory: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            files.extend(files_below(&path)?);
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_manifest(root: &Path) -> Manifest {
        fs::create_dir(root.join("pkg")).unwrap();
        let mut assets = Vec::new();
        for (role, name) in [(Role::Glue, "g.js"), (Role::Wasm, "w.wasm")] {
            let bytes = name.as_bytes();
            let identity_digest = digest(bytes);
            let extension = name.rsplit('.').next().expect("fixture extension");
            let path = format!("pkg/{identity_digest}.{extension}");
            fs::write(root.join(&path), bytes).unwrap();
            let mut representations = BTreeMap::from([(
                "identity".into(),
                Representation {
                    path: path.clone(),
                    sha256: identity_digest.clone(),
                },
            )]);
            for (encoding, suffix) in [("gzip", "gz"), ("br", "br")] {
                let representation_path = format!("{path}.{suffix}");
                fs::write(root.join(&representation_path), encoding).unwrap();
                representations.insert(
                    encoding.into(),
                    Representation {
                        path: representation_path,
                        sha256: digest(encoding.as_bytes()),
                    },
                );
            }
            assets.push(Asset {
                role: Some(role),
                path,
                sha256: identity_digest,
                representations,
            });
        }
        Manifest {
            version: VERSION,
            assets,
        }
    }

    #[test]
    fn verifies_exact_inventory_and_required_roles() {
        let root = tempfile::tempdir().unwrap();
        complete_manifest(root.path())
            .verify_bundle(root.path())
            .unwrap();
    }

    #[test]
    fn rejects_unsafe_duplicate_missing_and_extra_inventory() {
        let root = tempfile::tempdir().unwrap();
        let mut manifest = complete_manifest(root.path());
        manifest.assets[0].path = "../escape.js".into();
        assert!(matches!(
            manifest.verify_bundle(root.path()),
            Err(Error::UnsafePath(_))
        ));
        let duplicate_root = tempfile::tempdir().unwrap();
        let mut manifest = complete_manifest(duplicate_root.path());
        let mut duplicate = manifest.assets[0].clone();
        duplicate.role = Some(Role::Wasm);
        manifest.assets[1] = duplicate;
        assert!(matches!(manifest.validate(), Err(Error::DuplicatePath(_))));
        let root = tempfile::tempdir().unwrap();
        let manifest = complete_manifest(root.path());
        fs::write(root.path().join("pkg/extra.js"), b"x").unwrap();
        assert!(matches!(
            manifest.verify_bundle(root.path()),
            Err(Error::UnexpectedFile(_))
        ));
    }

    #[test]
    fn rejects_missing_file_digest_and_role_representations() {
        let root = tempfile::tempdir().unwrap();
        let manifest = complete_manifest(root.path());
        fs::remove_file(root.path().join(&manifest.assets[0].path)).unwrap();
        assert!(matches!(
            manifest.verify_bundle(root.path()),
            Err(Error::MissingFile(_))
        ));
        let root = tempfile::tempdir().unwrap();
        let mut manifest = complete_manifest(root.path());
        manifest.assets[0].sha256 = digest(b"wrong");
        assert!(matches!(
            manifest.verify_bundle(root.path()),
            Err(Error::NonContentAddressedPath(_))
        ));
        let root = tempfile::tempdir().unwrap();
        let mut manifest = complete_manifest(root.path());
        manifest.assets.pop();
        assert!(matches!(
            manifest.validate(),
            Err(Error::MissingRole(Role::Wasm))
        ));
        let root = tempfile::tempdir().unwrap();
        let mut manifest = complete_manifest(root.path());
        manifest.assets[0].representations.remove("br");
        assert!(matches!(
            manifest.validate(),
            Err(Error::MissingRepresentation { .. })
        ));
    }

    #[test]
    fn rejects_fixed_paths_unknown_encodings_and_wrong_sidecar_names() {
        let root = tempfile::tempdir().unwrap();
        let mut manifest = complete_manifest(root.path());
        manifest.assets[0].path = "pkg/fixed.js".into();
        manifest.assets[0]
            .representations
            .get_mut("identity")
            .unwrap()
            .path = "pkg/fixed.js".into();
        assert!(matches!(
            manifest.validate(),
            Err(Error::NonContentAddressedPath(_))
        ));
        let root = tempfile::tempdir().unwrap();
        let mut manifest = complete_manifest(root.path());
        let path = format!("{}.zst", manifest.assets[0].path);
        manifest.assets[0].representations.insert(
            "zstd".into(),
            Representation {
                path,
                sha256: digest(b"zstd"),
            },
        );
        assert!(matches!(
            manifest.validate(),
            Err(Error::UnsupportedRepresentation(_))
        ));
        let root = tempfile::tempdir().unwrap();
        let mut manifest = complete_manifest(root.path());
        manifest.assets[0]
            .representations
            .get_mut("gzip")
            .unwrap()
            .path = "pkg/not-a-sidecar.js.gz".into();
        assert!(matches!(
            manifest.validate(),
            Err(Error::NonContentAddressedPath(_))
        ));
    }

    #[test]
    fn serialization_is_deterministic() {
        let root = tempfile::tempdir().unwrap();
        let mut manifest = complete_manifest(root.path());
        let first = manifest.to_json().unwrap();
        manifest.assets.reverse();
        assert_eq!(first, manifest.to_json().unwrap());
    }
}
