use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
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

    /// Packages a verified, materialized corpus directory with test-owned tar
    /// and gzip code so archive-reader coverage cannot share production helpers.
    pub(crate) fn package_archive(
        source: &Path,
        destination: &Path,
    ) -> Result<(), BackupCorpusError> {
        let output = fs::File::create(destination)?;
        let encoder = GzEncoder::new(output, Compression::default());
        let mut archive = tar::Builder::new(encoder);
        archive.append_dir_all(".", source)?;
        archive.into_inner()?.finish()?;
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

#[cfg(test)]
mod reader_tests {
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
        commands::{cmd_backup, cmd_init, cmd_restore},
    };
    use rstest::*;
    use rstest_reuse::apply;
    use storage::{
        BackupError, BackupMode, EmailVerified, StorageRuntimeConfig, open_existing_database,
        test_support::{
            Backend, PostgresDbGuard, PostgresTestConfig, backends, sqlite_url, unique_postgres_url,
        },
    };
    use tempfile::TempDir;

    use super::{BackupCorpus, CorpusEntry, SupportState};

    struct InitializedCommandEnv {
        args: StorageArgs,
        base: TempDir,
        _postgres: Option<PostgresDbGuard>,
    }

    impl InitializedCommandEnv {
        async fn new(backend: Backend) -> Self {
            let base = TempDir::new().expect("create command environment");
            let (db, postgres) = match backend {
                Backend::Sqlite => (sqlite_url(&base), None),
                Backend::Postgres => {
                    let config = PostgresTestConfig::from_env();
                    let (db, guard) = unique_postgres_url(&config).await;
                    (db, Some(guard))
                }
            };
            let args = StorageArgs {
                storage_path: base.path().join("storage"),
                db,
            };
            cmd_init(&args, false).await.expect("initialize target");
            Self {
                args,
                base,
                _postgres: postgres,
            }
        }
    }

    #[derive(Clone, Copy)]
    enum RestoreInput {
        Directory,
        Archive,
    }

    impl RestoreInput {
        const ALL: [Self; 2] = [Self::Directory, Self::Archive];

        fn name(self) -> &'static str {
            match self {
                Self::Directory => "directory",
                Self::Archive => "archive",
            }
        }
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

    #[apply(backends)]
    #[tokio::test]
    async fn historical_corpus_entries_restore_through_each_public_input(#[case] backend: Backend) {
        let corpus = BackupCorpus::checked_in().expect("load corpus");
        for entry in corpus.entries() {
            for input in RestoreInput::ALL {
                let target = InitializedCommandEnv::new(backend).await;
                let schema_version = current_schema_version(&target).await;
                let materialized = target.base.path().join("materialized");
                corpus
                    .materialize(entry, &materialized, schema_version)
                    .expect("materialize verified historical fixture");
                let restore_path = match input {
                    RestoreInput::Directory => materialized,
                    RestoreInput::Archive => {
                        let archive = target.base.path().join("fixture.tar.gz");
                        BackupCorpus::package_archive(&materialized, &archive)
                            .expect("independently package fixture archive");
                        archive
                    }
                };

                match expected_restore(entry) {
                    RestoreExpectation::Succeeds => {
                        cmd_restore(&target.args, &restore_path)
                            .await
                            .unwrap_or_else(|error| {
                                panic!(
                                    "restore supported format {} from {}: {error:#}",
                                    entry.format_version,
                                    input.name()
                                )
                            });
                        assert_reader_inventory(&target.args).await;
                    }
                    RestoreExpectation::TypedUnsupportedFormat => {
                        let error = cmd_restore(&target.args, &restore_path)
                            .await
                            .expect_err("retired format must be rejected");
                        assert!(matches!(
                            error.downcast_ref::<BackupError>(),
                            Some(BackupError::UnsupportedFormatVersion { backup_version, .. })
                                if *backup_version == entry.format_version
                        ));
                        assert_target_unmodified(&target.args).await;
                    }
                }
            }
        }
    }

    async fn current_schema_version(target: &InitializedCommandEnv) -> i64 {
        let probe = target.base.path().join("schema-version-probe");
        cmd_backup(&target.args, BackupMode::Directory, Some(probe.clone()))
            .await
            .expect("export empty target to observe its schema version");
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(probe.join("manifest.json")).expect("read probe"),
        )
        .expect("parse probe manifest");
        manifest["schema_version"]
            .as_i64()
            .expect("backup manifest schema version")
    }

    async fn assert_reader_inventory(args: &StorageArgs) {
        let state = open_existing_database(&args.db, &StorageRuntimeConfig::default())
            .await
            .expect("open restored database");
        let user_id = UserId::from(41);
        let post_id = PostId::from(71);
        let username: Username = "legacyuser".parse().expect("fixture username");
        let user = state
            .users
            .get_user_by_username(&username)
            .await
            .expect("read restored user")
            .expect("fixture user exists");
        assert_eq!(user.user_id, user_id);
        assert_eq!(user.display_name, None);
        assert_eq!(user.email_verified, EmailVerified::UNVERIFIED);
        assert_eq!(user.is_operator, storage::OperatorStatus::OPERATOR);

        let post = state
            .posts
            .get_post_by_id(post_id, &ViewerIdentity::local(user_id))
            .await
            .expect("read restored post")
            .expect("fixture post exists");
        assert_eq!(post.user_id, user_id);
        assert_eq!(post.title.as_deref(), Some("Legacy wire roles"));
        assert_eq!(post.body.as_ref(), "legacy body");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(post.rendered_html.as_ref())
                .expect("structured JSON survives semantically"),
            serde_json::json!({"kind": "structured", "items": [1, true]})
        );
        assert_eq!(post.published_at, None);

        let history = state
            .posts
            .list_post_revision_history(user_id, post_id, None, PageSize::default())
            .await
            .expect("read restored revision history")
            .expect("fixture post history exists");
        assert_eq!(history.revisions.len(), 1);
        let revision = state
            .posts
            .get_post_revision_detail(user_id, post_id, history.revisions[0].revision_id)
            .await
            .expect("read restored revision")
            .expect("fixture revision exists")
            .revision;
        assert_eq!(revision.post_id, post_id);
        assert_eq!(revision.user_id, user_id);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(revision.rendered_html.as_ref())
                .expect("real JSON survives semantically"),
            serde_json::json!(1.25)
        );

        let media = state
            .media
            .get_media(
                user_id,
                &parse_content_hash(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                ),
                &parse_filename("avatar.txt"),
                &MediaSource::Upload,
            )
            .await
            .expect("read restored media")
            .expect("fixture media exists");
        assert_eq!(media.user_id, user_id);
        assert_eq!(media.size_bytes.to_string(), "13");
        assert_eq!(media.source_url, None);
        assert_eq!(
            std::fs::read(args.storage_path.join("media").join("avatar.txt"))
                .expect("read restored media bytes"),
            b"legacy media\n"
        );
    }
    async fn assert_target_unmodified(args: &StorageArgs) {
        let state = open_existing_database(&args.db, &StorageRuntimeConfig::default())
            .await
            .expect("open rejected restore target");
        let username: Username = "legacyuser".parse().expect("fixture username");
        assert!(
            state
                .users
                .get_user_by_username(&username)
                .await
                .expect("read rejected target")
                .is_none(),
            "unsupported format must not mutate database"
        );
        assert!(
            !args.storage_path.join("media").join("avatar.txt").exists(),
            "unsupported format must not mutate media"
        );
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
        io::Read,
        path::Path,
    };

    use flate2::read::GzDecoder;
    use jaunder::{
        cli::StorageArgs,
        commands::{cmd_backup, cmd_init},
    };
    use rstest::*;
    use rstest_reuse::apply;
    use serde_json::Value;
    use storage::{
        BackupMode,
        test_support::{
            Backend, PostgresDbGuard, PostgresTestConfig, backends, sqlite_url, unique_postgres_url,
        },
    };
    use tempfile::TempDir;

    use crate::misc::backup_fixture::{BackupFixtureIds, populate_backup_fixture};

    use super::{BackupCorpus, SupportState};

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

    struct InitializedCommandEnv {
        args: StorageArgs,
        base: TempDir,
        _postgres: Option<PostgresDbGuard>,
    }

    impl InitializedCommandEnv {
        async fn new(backend: Backend) -> Self {
            let base = TempDir::new().expect("create command environment");
            let (db, postgres) = match backend {
                Backend::Sqlite => (sqlite_url(&base), None),
                Backend::Postgres => {
                    let config = PostgresTestConfig::from_env();
                    let (db, guard) = unique_postgres_url(&config).await;
                    (db, Some(guard))
                }
            };
            let args = StorageArgs {
                storage_path: base.path().join("storage"),
                db,
            };
            cmd_init(&args, false).await.expect("initialize target");
            Self {
                args,
                base,
                _postgres: postgres,
            }
        }
    }

    #[derive(Clone, Copy)]
    enum BackupOutput {
        Directory,
        Archive,
    }

    impl BackupOutput {
        const ALL: [Self; 2] = [Self::Directory, Self::Archive];

        fn mode(self) -> BackupMode {
            match self {
                Self::Directory => BackupMode::Directory,
                Self::Archive => BackupMode::Archive,
            }
        }

        fn name(self) -> &'static str {
            match self {
                Self::Directory => "directory",
                Self::Archive => "archive",
            }
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn current_writer_satisfies_independent_v1_raw_wire_oracle(#[case] backend: Backend) {
        for output in BackupOutput::ALL {
            let source = InitializedCommandEnv::new(backend).await;
            let ids = populate_backup_fixture(&source.args).await;
            let written = source.base.path().join(match output {
                BackupOutput::Directory => "backup",
                BackupOutput::Archive => "backup.tar.gz",
            });
            let written_path = cmd_backup(&source.args, output.mode(), Some(written))
                .await
                .unwrap_or_else(|error| panic!("write {} backup: {error:#}", output.name()));
            let extracted = match output {
                BackupOutput::Directory => written_path,
                BackupOutput::Archive => {
                    let destination = source.base.path().join("extracted");
                    extract_archive(&written_path, &destination);
                    destination
                }
            };

            assert_writer_version_is_uniquely_supported(&extracted);
            assert_v1_raw_wire_oracle(&extracted, output, &ids);
        }
    }

    fn assert_writer_version_is_uniquely_supported(export: &Path) {
        let manifest = read_manifest(export);
        let version = manifest["format_version"]
            .as_u64()
            .expect("backup format compatibility: writer format_version must be an integer");
        let corpus = BackupCorpus::checked_in().expect("load backup format compatibility corpus");
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
            "backup format compatibility: writer emitted format version {version}, which must resolve to exactly one supported fixture/oracle entry; add a new immutable fixture, oracle, and corpus-index entry instead of changing format-1 history"
        );
        assert_eq!(
            version, 1,
            "backup format compatibility: format-1 oracle is the sole current writer oracle; add a new immutable fixture, oracle, and corpus-index entry instead of changing format-1 history"
        );
    }
    fn assert_v1_raw_wire_oracle(export: &Path, output: BackupOutput, ids: &BackupFixtureIds) {
        assert_inventory_is_complete();
        let manifest = read_manifest(export);
        let members = manifest
            .as_object()
            .expect("backup format compatibility: manifest must be an object");
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
            "backup format compatibility: manifest members changed; add a new immutable fixture, oracle, and corpus-index entry instead of changing format-1 history"
        );
        assert_eq!(manifest["format_version"], Value::from(1));
        assert!(manifest["version"].is_string());
        assert!(manifest["schema_version"].is_i64() || manifest["schema_version"].is_u64());
        assert!(manifest["schema_checksum"].is_string());
        assert!(manifest["timestamp"].is_string());
        assert_eq!(
            manifest["mode"],
            Value::from(match output {
                BackupOutput::Directory => "directory",
                BackupOutput::Archive => "archive",
            })
        );
        assert_eq!(
            manifest["tables"]
                .as_array()
                .expect("manifest tables array"),
            &V1_TABLES
                .iter()
                .map(|table| Value::from(*table))
                .collect::<Vec<_>>(),
            "backup format compatibility: manifest tables must be alphabetical and exact; add a new immutable fixture, oracle, and corpus-index entry instead of changing format-1 history"
        );

        let paths = regular_file_bytes(export);
        let expected_paths = std::iter::once("manifest.json".to_owned())
            .chain(V1_TABLES.iter().map(|table| format!("db/{table}.ndjson")))
            .chain(std::iter::once("media/avatar.txt".to_owned()))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            paths.keys().cloned().collect::<BTreeSet<_>>(),
            expected_paths,
            "backup format compatibility: export path set changed; add a new immutable fixture, oracle, and corpus-index entry instead of changing format-1 history"
        );
        assert_eq!(paths["media/avatar.txt"], b"media");

        let rows = parse_ndjson_tables(&paths);
        assert_writer_roles(&rows, ids);
    }

    fn read_manifest(export: &Path) -> Value {
        serde_json::from_slice(
            &fs::read(export.join("manifest.json"))
                .expect("backup format compatibility: read manifest"),
        )
        .expect("backup format compatibility: parse manifest JSON")
    }

    fn regular_file_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(directory).expect("read backup output directory") {
                let entry = entry.expect("read backup output entry");
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).expect("inspect backup output entry");
                if metadata.file_type().is_dir() {
                    visit(root, &path, files);
                } else {
                    assert!(
                        metadata.file_type().is_file(),
                        "backup format compatibility: output contains a special entry"
                    );
                    let relative = path
                        .strip_prefix(root)
                        .expect("backup output child")
                        .to_str()
                        .expect("UTF-8 backup output path")
                        .replace('\\', "/");
                    files.insert(relative, fs::read(path).expect("read backup output file"));
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
                let bytes = &files[&path];
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
                            let row: Value =
                                serde_json::from_slice(line).unwrap_or_else(|error| {
                                    panic!(
                                        "backup format compatibility: {path} has invalid NDJSON: {error}"
                                    )
                                });
                            assert!(
                                row.is_object(),
                                "backup format compatibility: {path} has a non-object NDJSON line; add a new immutable fixture, oracle, and corpus-index entry instead of changing format-1 history"
                            );
                            row
                        })
                        .collect()
                };
                ((*table).to_owned(), rows)
            })
            .collect()
    }

    fn assert_writer_roles(rows: &BTreeMap<String, Vec<Value>>, ids: &BackupFixtureIds) {
        let author = ids.author.to_string();
        let viewer = ids.viewer.to_string();
        assert!(
            rows["users"].iter().any(|row| {
                row_id_is(row, "user_id", &author)
                    && row["username"] == "backupuser"
                    && row["display_name"] == "Backup User"
                    && row["is_operator"] == true
            }),
            "backup format compatibility: author seed must emit its exact boolean and text values"
        );
        assert!(
            rows["users"].iter().any(|row| {
                row_id_is(row, "user_id", &viewer)
                    && row["username"] == "viewer"
                    && row["display_name"] == "Viewer"
                    && row["is_operator"] == false
            }),
            "backup format compatibility: viewer seed must emit its exact boolean and text values"
        );
        assert!(
            rows["media"].iter().any(|row| {
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
            rows["user_config"].iter().any(|row| {
                row_id_is(row, "user_id", &author)
                    && row["key"] == "posts.default_format"
                    && row["value"] == "org"
            }),
            "backup format compatibility: user-config seed must emit its exact text value"
        );
        assert!(
            rows["posts"].iter().any(|row| {
                row_id_is(row, "post_id", &ids.public_post.to_string())
                    && row_id_is(row, "user_id", &author)
            }),
            "backup format compatibility: public post must retain its author relationship"
        );
        assert!(
            rows["audience_members"]
                .iter()
                .any(|row| row_id_is(row, "author_user_id", &author)),
            "backup format compatibility: named-audience membership must retain its author relationship"
        );
        assert!(
            rows["post_audiences"].iter().any(|row| row_id_is(
                row,
                "post_id",
                &ids.named_post.to_string()
            )),
            "backup format compatibility: named post must retain its audience relationship"
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

    fn extract_archive(source: &Path, destination: &Path) {
        fs::create_dir(destination).expect("create archive extraction directory");
        let input = fs::File::open(source).expect("open production archive");
        let decoder = GzDecoder::new(input);
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries().expect("read archive entries") {
            let mut entry = entry.expect("read archive entry");
            let relative = entry.path().expect("read archive entry path").into_owned();
            let destination_path = destination.join(&relative);
            if entry.header().entry_type().is_dir() {
                fs::create_dir_all(destination_path).expect("create extracted archive directory");
            } else {
                assert!(
                    entry.header().entry_type().is_file(),
                    "backup format compatibility: archive contains a non-file entry"
                );
                if let Some(parent) = destination_path.parent() {
                    fs::create_dir_all(parent).expect("create extracted archive parent");
                }
                let mut bytes = Vec::new();
                entry
                    .read_to_end(&mut bytes)
                    .expect("read archive entry bytes");
                fs::write(destination_path, bytes).expect("write extracted archive entry");
            }
        }
    }
}
