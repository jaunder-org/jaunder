use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    ops::Range,
    path::{Component, Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
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
    /// substituting only the target schema-version value bytes in that temporary
    /// manifest.
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
        let manifest = fs::read(&manifest_path)?;
        let schema_version = schema_version_value_span(&manifest)?;
        let mut materialized =
            Vec::with_capacity(manifest.len() - (schema_version.end - schema_version.start) + 20);
        materialized.extend_from_slice(&manifest[..schema_version.start]);
        materialized.extend_from_slice(target_schema_version.to_string().as_bytes());
        materialized.extend_from_slice(&manifest[schema_version.end..]);
        fs::write(manifest_path, materialized)?;
        Ok(())
    }

    /// Packages a verified, materialized corpus directory with test-owned tar
    /// and gzip code so archive-reader coverage cannot share production helpers.
    pub(crate) fn package_archive(
        source: &Path,
        destination: &Path,
    ) -> Result<(), BackupCorpusError> {
        let output = fs::File::create(destination)?;
        let encoder = GzEncoder::new(output, Compression::default());
        let mut archive = tar::Builder::new(encoder);
        archive.append_dir_all("", source)?;
        archive.into_inner()?.finish()?;
        Ok(())
    }

    /// Independently extracts a production archive for raw-wire inspection.
    /// This intentionally does not reuse production archive helpers.
    pub(crate) fn extract_archive(
        source: &Path,
        destination: &Path,
    ) -> Result<(), BackupCorpusError> {
        let input = fs::File::open(source)?;
        let decoder = flate2::read::GzDecoder::new(input);
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;
            let Some(relative) = normalized_archive_relative_path(&path)? else {
                continue;
            };
            let destination_path = destination.join(&relative);
            if entry.header().entry_type().is_dir() {
                fs::create_dir_all(destination_path)?;
            } else if entry.header().entry_type().is_file() {
                if let Some(parent) = destination_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                fs::write(destination_path, bytes)?;
            } else {
                return Err(BackupCorpusError::UnsafeFixture(format!(
                    "archive contains a non-file entry: {}",
                    relative.display()
                )));
            }
        }
        Ok(())
    }
}

fn normalized_archive_relative_path(path: &Path) -> Result<Option<PathBuf>, BackupCorpusError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(BackupCorpusError::UnsafeFixture(format!(
                    "archive entry path must contain only normalized relative components: {}",
                    path.display()
                )));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

/// Locates the source bytes occupied by the top-level `schema_version` value
/// after validating that the manifest is a JSON object with an integer version.
fn schema_version_value_span(manifest: &[u8]) -> Result<Range<usize>, BackupCorpusError> {
    let value: Value = serde_json::from_slice(manifest)?;
    let Some(object) = value.as_object() else {
        return Err(BackupCorpusError::InvalidIndex(
            "fixture manifest must be a JSON object".to_owned(),
        ));
    };
    if object
        .get("schema_version")
        .and_then(Value::as_i64)
        .is_none()
    {
        return Err(BackupCorpusError::InvalidIndex(
            "fixture manifest must contain an integer schema_version".to_owned(),
        ));
    }

    let mut cursor = skip_json_whitespace(manifest, 0);
    if manifest.get(cursor) != Some(&b'{') {
        return Err(BackupCorpusError::InvalidIndex(
            "fixture manifest must be a JSON object".to_owned(),
        ));
    }
    cursor += 1;
    loop {
        cursor = skip_json_whitespace(manifest, cursor);
        if manifest.get(cursor) == Some(&b'}') {
            break;
        }
        let key_start = cursor;
        cursor = json_string_end(manifest, cursor)?;
        let key: String = serde_json::from_slice(&manifest[key_start..cursor])?;
        cursor = skip_json_whitespace(manifest, cursor);
        if manifest.get(cursor) != Some(&b':') {
            return Err(BackupCorpusError::InvalidIndex(
                "fixture manifest has an invalid member separator".to_owned(),
            ));
        }
        cursor = skip_json_whitespace(manifest, cursor + 1);
        let value_start = cursor;
        cursor = json_value_end(manifest, cursor)?;
        if key == "schema_version" {
            return Ok(value_start..cursor);
        }
        cursor = skip_json_whitespace(manifest, cursor);
        match manifest.get(cursor) {
            Some(b',') => cursor += 1,
            Some(b'}') => break,
            _ => {
                return Err(BackupCorpusError::InvalidIndex(
                    "fixture manifest has an invalid member separator".to_owned(),
                ));
            }
        }
    }
    Err(BackupCorpusError::InvalidIndex(
        "fixture manifest must contain schema_version".to_owned(),
    ))
}

fn skip_json_whitespace(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
    {
        cursor += 1;
    }
    cursor
}

fn json_string_end(bytes: &[u8], start: usize) -> Result<usize, BackupCorpusError> {
    if bytes.get(start) != Some(&b'"') {
        return Err(BackupCorpusError::InvalidIndex(
            "fixture manifest has an invalid object key".to_owned(),
        ));
    }
    let mut cursor = start + 1;
    while let Some(byte) = bytes.get(cursor) {
        match byte {
            b'\\' => cursor += 2,
            b'"' => return Ok(cursor + 1),
            _ => cursor += 1,
        }
    }
    Err(BackupCorpusError::InvalidIndex(
        "fixture manifest has an unterminated string".to_owned(),
    ))
}

fn json_value_end(bytes: &[u8], start: usize) -> Result<usize, BackupCorpusError> {
    match bytes.get(start) {
        Some(b'"') => json_string_end(bytes, start),
        Some(b'{' | b'[') => {
            let mut cursor = start;
            let mut depth = 0;
            while let Some(byte) = bytes.get(cursor) {
                match byte {
                    b'"' => cursor = json_string_end(bytes, cursor)?,
                    b'{' | b'[' => {
                        depth += 1;
                        cursor += 1;
                    }
                    b'}' | b']' => {
                        depth -= 1;
                        cursor += 1;
                        if depth == 0 {
                            return Ok(cursor);
                        }
                    }
                    _ => cursor += 1,
                }
            }
            Err(BackupCorpusError::InvalidIndex(
                "fixture manifest has an unterminated compound value".to_owned(),
            ))
        }
        Some(_) => {
            let mut cursor = start;
            while bytes.get(cursor).is_some_and(|byte| {
                !matches!(byte, b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t')
            }) {
                cursor += 1;
            }
            if cursor == start {
                return Err(BackupCorpusError::InvalidIndex(
                    "fixture manifest has an invalid value".to_owned(),
                ));
            }
            Ok(cursor)
        }
        None => Err(BackupCorpusError::InvalidIndex(
            "fixture manifest has a missing value".to_owned(),
        )),
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
        let corpus = compatibility_result(BackupCorpus::checked_in(), "load checked-in corpus");
        let entry = compatibility_option(corpus.entries().first(), "one initial format fixture");
        assert_eq!(entry.format_version, 1);
        assert_eq!(entry.support, SupportState::Supported);
        compatibility_result(corpus.verify(entry), "known fixture digest matches");
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
        let corpus = compatibility_result(BackupCorpus::checked_in(), "load checked-in corpus");
        let entry = compatibility_option(
            corpus.entries().first(),
            "fixture index must contain an entry",
        );
        let bytes = compatibility_result(
            fs::read(corpus.root.join(&entry.fixture).join("manifest.json")),
            "read fixture manifest",
        );
        let manifest: Value =
            compatibility_result(serde_json::from_slice(&bytes), "parse fixture manifest");
        assert!(manifest.get("format_version").is_none());
        assert_ne!(manifest["version"], env!("CARGO_PKG_VERSION"));
        assert_ne!(manifest["schema_checksum"], "");
        assert!(
            compatibility_option(
                manifest.get("tables").and_then(Value::as_array),
                "fixture manifest tables must be an array",
            )
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
        let corpus = compatibility_result(BackupCorpus::checked_in(), "load checked-in corpus");
        let entry = compatibility_option(
            corpus.entries().first(),
            "fixture index must contain an entry",
        );
        let root = corpus.root.join(&entry.fixture);
        let bytes = compatibility_result(
            fs::read(root.join("manifest.json")),
            "read fixture manifest",
        );
        let manifest: Value =
            compatibility_result(serde_json::from_slice(&bytes), "parse fixture manifest");
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

        let source_manifest = &source_before["manifest.json"];
        let materialized_manifest =
            fs::read(destination.join("manifest.json")).expect("read materialized manifest");
        assert_eq!(
            normalize_schema_version_value(source_manifest),
            normalize_schema_version_value(&materialized_manifest),
            "materialization must preserve every manifest byte other than schema_version"
        );
        let parsed: Value =
            serde_json::from_slice(&materialized_manifest).expect("parse materialized manifest");
        assert_eq!(parsed["schema_version"], target_schema_version);

        let mut materialized = tree_bytes(&destination);
        materialized.remove("manifest.json");
        let mut source = source_before;
        source.remove("manifest.json");
        assert_eq!(materialized, source);
    }

    fn normalize_schema_version_value(manifest: &[u8]) -> Vec<u8> {
        let value = schema_version_value_span(manifest).expect("locate schema version");
        let mut normalized = Vec::with_capacity(manifest.len());
        normalized.extend_from_slice(&manifest[..value.start]);
        normalized.extend_from_slice(b"<schema-version>");
        normalized.extend_from_slice(&manifest[value.end..]);
        normalized
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
    #[test]
    fn extraction_rejects_traversal_before_writing_outside_destination() {
        let temp = TempDir::new().expect("create archive test root");
        let archive_path = temp.path().join("traversal.tar.gz");
        let output = fs::File::create(&archive_path).expect("create traversal archive");
        let encoder = GzEncoder::new(output, Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(6);
        header.set_mode(0o644);
        let name = b"../outside\0";
        header.as_mut_bytes()[..name.len()].copy_from_slice(name);
        header.set_cksum();
        archive
            .append(&header, &b"escape"[..])
            .expect("add traversal archive member");
        archive
            .into_inner()
            .expect("finish traversal tar")
            .finish()
            .expect("finish traversal gzip");

        let destination = temp.path().join("destination");
        let outside = temp.path().join("outside");
        assert!(matches!(
            BackupCorpus::extract_archive(&archive_path, &destination),
            Err(BackupCorpusError::UnsafeFixture(message)) if message.contains("normalized relative")
        ));
        assert!(
            !outside.exists(),
            "traversal archive must not write outside its destination"
        );
    }
}

#[cfg(test)]
struct InitializedCommandEnv {
    args: jaunder::cli::StorageArgs,
    base: tempfile::TempDir,
    _postgres: Option<storage::test_support::PostgresDbGuard>,
}

#[cfg(test)]
impl InitializedCommandEnv {
    async fn new(backend: storage::test_support::Backend) -> Self {
        use storage::test_support::{PostgresTestConfig, sqlite_url, unique_postgres_url};

        let base = tempfile::TempDir::new().expect("create command environment");
        let (db, postgres) = match backend {
            storage::test_support::Backend::Sqlite => (sqlite_url(&base), None),
            storage::test_support::Backend::Postgres => {
                let config = PostgresTestConfig::from_env();
                let (db, guard) = unique_postgres_url(&config).await;
                (db, Some(guard))
            }
        };
        let args = jaunder::cli::StorageArgs {
            storage_path: base.path().join("storage"),
            db,
        };
        jaunder::commands::cmd_init(&args, false)
            .await
            .expect("initialize target");
        Self {
            args,
            base,
            _postgres: postgres,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum CorpusIoMode {
    Directory,
    Archive,
}

#[cfg(test)]
impl CorpusIoMode {
    const ALL: [Self; 2] = [Self::Directory, Self::Archive];

    fn name(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Archive => "archive",
        }
    }

    fn backup_mode(self) -> storage::BackupMode {
        match self {
            Self::Directory => storage::BackupMode::Directory,
            Self::Archive => storage::BackupMode::Archive,
        }
    }
}

#[cfg(test)]
const FORMAT_BUMP_DIAGNOSTIC: &str = "add a new immutable fixture, oracle, and corpus-index entry instead of changing format-1 history";

#[cfg(test)]
fn format_compatibility_diagnostic(detail: impl std::fmt::Display) -> String {
    format!("backup format compatibility: {detail}; {FORMAT_BUMP_DIAGNOSTIC}")
}
#[cfg(test)]
fn compatibility_result<T, E: std::fmt::Display>(
    result: Result<T, E>,
    detail: impl std::fmt::Display,
) -> T {
    result.unwrap_or_else(|error| {
        panic!(
            "{}",
            format_compatibility_diagnostic(format_args!("{detail}: {error}"))
        )
    })
}

#[cfg(test)]
fn compatibility_option<T>(value: Option<T>, detail: impl std::fmt::Display) -> T {
    value.unwrap_or_else(|| panic!("{}", format_compatibility_diagnostic(detail)))
}

#[cfg(test)]
mod reader_tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::Path,
    };

    use super::{
        BackupCorpus, CorpusEntry, CorpusIoMode, InitializedCommandEnv, SupportState,
        compatibility_option, compatibility_result,
    };
    use common::{
        ids::{PostId, UserId},
        media::MediaSource,
        pagination::PageSize,
        test_support::{parse_content_hash, parse_filename},
        username::Username,
        visibility::ViewerIdentity,
    };
    use jaunder::{
        cli::StorageArgs,
        commands::{cmd_backup, cmd_restore},
    };
    use rstest::*;
    use rstest_reuse::apply;
    use serde_json::Value;
    use storage::{
        BackupError, BackupMode, StorageRuntimeConfig, open_existing_database,
        test_support::{Backend, backends},
    };
    macro_rules! assert_eq {
        ($left:expr, $right:expr $(,)?) => {
            ::std::assert_eq!(
                $left,
                $right,
                "{}",
                super::format_compatibility_diagnostic("reader contract value differs")
            )
        };
        ($left:expr, $right:expr, $($message:tt)+) => {
            ::std::assert_eq!(
                $left,
                $right,
                "{}",

                super::format_compatibility_diagnostic(format_args!($($message)+))
            )
        };
    }

    macro_rules! assert {
        ($condition:expr $(,)?) => {
            ::std::assert!(
                $condition,
                "{}",
                super::format_compatibility_diagnostic("reader contract assertion failed")
            )
        };
        ($condition:expr, $($message:tt)+) => {
            ::std::assert!(
                $condition,
                "{}",
                super::format_compatibility_diagnostic(format_args!($($message)+))
            )
        };
    }
    macro_rules! assert_restored_reader_roles {
        ($args:expr, $user_id:expr, $post_id:expr, $user:expr, $post:expr, $revision:expr, $media:expr) => {
            for role in READER_ROLE_INVENTORY {
                match role.name {
                    "null" => assert_eq!($user.display_name, None, "{}", role.restored_expectation),
                    "boolean" => assert_eq!($user.is_operator, storage::OperatorStatus::OPERATOR, "{}", role.restored_expectation),
                    "integer" => assert_eq!($media.size_bytes.to_string(), "13", "{}", role.restored_expectation),
                    "real" => assert_eq!(
                        compatibility_result(serde_json::from_str::<Value>($revision.rendered_html.as_ref()), "real JSON must survive semantically"),
                        serde_json::json!(1.25), "{}", role.restored_expectation
                    ),
                    "text" => assert_eq!($post.title.as_deref(), Some("Legacy wire roles"), "{}", role.restored_expectation),
                    "structured JSON" => assert_eq!(
                        compatibility_result(serde_json::from_str::<Value>($post.rendered_html.as_ref()), "structured JSON must survive semantically"),
                        serde_json::json!({"kind": "structured", "items": [1, true]}), "{}", role.restored_expectation
                    ),
                    "user→post→revision" => {
                        assert_eq!($post.user_id, $user_id, "{}", role.restored_expectation);
                        assert_eq!($revision.post_id, $post_id, "{}", role.restored_expectation);
                        assert_eq!($revision.user_id, $user_id, "{}", role.restored_expectation);
                    }
                    "user→media" => {
                        assert_eq!($media.user_id, $user_id, "{}", role.restored_expectation);
                    }
                    "exact media bytes" => assert_eq!(
                        compatibility_result(fs::read($args.storage_path.join("media").join("avatar.txt")), "read restored media bytes"),
                        b"legacy media\n", "{}", role.restored_expectation
                    ),
                    role => panic!("{}", super::format_compatibility_diagnostic(format_args!("unknown reader-role inventory entry: {role}"))),
                }
            }
        };
    }

    #[derive(Debug, PartialEq, Eq)]
    enum RestoreExpectation {
        Succeeds,
        TypedUnsupportedFormat,
    }

    // The checked-in index currently has no retired production format, so the
    // classification test below covers that index contract while commands.rs
    // retains executable typed-error coverage for the production reader.

    fn expected_restore(entry: &CorpusEntry) -> RestoreExpectation {
        match entry.support {
            SupportState::Supported => RestoreExpectation::Succeeds,
            SupportState::Retired => RestoreExpectation::TypedUnsupportedFormat,
        }
    }
    struct ReaderRole {
        name: &'static str,
        fixture_path: &'static str,
        table: &'static str,
        column: &'static str,
        wire_value: &'static str,
        restored_expectation: &'static str,
    }

    const READER_ROLE_INVENTORY: &[ReaderRole] = &[
        ReaderRole {
            name: "null",
            fixture_path: "db/users.ndjson",
            table: "users",
            column: "display_name",
            wire_value: "null",
            restored_expectation: "legacyuser.display_name is None",
        },
        ReaderRole {
            name: "boolean",
            fixture_path: "db/users.ndjson",
            table: "users",
            column: "is_operator",
            wire_value: "true",
            restored_expectation: "legacyuser.is_operator is OPERATOR",
        },
        ReaderRole {
            name: "integer",
            fixture_path: "db/media.ndjson",
            table: "media",
            column: "size_bytes",
            wire_value: "13",
            restored_expectation: "avatar.txt.size_bytes is 13",
        },
        ReaderRole {
            name: "real",
            fixture_path: "db/post_revisions.ndjson",
            table: "post_revisions",
            column: "rendered_html",
            wire_value: "1.25",
            restored_expectation: "revision.rendered_html semantically equals 1.25",
        },
        ReaderRole {
            name: "text",
            fixture_path: "db/posts.ndjson",
            table: "posts",
            column: "title",
            wire_value: "\"Legacy wire roles\"",
            restored_expectation: "post.title is Legacy wire roles",
        },
        ReaderRole {
            name: "structured JSON",
            fixture_path: "db/posts.ndjson",
            table: "posts",
            column: "rendered_html",
            wire_value: "{\"kind\":\"structured\",\"items\":[1,true]}",
            restored_expectation: "post.rendered_html semantically preserves the object",
        },
        ReaderRole {
            name: "user→post→revision",
            fixture_path: "db/post_revisions.ndjson",
            table: "post_revisions",
            column: "post_id",
            wire_value: "71",
            restored_expectation: "revision belongs to post 71 owned by user 41",
        },
        ReaderRole {
            name: "user→media",
            fixture_path: "db/media.ndjson",
            table: "media",
            column: "user_id",
            wire_value: "41",
            restored_expectation: "avatar.txt belongs to user 41",
        },
        ReaderRole {
            name: "exact media bytes",
            fixture_path: "media/avatar.txt",
            table: "media",
            column: "bytes",
            wire_value: "legacy media\n",
            restored_expectation: "restored media/avatar.txt bytes exactly match",
        },
    ];

    fn assert_reader_inventory_is_complete() {
        assert_eq!(
            READER_ROLE_INVENTORY
                .iter()
                .map(|role| role.name)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "boolean",
                "exact media bytes",
                "integer",
                "null",
                "real",
                "structured JSON",
                "text",
                "user→media",
                "user→post→revision",
            ]),
            "reader inventory must name every required historical fixture role"
        );
        assert!(READER_ROLE_INVENTORY.iter().all(|role| {
            !role.fixture_path.is_empty()
                && !role.table.is_empty()
                && !role.column.is_empty()
                && !role.wire_value.is_empty()
                && !role.restored_expectation.is_empty()
        }));
    }

    fn assert_fixture_wire_inventory(fixture: &Path) {
        assert_reader_inventory_is_complete();
        for role in READER_ROLE_INVENTORY {
            let observed = fs::read(fixture.join(role.fixture_path)).unwrap_or_else(|error| {
                panic!(
                    "{}",
                    super::format_compatibility_diagnostic(format_args!(
                        "{} fixture path {} cannot be read: {error}",
                        role.name, role.fixture_path
                    ))
                )
            });
            if role.name == "exact media bytes" {
                assert_eq!(
                    observed,
                    role.wire_value.as_bytes(),
                    "{} fixture media bytes differ from the inventory",
                    role.name
                );
                continue;
            }
            let row = serde_json::from_slice::<Value>(&observed).unwrap_or_else(|error| {
                panic!(
                    "{}",
                    super::format_compatibility_diagnostic(format_args!(
                        "{} fixture row {} is not JSON: {error}",
                        role.name, role.fixture_path
                    ))
                )
            });
            let object = row.as_object().unwrap_or_else(|| {
                panic!(
                    "{}",
                    super::format_compatibility_diagnostic(format_args!(
                        "{} fixture row {} must be a JSON object",
                        role.name, role.fixture_path
                    ))
                )
            });
            let expected = serde_json::from_str::<Value>(role.wire_value).unwrap_or_else(|error| {
                panic!(
                    "{}",
                    super::format_compatibility_diagnostic(format_args!(
                        "{} inventory wire value is invalid JSON: {error}",
                        role.name
                    ))
                )
            });
            assert_eq!(
                object.get(role.column),
                Some(&expected),
                "{} inventory entry for {}.{} wire value differs",
                role.name,
                role.table,
                role.column
            );
        }
    }
    async fn assert_reader_inventory(args: &StorageArgs) {
        let factory = compatibility_result(
            open_existing_database(&args.db, &StorageRuntimeConfig::default()).await,
            "open restored database",
        );
        let user_id = UserId::from(41);
        let post_id = PostId::from(71);
        let username: Username =
            compatibility_result("legacyuser".parse(), "parse fixture username");
        let user = compatibility_option(
            compatibility_result(
                factory.users().get_user_by_username(&username).await,
                "read restored user",
            ),
            "fixture user exists",
        );
        let post = compatibility_option(
            compatibility_result(
                factory
                    .posts()
                    .get_post_by_id(post_id, &ViewerIdentity::local(user_id))
                    .await,
                "read restored post",
            ),
            "fixture post exists",
        );
        let history = compatibility_option(
            compatibility_result(
                factory
                    .posts()
                    .list_post_revision_history(user_id, post_id, None, PageSize::default())
                    .await,
                "read restored revision history",
            ),
            "fixture post history exists",
        );
        let first_revision = compatibility_option(
            history.revisions.first(),
            "fixture post history must contain a revision",
        );
        let revision = compatibility_option(
            compatibility_result(
                factory
                    .posts()
                    .get_post_revision_detail(user_id, post_id, first_revision.revision_id)
                    .await,
                "read restored revision",
            ),
            "fixture revision exists",
        )
        .revision;
        let media = compatibility_option(
            compatibility_result(
                factory
                    .media()
                    .get_media(
                        user_id,
                        &parse_content_hash(
                            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                        ),
                        &parse_filename("avatar.txt"),
                        &MediaSource::Upload,
                    )
                    .await,
                "read restored media",
            ),
            "fixture media exists",
        );

        assert_restored_reader_roles!(args, user_id, post_id, user, post, revision, media);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn historical_corpus_entries_restore_through_each_public_input(#[case] backend: Backend) {
        let corpus = compatibility_result(BackupCorpus::checked_in(), "load corpus");
        for entry in corpus.entries() {
            for input in CorpusIoMode::ALL {
                let target = InitializedCommandEnv::new(backend).await;
                let schema_version = current_schema_version(&target).await;
                let materialized = target.base.path().join("materialized");
                compatibility_result(
                    corpus.materialize(entry, &materialized, schema_version),
                    "materialize verified historical fixture",
                );
                assert_fixture_wire_inventory(&materialized);
                let restore_path = match input {
                    CorpusIoMode::Directory => materialized,
                    CorpusIoMode::Archive => {
                        let archive = target.base.path().join("fixture.tar.gz");
                        compatibility_result(
                            BackupCorpus::package_archive(&materialized, &archive),
                            "independently package fixture archive",
                        );
                        archive
                    }
                };

                match expected_restore(entry) {
                    RestoreExpectation::Succeeds => {
                        cmd_restore(&target.args, &restore_path)
                            .await
                            .unwrap_or_else(|error| {
                                panic!(
                                    "{}",
                                    super::format_compatibility_diagnostic(format_args!(
                                        "supported historical restore version {} from {} failed: {error:#}",
                                        entry.format_version,
                                        input.name()
                                    ))
                                )
                            });
                        assert_reader_inventory(&target.args).await;
                    }
                    RestoreExpectation::TypedUnsupportedFormat => {
                        let before =
                            snapshot_target_state(&target, "before-rejected-restore").await;
                        let Err(error) = cmd_restore(&target.args, &restore_path).await else {
                            panic!(
                                "{}",
                                super::format_compatibility_diagnostic(
                                    "retired format must be rejected"
                                )
                            );
                        };
                        assert!(matches!(
                            error.downcast_ref::<BackupError>(),
                            Some(BackupError::UnsupportedFormatVersion { backup_version, .. })
                                if *backup_version == entry.format_version
                        ));
                        assert_target_unmodified(&target, &before).await;
                    }
                }
            }
        }
    }

    async fn current_schema_version(target: &InitializedCommandEnv) -> i64 {
        let probe = target.base.path().join("schema-version-probe");
        compatibility_result(
            cmd_backup(&target.args, BackupMode::Directory, Some(probe.clone())).await,
            "export empty target to observe its schema version",
        );
        let bytes = compatibility_result(
            fs::read(probe.join("manifest.json")),
            "read schema-version probe manifest",
        );
        let manifest: Value = compatibility_result(
            serde_json::from_slice(&bytes),
            "parse schema-version probe manifest",
        );
        compatibility_option(
            manifest.get("schema_version").and_then(Value::as_i64),
            "backup manifest schema_version must be an integer",
        )
    }
    async fn assert_target_unmodified(
        target: &InitializedCommandEnv,
        before: &BTreeMap<String, Vec<u8>>,
    ) {
        let after = snapshot_target_state(target, "after-rejected-restore").await;
        assert_eq!(
            after, *before,
            "unsupported format must not mutate any database-table NDJSON or media bytes"
        );
    }

    async fn snapshot_target_state(
        target: &InitializedCommandEnv,
        name: &str,
    ) -> BTreeMap<String, Vec<u8>> {
        let backup = target.base.path().join(name);
        compatibility_result(
            cmd_backup(&target.args, BackupMode::Directory, Some(backup.clone())).await,
            "snapshot restore target",
        );
        let mut files = Vec::new();
        compatibility_result(
            super::collect_regular_files(&backup, Path::new(""), &mut files),
            "collect target snapshot",
        );
        files
            .into_iter()
            .filter_map(|(path, file)| {
                (path.starts_with("db/") || path.starts_with("media/")).then(|| {
                    (
                        path,
                        compatibility_result(std::fs::read(file), "read target snapshot entry"),
                    )
                })
            })
            .collect()
    }

    #[test]
    fn support_state_classifies_retired_entries_as_typed_unsupported_formats() {
        let retired = CorpusEntry {
            fixture: "retired-format".to_owned(),
            format_version: 2,
            support: SupportState::Retired,
            digest: "0".repeat(64),
        };
        assert_eq!(
            expected_restore(&retired),
            RestoreExpectation::TypedUnsupportedFormat
        );
    }
}

#[cfg(test)]
mod writer_tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::Path,
    };

    use jaunder::commands::cmd_backup;
    use rstest::*;
    use rstest_reuse::apply;
    use serde_json::Value;
    use storage::test_support::{Backend, backends};

    use crate::misc::backup_fixture::{BackupFixtureIds, populate_backup_fixture};

    use super::{
        BackupCorpus, CorpusIoMode, InitializedCommandEnv, SupportState, compatibility_option,
        compatibility_result, format_compatibility_diagnostic,
    };
    macro_rules! assert_eq {
        ($left:expr, $right:expr $(,)?) => {
            ::std::assert_eq!(
                $left,
                $right,
                "{}",
                super::format_compatibility_diagnostic("writer contract value differs")
            )
        };
        ($left:expr, $right:expr, $($message:tt)+) => {
            ::std::assert_eq!(
                $left,
                $right,
                "{}",
                super::format_compatibility_diagnostic(format_args!($($message)+))
            )
        };
    }

    macro_rules! assert {
        ($condition:expr $(,)?) => {
            ::std::assert!(
                $condition,
                "{}",
                super::format_compatibility_diagnostic("writer contract assertion failed")
            )
        };
        ($condition:expr, $($message:tt)+) => {
            ::std::assert!(
                $condition,
                "{}",
                super::format_compatibility_diagnostic(format_args!($($message)+))
            )
        };
    }

    const V1_TABLES: &[&str] = &[
        "audience_members",
        "audiences",
        "channels",
        "email_verifications",
        "feed_events",
        "idempotency_keys",
        "instance_identity",
        "invites",
        "media",
        "password_resets",
        "post_audiences",
        "post_media",
        "post_revision_audiences",
        "post_revision_tags",
        "post_revisions",
        "post_tags",
        "posts",
        "publisher_state",
        "sessions",
        "site_config",
        "subscription_statuses",
        "subscriptions",
        "tags",
        "target_kinds",
        "theme_content_eligibility",
        "theme_draft_assets",
        "theme_draft_charges",
        "theme_draft_content_charges",
        "theme_drafts",
        "theme_header_pool",
        "theme_owner_quotas",
        "theme_retained_content_charges",
        "theme_revision_assets",
        "theme_revisions",
        "theme_role_bindings",
        "theme_selections",
        "theme_site_quota",
        "themes",
        "user_config",
        "users",
    ];

    struct WriterRole {
        name: &'static str,
        seeded_source: &'static str,
    }

    const WRITER_ROLE_INVENTORY: &[WriterRole] = &[
        WriterRole {
            name: "null",
            seeded_source: "MediaRecord::source_url = None",
        },
        WriterRole {
            name: "boolean",
            seeded_source: "author User::is_operator = true and viewer User::is_operator = false",
        },
        WriterRole {
            name: "integer",
            seeded_source: "MediaRecord::size_bytes = 4",
        },
        WriterRole {
            name: "text",
            seeded_source: "author User::display_name = Backup User and UserConfig::DefaultPostFormat = org",
        },
        WriterRole {
            name: "relationships",
            seeded_source: "SeedRawPost author, named audience membership, and post audience assignment",
        },
        WriterRole {
            name: "media bytes",
            seeded_source: "storage/media/avatar.txt = media",
        },
    ];

    /// Reader fixture roles that the current storage API cannot emit. Keeping
    /// these reasons next to the writer oracle makes omitted raw-wire roles a
    /// deliberate compatibility boundary rather than accidental test coverage.
    const READER_ONLY_WRITER_INAPPLICABILITIES: &[(&str, &str)] = &[
        (
            "real",
            "the current public storage APIs expose no persisted real-valued backup column",
        ),
        (
            "structured JSON",
            "the current public storage APIs expose structured JSON only as text payloads, not JSON-valued backup columns",
        ),
    ];

    #[apply(backends)]
    #[tokio::test]
    async fn current_writer_satisfies_independent_v1_raw_wire_oracle(#[case] backend: Backend) {
        for output in CorpusIoMode::ALL {
            let source = InitializedCommandEnv::new(backend).await;
            let ids = populate_backup_fixture(&source.args).await;
            let written = source.base.path().join(match output {
                CorpusIoMode::Directory => "backup",
                CorpusIoMode::Archive => "backup.tar.gz",
            });
            let written_path = compatibility_result(
                cmd_backup(&source.args, output.backup_mode(), Some(written)).await,
                format_args!("write {} backup", output.name()),
            );
            let extracted = match output {
                CorpusIoMode::Directory => written_path,
                CorpusIoMode::Archive => {
                    let destination = source.base.path().join("extracted");
                    BackupCorpus::extract_archive(&written_path, &destination).unwrap_or_else(
                        |error| {
                            panic!(
                                "{}",
                                format_compatibility_diagnostic(format_args!(
                                    "independent archive extraction failed: {error}"
                                ))
                            )
                        },
                    );
                    destination
                }
            };

            assert_writer_version_is_uniquely_supported(&extracted);
            assert_v1_raw_wire_oracle(&extracted, output, &ids);
        }
    }

    fn assert_writer_version_is_uniquely_supported(export: &Path) {
        let manifest = read_manifest(export);
        let version = compatibility_option(
            manifest.get("format_version").and_then(Value::as_u64),
            "writer format_version must be an integer",
        );
        let corpus = compatibility_result(
            BackupCorpus::checked_in(),
            "load backup format compatibility corpus",
        );
        let matches = corpus
            .entries()
            .iter()
            .filter(|entry| {
                u64::from(entry.format_version) == version
                    && entry.support == SupportState::Supported
            })
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "{}",
            format_compatibility_diagnostic(
                "writer emitted a format version that does not resolve to exactly one supported fixture and oracle"
            )
        );
        assert_eq!(
            version,
            1,
            "{}",
            format_compatibility_diagnostic("format-1 is the sole current writer oracle")
        );
    }
    fn assert_v1_raw_wire_oracle(export: &Path, output: CorpusIoMode, ids: &BackupFixtureIds) {
        assert_inventory_is_complete();
        let manifest = read_manifest(export);
        let members = compatibility_option(manifest.as_object(), "manifest must be an object");
        let expected_members = BTreeSet::from([
            "format_version",
            "mode",
            "schema_checksum",
            "schema_version",
            "tables",
            "timestamp",
            "version",
        ]);
        assert_eq!(
            members.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            expected_members,
            "{}",
            format_compatibility_diagnostic("manifest members changed")
        );
        assert_eq!(
            manifest["format_version"],
            Value::from(1),
            "{}",
            format_compatibility_diagnostic("writer format_version changed")
        );
        assert!(
            manifest["version"].is_string(),
            "{}",
            format_compatibility_diagnostic("manifest version must be a string")
        );
        assert!(
            manifest["schema_version"].is_i64() || manifest["schema_version"].is_u64(),
            "{}",
            format_compatibility_diagnostic("manifest schema_version must be an integer")
        );
        assert!(
            manifest["schema_checksum"].is_string(),
            "{}",
            format_compatibility_diagnostic("manifest schema_checksum must be a string")
        );
        assert!(
            manifest["timestamp"].is_string(),
            "{}",
            format_compatibility_diagnostic("manifest timestamp must be a string")
        );
        assert_eq!(
            manifest["mode"],
            Value::from(match output {
                CorpusIoMode::Directory => "directory",
                CorpusIoMode::Archive => "archive",
            }),
            "{}",
            format_compatibility_diagnostic(
                "manifest mode does not match the requested corpus I/O mode"
            )
        );

        let tables = compatibility_option(
            manifest.get("tables").and_then(Value::as_array),
            "manifest tables must be an array",
        );
        assert_eq!(
            tables,
            &V1_TABLES
                .iter()
                .map(|table| Value::from(*table))
                .collect::<Vec<_>>(),
            "{}",
            format_compatibility_diagnostic("manifest tables must be alphabetical and exact")
        );

        let paths = regular_file_bytes(export);
        let expected_paths = std::iter::once("manifest.json".to_owned())
            .chain(V1_TABLES.iter().map(|table| format!("db/{table}.ndjson")))
            .chain(std::iter::once("media/avatar.txt".to_owned()))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            paths.keys().cloned().collect::<BTreeSet<_>>(),
            expected_paths,
            "{}",
            format_compatibility_diagnostic("export path set changed")
        );
        assert_eq!(
            compatibility_option(paths.get("media/avatar.txt"), "media member is missing"),
            b"media",
            "{}",
            format_compatibility_diagnostic("media bytes changed")
        );

        let rows = parse_ndjson_tables(&paths);
        assert_writer_roles(&rows, ids);
    }

    fn read_manifest(export: &Path) -> Value {
        let bytes = compatibility_result(fs::read(export.join("manifest.json")), "read manifest");
        compatibility_result(serde_json::from_slice(&bytes), "manifest JSON is invalid")
    }

    fn regular_file_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
            for entry in compatibility_result(
                fs::read_dir(directory),
                format_args!("read backup output directory {}", directory.display()),
            ) {
                let entry = compatibility_result(entry, "read backup output entry");
                let path = entry.path();
                let metadata = compatibility_result(
                    fs::symlink_metadata(&path),
                    format_args!("inspect backup output entry {}", path.display()),
                );
                if metadata.file_type().is_dir() {
                    visit(root, &path, files);
                } else {
                    assert!(
                        metadata.file_type().is_file(),
                        "output contains a special entry"
                    );
                    let relative_path = compatibility_result(
                        path.strip_prefix(root),
                        format_args!("backup output path {} is outside its root", path.display()),
                    );
                    let relative = compatibility_option(
                        relative_path.to_str(),
                        format_args!(
                            "backup output path {} is not UTF-8",
                            relative_path.display()
                        ),
                    )
                    .replace('\\', "/");
                    files.insert(
                        relative,
                        compatibility_result(fs::read(path), "read backup output file"),
                    );
                }
            }
        }

        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    fn parse_ndjson_tables(files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<Value>> {
        V1_TABLES
            .iter()
            .map(|table| {
                let path = format!("db/{table}.ndjson");
                let bytes = compatibility_option(
                    files.get(&path),
                    format_args!("manifest table {table} is missing its NDJSON member {path}"),
                );
                let rows = if bytes.is_empty() {
                    Vec::new()
                } else {
                    assert!(
                        bytes.ends_with(b"\n"),
                        "backup format compatibility: {path} must end with one LF-delimited JSON object per line"
                    );
                    bytes[..bytes.len() - 1]
                        .split(|byte| *byte == b'\n')
                        .map(|line| {
                            assert!(
                                !line.is_empty(),
                                "backup format compatibility: {path} has an empty NDJSON line"
                            );
                            let row: Value = serde_json::from_slice(line).unwrap_or_else(|error| {
                                panic!(
                                    "{}",
                                    format_compatibility_diagnostic(format_args!(
                                        "{path} has invalid NDJSON: {error}"
                                    ))
                                )
                            });
                            assert!(
                                row.is_object(),
                                "{path} has a non-object NDJSON line"
                            );
                            row
                        })
                        .collect()
                };
                ((*table).to_owned(), rows)
            })
            .collect()
    }

    fn table_rows<'a>(rows: &'a BTreeMap<String, Vec<Value>>, table: &str) -> &'a Vec<Value> {
        compatibility_option(
            rows.get(table),
            format_args!("writer rows are missing required table {table}"),
        )
    }

    fn assert_writer_roles(rows: &BTreeMap<String, Vec<Value>>, ids: &BackupFixtureIds) {
        let author = ids.author.to_string();
        let viewer = ids.viewer.to_string();
        let audience = ids.audience.to_string();
        let subscription = ids.subscription.to_string();
        assert!(
            table_rows(rows, "users").iter().any(|row| {
                row_id_is(row, "user_id", &author)
                    && row["username"] == "backupuser"
                    && row["display_name"] == "Backup User"
                    && row["is_operator"] == true
            }),
            "backup format compatibility: author seed must emit its exact boolean and text values"
        );
        assert!(
            table_rows(rows, "users").iter().any(|row| {
                row_id_is(row, "user_id", &viewer)
                    && row["username"] == "viewer"
                    && row["display_name"] == "Viewer"
                    && row["is_operator"] == false
            }),
            "backup format compatibility: viewer seed must emit its exact boolean and text values"
        );
        assert!(
            table_rows(rows, "media").iter().any(|row| {
                row_id_is(row, "user_id", &author)
                    && row["sha256"]
                        == "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    && row["filename"] == "my%20photo.jpg"
                    && row["source"] == "upload"
                    && row["content_type"] == "image/jpeg"
                    && numeric_is(row, "size_bytes", 4)
                    && row["source_url"].is_null()
            }),
            "backup format compatibility: media seed must emit exact null, integer, and text values"
        );
        assert!(
            table_rows(rows, "user_config").iter().any(|row| {
                row_id_is(row, "user_id", &author)
                    && row["key"] == "posts.default_format"
                    && row["value"] == "org"
            }),
            "backup format compatibility: user-config seed must emit its exact text value"
        );
        assert!(
            table_rows(rows, "posts").iter().any(|row| {
                row_id_is(row, "post_id", &ids.public_post.to_string())
                    && row_id_is(row, "user_id", &author)
            }),
            "backup format compatibility: public post must retain its author relationship"
        );
        assert!(
            table_rows(rows, "audiences").iter().any(|row| {
                row_id_is(row, "audience_id", &audience)
                    && row_id_is(row, "author_user_id", &author)
            }),
            "{}",
            format_compatibility_diagnostic(
                "named audience must retain its exact audience and author IDs"
            )
        );
        assert!(
            table_rows(rows, "audience_members").iter().any(|row| {
                row_id_is(row, "audience_id", &audience)
                    && row_id_is(row, "author_user_id", &author)
                    && row_id_is(row, "subscription_id", &subscription)
            }),
            "{}",
            format_compatibility_diagnostic(
                "named-audience membership must retain exact audience, author, and member subscription IDs"
            )
        );
        assert!(
            table_rows(rows, "post_audiences").iter().any(|row| {
                row_id_is(row, "post_id", &ids.named_post.to_string())
                    && row_id_is(row, "audience_id", &audience)
            }),
            "{}",
            format_compatibility_diagnostic(
                "named post must retain its exact post and audience IDs"
            )
        );
    }

    fn assert_inventory_is_complete() {
        assert_eq!(
            WRITER_ROLE_INVENTORY
                .iter()
                .map(|role| role.name)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "boolean",
                "integer",
                "media bytes",
                "null",
                "relationships",
                "text",
            ]),
            "backup format compatibility: writer inventory must name every applicable seeded wire role"
        );
        assert!(
            WRITER_ROLE_INVENTORY
                .iter()
                .all(|role| !role.seeded_source.is_empty())
        );
        assert_eq!(
            READER_ONLY_WRITER_INAPPLICABILITIES
                .iter()
                .map(|(role, _)| *role)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["real", "structured JSON"]),
            "backup format compatibility: reader-only inventory must explicitly account for real and structured JSON"
        );
        assert!(
            READER_ONLY_WRITER_INAPPLICABILITIES
                .iter()
                .all(|(_, rationale)| !rationale.is_empty())
        );
    }

    fn numeric_is(row: &Value, key: &str, expected: u64) -> bool {
        row[key].as_u64().is_some_and(|value| value == expected)
            || row[key]
                .as_str()
                .is_some_and(|value| value == expected.to_string())
    }

    fn row_id_is(row: &Value, key: &str, expected: &str) -> bool {
        row[key].as_str().map_or_else(
            || {
                row[key]
                    .as_u64()
                    .is_some_and(|value| value.to_string() == expected)
            },
            |value| value == expected,
        )
    }
}
