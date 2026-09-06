use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const CORPUS_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/misc/backup_corpus");
const INDEX_FILE: &str = "index.json";

#[derive(Debug, thiserror::Error)]
pub(crate) enum BackupCorpusError {
    #[error("backup format compatibility corpus index is invalid: {0}")]
    InvalidIndex(String),
    #[error("backup format compatibility corpus fixture is unsafe: {0}")]
    UnsafeFixture(String),
    #[error(
        "backup format compatibility corpus digest mismatch for {fixture}: expected {expected}, got {actual}; add a new fixture, oracle, and index entry instead of editing a historical fixture"
    )]
    DigestMismatch {
        fixture: String,
        expected: String,
        actual: String,
    },
    #[error("backup format compatibility corpus I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("backup format compatibility corpus JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SupportState {
    Supported,
    Retired,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CorpusEntry {
    pub(crate) fixture: String,
    pub(crate) format_version: u32,
    pub(crate) support: SupportState,
    digest: String,
}

#[derive(Debug, Deserialize)]
struct CorpusIndex {
    fixtures: Vec<CorpusEntry>,
}

/// Checked-in backup-format evidence. The interface verifies a historical tree
/// before it is copied, then changes only the copied manifest's schema version.
pub(crate) struct BackupCorpus {
    root: PathBuf,
    entries: Vec<CorpusEntry>,
}

impl BackupCorpus {
    pub(crate) fn checked_in() -> Result<Self, BackupCorpusError> {
        Self::load(Path::new(CORPUS_ROOT))
    }

    pub(crate) fn load(root: impl Into<PathBuf>) -> Result<Self, BackupCorpusError> {
        let root = root.into();
        let index: CorpusIndex = serde_json::from_slice(&fs::read(root.join(INDEX_FILE))?)?;
        if index.fixtures.is_empty() {
            return Err(BackupCorpusError::InvalidIndex(
                "fixtures must not be empty".to_owned(),
            ));
        }
        let mut versions = BTreeSet::new();
        for entry in &index.fixtures {
            if !is_fixture_name_safe(&entry.fixture) {
                return Err(BackupCorpusError::InvalidIndex(
                    "fixture names must be non-empty relative paths without traversal".to_owned(),
                ));
            }
            if !versions.insert(entry.format_version) {
                return Err(BackupCorpusError::InvalidIndex(format!(
                    "format version {} appears more than once",
                    entry.format_version
                )));
            }
            decode_digest(&entry.digest)?;
        }
        Ok(Self {
            root,
            entries: index.fixtures,
        })
    }

    pub(crate) fn entries(&self) -> &[CorpusEntry] {
        &self.entries
    }

    pub(crate) fn verify(&self, entry: &CorpusEntry) -> Result<(), BackupCorpusError> {
        let actual = normalized_digest(&self.root.join(&entry.fixture))?;
        if actual != entry.digest {
            return Err(BackupCorpusError::DigestMismatch {
                fixture: entry.fixture.clone(),
                expected: entry.digest.clone(),
                actual,
            });
        }
        Ok(())
    }

    /// Verifies the immutable fixture before copying it to `destination` and
    /// substituting the target schema version in that temporary manifest.
    pub(crate) fn materialize(
        &self,
        entry: &CorpusEntry,
        destination: &Path,
        target_schema_version: i64,
    ) -> Result<(), BackupCorpusError> {
        self.verify(entry)?;
        if destination.exists() {
            return Err(BackupCorpusError::InvalidIndex(format!(
                "materialization destination already exists: {}",
                destination.display()
            )));
        }
        copy_regular_tree(&self.root.join(&entry.fixture), destination)?;
        let manifest_path = destination.join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
        let Some(object) = manifest.as_object_mut() else {
            return Err(BackupCorpusError::InvalidIndex(
                "fixture manifest must be a JSON object".to_owned(),
            ));
        };
        let Some(schema_version) = object.get_mut("schema_version") else {
            return Err(BackupCorpusError::InvalidIndex(
                "fixture manifest must contain schema_version".to_owned(),
            ));
        };
        *schema_version = Value::from(target_schema_version);
        fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
        Ok(())
    }
}

fn is_fixture_name_safe(name: &str) -> bool {
    !name.is_empty()
        && Path::new(name)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn decode_digest(digest: &str) -> Result<(), BackupCorpusError> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackupCorpusError::InvalidIndex(
            "digest must be a 64-character SHA-256 hexadecimal value".to_owned(),
        ));
    }
    Ok(())
}

fn normalized_digest(root: &Path) -> Result<String, BackupCorpusError> {
    let mut files = Vec::new();
    collect_regular_files(root, Path::new(""), &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (path, file) in files {
        let path = path.as_bytes();
        hasher.update((path.len() as u64).to_be_bytes());
        hasher.update(path);
        let mut content = Vec::new();
        fs::File::open(file)?.read_to_end(&mut content)?;
        hasher.update((content.len() as u64).to_be_bytes());
        hasher.update(content);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_regular_files(
    root: &Path,
    relative: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), BackupCorpusError> {
    let directory = root.join(relative);
    let metadata = fs::symlink_metadata(&directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BackupCorpusError::UnsafeFixture(format!(
            "{} is not a directory",
            directory.display()
        )));
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let child = relative.join(&name);
        let path = root.join(&child);
        let file_type = fs::symlink_metadata(&path)?.file_type();
        if file_type.is_symlink() {
            return Err(BackupCorpusError::UnsafeFixture(format!(
                "symlink entry: {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            collect_regular_files(root, &child, files)?;
        } else if file_type.is_file() {
            let name = relative_path(&child)?;
            files.push((name, path));
        } else {
            return Err(BackupCorpusError::UnsafeFixture(format!(
                "special entry: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn relative_path(path: &Path) -> Result<String, BackupCorpusError> {
    let parts = path
        .components()
        .map(|component| {
            component.as_os_str().to_str().ok_or_else(|| {
                BackupCorpusError::UnsafeFixture(format!(
                    "non-UTF-8 fixture path: {}",
                    path.display()
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

fn copy_regular_tree(source: &Path, destination: &Path) -> Result<(), BackupCorpusError> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BackupCorpusError::UnsafeFixture(format!(
            "{} is not a directory",
            source.display()
        )));
    }
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let source_child = entry.path();
        let destination_child = destination.join(name);
        let file_type = fs::symlink_metadata(&source_child)?.file_type();
        if file_type.is_symlink() {
            return Err(BackupCorpusError::UnsafeFixture(format!(
                "symlink entry: {}",
                source_child.display()
            )));
        }
        if file_type.is_dir() {
            copy_regular_tree(&source_child, &destination_child)?;
        } else if file_type.is_file() {
            fs::copy(source_child, destination_child)?;
        } else {
            return Err(BackupCorpusError::UnsafeFixture(format!(
                "special entry: {}",
                source_child.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use tempfile::TempDir;

    fn fixture_root() -> TempDir {
        let temp = TempDir::new().expect("create fixture root");
        fs::create_dir_all(temp.path().join("fixture").join("nested"))
            .expect("create fixture directories");
        fs::write(
            temp.path().join("fixture").join("nested").join("value"),
            b"value",
        )
        .expect("write fixture value");
        fs::write(
            temp.path().join(INDEX_FILE),
            r#"{"fixtures":[{"fixture":"fixture","format_version":1,"support":"supported","digest":"0000000000000000000000000000000000000000000000000000000000000000"}]}"#,
        )
        .expect("write index");
        temp
    }

    #[test]
    fn checked_in_legacy_v1_fixture_matches_its_known_digest() {
        let corpus = BackupCorpus::checked_in().expect("load checked-in corpus");
        let [entry] = corpus.entries() else {
            panic!("one initial format fixture");
        };
        assert_eq!(entry.format_version, 1);
        assert_eq!(entry.support, SupportState::Supported);
        corpus.verify(entry).expect("known fixture digest matches");
    }

    #[test]
    fn index_rejects_fixture_paths_that_escape_the_corpus_root() {
        let temp = fixture_root();
        fs::write(
            temp.path().join(INDEX_FILE),
            r#"{"fixtures":[{"fixture":"../outside","format_version":1,"support":"supported","digest":"0000000000000000000000000000000000000000000000000000000000000000"}]}"#,
        )
        .expect("write escaping index");
        assert!(matches!(
            BackupCorpus::load(temp.path()),
            Err(BackupCorpusError::InvalidIndex(message)) if message.contains("relative")
        ));
    }

    #[test]
    fn digest_changes_when_a_fixture_path_or_content_changes() {
        let temp = fixture_root();
        let original = normalized_digest(&temp.path().join("fixture")).expect("hash fixture");
        fs::write(
            temp.path().join("fixture").join("nested").join("value"),
            b"changed",
        )
        .expect("change fixture content");
        assert_ne!(
            normalized_digest(&temp.path().join("fixture")).expect("hash changed fixture"),
            original
        );
        fs::rename(
            temp.path().join("fixture").join("nested").join("value"),
            temp.path().join("fixture").join("nested").join("renamed"),
        )
        .expect("change fixture path");
        assert_ne!(
            normalized_digest(&temp.path().join("fixture")).expect("hash renamed fixture"),
            original
        );
    }

    #[test]
    fn digest_length_frames_paths_and_content_unambiguously() {
        let temp = TempDir::new().expect("create fixture root");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).expect("create first fixture");
        fs::create_dir(&second).expect("create second fixture");
        fs::write(first.join("a"), b"bc").expect("write first ambiguity candidate");
        fs::write(second.join("ab"), b"c").expect("write second ambiguity candidate");
        assert_ne!(
            normalized_digest(&first).expect("hash first fixture"),
            normalized_digest(&second).expect("hash second fixture")
        );
    }

    #[test]
    fn digest_traverses_directories_but_rejects_digest_mismatch_before_copying() {
        let temp = fixture_root();
        let corpus = BackupCorpus::load(temp.path()).expect("load corpus");
        let entry = &corpus.entries()[0];
        let target_schema_version = 7;
        assert!(matches!(
            corpus.materialize(
                entry,
                &temp.path().join("materialized"),
                target_schema_version
            ),
            Err(BackupCorpusError::DigestMismatch { .. })
        ));
        assert!(!temp.path().join("materialized").exists());
    }

    #[cfg(unix)]
    #[test]
    fn digest_rejects_symlink_and_socket_entries() {
        use std::os::unix::{fs::symlink, net::UnixListener};

        let temp = TempDir::new().expect("create fixture root");
        let root = temp.path().join("fixture");
        fs::create_dir(&root).expect("create fixture");
        fs::write(root.join("target"), b"target").expect("write target");
        symlink(root.join("target"), root.join("link")).expect("create symlink");
        assert!(matches!(
            normalized_digest(&root),
            Err(BackupCorpusError::UnsafeFixture(message)) if message.contains("symlink")
        ));
        fs::remove_file(root.join("link")).expect("remove symlink");
        let _socket = UnixListener::bind(root.join("socket")).expect("create socket");
        assert!(matches!(
            normalized_digest(&root),
            Err(BackupCorpusError::UnsafeFixture(message)) if message.contains("special")
        ));
    }

    #[test]
    fn legacy_manifest_preserves_sentinels_and_omits_format_version() {
        let corpus = BackupCorpus::checked_in().expect("load checked-in corpus");
        let entry = &corpus.entries()[0];
        let manifest: Value = serde_json::from_slice(
            &fs::read(corpus.root.join(&entry.fixture).join("manifest.json"))
                .expect("read fixture manifest"),
        )
        .expect("parse fixture manifest");
        assert!(manifest.get("format_version").is_none());
        assert_ne!(manifest["version"], env!("CARGO_PKG_VERSION"));
        assert_ne!(manifest["schema_checksum"], "");
        assert!(
            manifest["tables"]
                .as_array()
                .expect("tables array")
                .contains(&Value::from("instance_identity"))
        );
        assert!(
            corpus
                .root
                .join(&entry.fixture)
                .join("media")
                .join("avatar.txt")
                .is_file()
        );
    }

    #[test]
    fn legacy_fixture_pins_every_wire_role_and_its_relationships() {
        let corpus = BackupCorpus::checked_in().expect("load checked-in corpus");
        let entry = &corpus.entries()[0];
        let root = corpus.root.join(&entry.fixture);
        let manifest: Value =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).expect("read manifest"))
                .expect("parse manifest");
        assert_eq!(
            manifest["tables"],
            serde_json::json!([
                "instance_identity",
                "media",
                "post_revisions",
                "posts",
                "users"
            ])
        );
        let user = ndjson_row(&root, "users");
        assert_eq!(user["user_id"], 41);
        assert_eq!(user["display_name"], Value::Null);
        assert_eq!(user["email_verified"], false);
        assert_eq!(user["is_operator"], true);

        let media = ndjson_row(&root, "media");
        assert_eq!(media["user_id"], user["user_id"]);
        assert_eq!(media["size_bytes"], 13);
        assert_eq!(media["source_url"], Value::Null);

        let post = ndjson_row(&root, "posts");
        assert_eq!(post["user_id"], user["user_id"]);
        assert_eq!(
            post["rendered_html"],
            serde_json::json!({"kind": "structured", "items": [1, true]})
        );
        let revision = ndjson_row(&root, "post_revisions");
        assert_eq!(revision["post_id"], post["post_id"]);
        assert_eq!(revision["user_id"], user["user_id"]);
        assert_eq!(revision["rendered_html"], 1.25);
    }

    #[test]
    fn materialization_leaves_source_immutable_and_changes_only_schema_version() {
        let corpus = BackupCorpus::checked_in().expect("load checked-in corpus");
        let entry = &corpus.entries()[0];
        let source_before = tree_bytes(&corpus.root.join(&entry.fixture));
        let temp = TempDir::new().expect("create materialization root");
        let destination = temp.path().join("fixture");
        let target_schema_version = 7;
        corpus
            .materialize(entry, &destination, target_schema_version)
            .expect("materialize fixture");
        assert_eq!(tree_bytes(&corpus.root.join(&entry.fixture)), source_before);

        let source_manifest: Value =
            serde_json::from_slice(&source_before["manifest.json"]).expect("parse source manifest");
        let materialized_manifest: Value = serde_json::from_slice(
            &fs::read(destination.join("manifest.json")).expect("read materialized manifest"),
        )
        .expect("parse materialized manifest");
        let mut expected = source_manifest;
        expected["schema_version"] = Value::from(target_schema_version);
        assert_eq!(materialized_manifest, expected);
        let mut materialized = tree_bytes(&destination);
        materialized.remove("manifest.json");
        let mut source = source_before;
        source.remove("manifest.json");
        assert_eq!(materialized, source);
    }

    fn ndjson_row(root: &Path, table: &str) -> Value {
        serde_json::from_slice(
            &fs::read(root.join("db").join(format!("{table}.ndjson")))
                .expect("read fixture table row"),
        )
        .expect("parse fixture table row")
    }

    fn tree_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut files = Vec::new();
        collect_regular_files(root, Path::new(""), &mut files).expect("safe fixture tree");
        files
            .into_iter()
            .map(|(path, file)| (path, fs::read(file).expect("read fixture file")))
            .collect()
    }
}
