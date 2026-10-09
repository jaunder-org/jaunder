//! Deterministic system styling artifacts produced from closed release inputs.
//!
//! Bundled public Themes deliberately take the same portable archive, validation,
//! and CSS-compilation path as owner-authored Theme Packages. Application CSS is
//! not a Theme Package: it styles protected/private application structure and is
//! consequently admitted only through the private, compiled-in application role
//! below. Both roles expose exact content bytes and digests without inventing a
//! second package compiler or hashing scheme.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::Path,
};

use common::theme::{
    Theme, ThemeAssetDigest, ThemeContentDigest, ThemeRevisionDigest, ThemeSourceDigest,
    ThemeStylesheetDigest,
};
use thiserror::Error;

use crate::theme_package::{
    CompiledThemeContent, CompiledThemeRevision, ThemePackageError, ThemePackageLimits,
    export_theme_package, validate_theme_package,
};

const APPLICATION_CSS: &[u8] = include_bytes!("../../server/assets/jaunder.css");
const STUDIO_MANIFEST: &[u8] = include_bytes!("../system_theme_sources/studio/theme.json");
const STUDIO_CSS: &[u8] = include_bytes!("../system_theme_sources/studio/style.css");
const TERMINAL_MANIFEST: &[u8] = include_bytes!("../system_theme_sources/terminal/theme.json");
const TERMINAL_CSS: &[u8] = include_bytes!("../system_theme_sources/terminal/style.css");
const READER_MANIFEST: &[u8] = include_bytes!("../system_theme_sources/reader/theme.json");
const READER_CSS: &[u8] = include_bytes!("../system_theme_sources/reader/style.css");

/// Closed role for the one trusted application stylesheet.
///
/// This type has no public constructor and is not accepted by Theme Package
/// mutation APIs; it prevents an owner-authored source from obtaining the
/// application stylesheet's unscoped authority. Its only exposed payload is the
/// compiler's shared MIME/bytes/digest view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationStylesheet {
    bytes: Vec<u8>,
}

impl ApplicationStylesheet {
    /// Returns the shared compiler content view, minted only through the
    /// crate-owned trusted application role.
    #[must_use]
    pub fn content(&self) -> CompiledThemeContent<'_> {
        CompiledThemeContent::trusted_system("text/css; charset=utf-8", &self.bytes)
    }

    #[must_use]
    pub fn content_digest(&self) -> ThemeContentDigest {
        ThemeContentDigest::from_digest(self.content().digest())
    }
}

/// One bundled public Theme compiled by the regular Theme Package compiler.
#[derive(Debug)]
pub struct BundledThemePackage {
    theme: Theme,
    source_digest: [u8; 32],
    package_bytes: Vec<u8>,
    revision: CompiledThemeRevision,
}

impl BundledThemePackage {
    #[must_use]
    pub const fn theme(&self) -> Theme {
        self.theme
    }

    #[must_use]
    pub fn source_digest(&self) -> ThemeSourceDigest {
        ThemeSourceDigest::from_digest(self.source_digest)
    }

    /// Returns the canonical validated portable package source replayed by the
    /// system artifact producer.
    #[must_use]
    pub fn package_bytes(&self) -> &[u8] {
        &self.package_bytes
    }

    #[must_use]
    pub fn revision_digest(&self) -> ThemeRevisionDigest {
        ThemeRevisionDigest::from_digest(self.revision.revision_digest())
    }

    #[must_use]
    pub fn stylesheet_digest(&self) -> ThemeStylesheetDigest {
        ThemeStylesheetDigest::from_digest(self.revision.stylesheet_content().digest())
    }

    #[must_use]
    pub fn stylesheet_content(&self) -> CompiledThemeContent<'_> {
        self.revision.stylesheet_content()
    }

    #[must_use]
    pub fn revision(&self) -> &CompiledThemeRevision {
        &self.revision
    }
}

/// Complete deterministic inventory for one release's system styling inputs.
#[derive(Debug)]
pub struct SystemArtifactInventory {
    application: ApplicationStylesheet,
    themes: [BundledThemePackage; 3],
}

impl SystemArtifactInventory {
    #[must_use]
    pub const fn application(&self) -> &ApplicationStylesheet {
        &self.application
    }

    #[must_use]
    pub fn theme(&self, theme: Theme) -> &BundledThemePackage {
        self.themes
            .iter()
            .find(|package| package.theme == theme)
            .unwrap_or_else(|| {
                // Both private constructors populate every closed Theme variant.
                unreachable!("the closed Theme enum has a bundled package")
            })
    }

    pub fn themes(&self) -> impl Iterator<Item = &BundledThemePackage> {
        self.themes.iter()
    }
}

/// System source failed the same package admission/compilation boundary used by
/// custom Theme Packages.
#[derive(Debug, Error)]
#[error("invalid bundled Theme Package `{theme}`: {source}")]
pub struct SystemThemeError {
    theme: Theme,
    #[source]
    source: ThemePackageError,
}

/// Staging the producer's exact compiled inventory failed.
#[derive(Debug, Error)]
#[error("staging system artifacts at {path}: {source}")]
pub struct SystemArtifactStageError {
    path: std::path::PathBuf,
    #[source]
    source: std::io::Error,
}

/// A source of staged system-artifact files.
///
/// This is an intentionally open transport seam for the trusted build tree and
/// private binary embed. The loader authenticates neither provider nor release;
/// composition roots choose those trusted sources. It does enforce the fixed
/// role grammar and never exposes application-CSS construction to custom APIs.
pub trait SystemArtifactSource {
    /// Returns the complete relative file inventory.
    ///
    /// # Errors
    ///
    /// Returns an error when the source inventory cannot be enumerated.
    fn paths(&self) -> Result<BTreeSet<String>, SystemArtifactLoadError>;

    /// Returns exact bytes for one declared relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when the declared artifact cannot be read.
    fn read(&self, path: &str) -> Result<Vec<u8>, SystemArtifactLoadError>;
}

/// A persisted system-artifact tree was malformed or disagreed with compiler-minted identity.
#[derive(Debug, Error)]
pub enum SystemArtifactLoadError {
    #[error("invalid system-artifact inventory: {0}")]
    Invalid(String),
    #[error("reading system artifact `{path}`: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid bundled Theme Package `{theme}`: {source}")]
    Package {
        theme: Theme,
        #[source]
        source: ThemePackageError,
    },
}

/// Filesystem source for a producer-staged system-artifact tree.
pub struct DirectorySystemArtifactSource<'a> {
    root: &'a Path,
}

impl<'a> DirectorySystemArtifactSource<'a> {
    #[must_use]
    pub const fn new(root: &'a Path) -> Self {
        Self { root }
    }
}

impl SystemArtifactSource for DirectorySystemArtifactSource<'_> {
    fn paths(&self) -> Result<BTreeSet<String>, SystemArtifactLoadError> {
        let mut paths = BTreeSet::new();
        collect_source_paths(self.root, Path::new(""), &mut paths)?;
        Ok(paths)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, SystemArtifactLoadError> {
        fs::read(self.root.join(path)).map_err(|source| SystemArtifactLoadError::Read {
            path: path.to_owned(),
            source,
        })
    }
}

fn collect_source_paths(
    root: &Path,
    relative: &Path,
    paths: &mut BTreeSet<String>,
) -> Result<(), SystemArtifactLoadError> {
    let directory = root.join(relative);
    for entry in fs::read_dir(&directory).map_err(|source| SystemArtifactLoadError::Read {
        path: relative.display().to_string(),
        source,
    })? {
        // cov:ignore-start: A ReadDir item error requires a live filesystem fault/race; std exposes no deterministic item-error injection seam.
        let entry = entry.map_err(|source| SystemArtifactLoadError::Read {
            path: relative.display().to_string(),
            source,
        })?;
        // cov:ignore-stop
        let child = relative.join(entry.file_name());
        // cov:ignore-start: DirEntry file_type failure requires a post-enumeration OS fault on the authoritative Linux filesystem; normal entries and invalid node kinds are tested.
        let file_type = entry
            .file_type()
            .map_err(|source| SystemArtifactLoadError::Read {
                path: child.display().to_string(),
                source,
            })?;
        // cov:ignore-stop
        if file_type.is_dir() {
            collect_source_paths(root, &child, paths)?;
        } else if file_type.is_file() {
            let path = child.to_str().ok_or_else(|| {
                SystemArtifactLoadError::Invalid("artifact path is not UTF-8".into())
            })?;
            if path.contains('\\') || path.split('/').any(|part| matches!(part, "" | "." | "..")) {
                return Err(SystemArtifactLoadError::Invalid(
                    "artifact path is not canonical".into(),
                ));
            }
            paths.insert(path.to_owned());
        } else {
            return Err(SystemArtifactLoadError::Invalid(
                "artifact tree contains a non-file".into(),
            ));
        }
    }
    Ok(())
}

/// Loads a staged inventory only when every declared role replayed through the
/// canonical validator/compiler agrees with its exact source and compiled bytes.
///
/// # Errors
///
/// Rejects incomplete, extra, duplicate, traversal-shaped, or inconsistent staged input.
pub fn load_system_artifact_inventory(
    source: &dyn SystemArtifactSource,
) -> Result<SystemArtifactInventory, SystemArtifactLoadError> {
    let expected = BTreeSet::from([
        "inventory.txt".to_owned(),
        "application.css".to_owned(),
        "themes/terminal.css".to_owned(),
        "themes/studio.css".to_owned(),
        "themes/reader.css".to_owned(),
        "packages/terminal.zip".to_owned(),
        "packages/studio.zip".to_owned(),
        "packages/reader.zip".to_owned(),
    ]);
    if source.paths()? != expected {
        return Err(SystemArtifactLoadError::Invalid(
            "artifact paths are not the closed producer inventory".into(),
        ));
    }
    let manifest = String::from_utf8(source.read("inventory.txt")?)
        .map_err(|_| SystemArtifactLoadError::Invalid("inventory is not UTF-8".into()))?;
    let application = ApplicationStylesheet {
        bytes: source.read("application.css")?,
    };
    let application_content = application.content();
    let mut records = manifest.lines();
    let application_record = records.next().ok_or_else(|| {
        SystemArtifactLoadError::Invalid("inventory omits application role".into())
    })?;
    let expected_application = format!(
        "application\tmime={}\tdigest={}\tbytes={}\tpath=application.css",
        application_content.mime(),
        application.content_digest(),
        application_content.bytes().len(),
    );
    if application_record != expected_application {
        return Err(SystemArtifactLoadError::Invalid(
            "application role metadata disagrees with bytes".into(),
        ));
    }
    let terminal = load_staged_theme(source, Theme::Terminal, records.next())?;
    let studio = load_staged_theme(source, Theme::Studio, records.next())?;
    let reader = load_staged_theme(source, Theme::Reader, records.next())?;
    if records.next().is_some() {
        return Err(SystemArtifactLoadError::Invalid(
            "inventory has extra roles".into(),
        ));
    }
    Ok(SystemArtifactInventory {
        application,
        themes: [terminal, studio, reader],
    })
}

fn load_staged_theme(
    source: &dyn SystemArtifactSource,
    theme: Theme,
    record: Option<&str>,
) -> Result<BundledThemePackage, SystemArtifactLoadError> {
    let package_path = format!("packages/{}.zip", theme.token());
    let stylesheet_path = format!("themes/{}.css", theme.token());
    let package_bytes = source.read(&package_path)?;
    let stylesheet = source.read(&stylesheet_path)?;
    let validated = validate_theme_package(&package_bytes, ThemePackageLimits::default())
        .map_err(|source| SystemArtifactLoadError::Package { theme, source })?;
    let source_digest = validated.source_digest();
    if validated
        .export_archive()
        .map_err(|source| SystemArtifactLoadError::Package { theme, source })?
        != package_bytes
    {
        return Err(SystemArtifactLoadError::Invalid(format!(
            "package source is not canonical for {}",
            theme.token()
        )));
    }
    let asset_urls = validated
        .asset_digests()
        .map(|(path, digest)| {
            (
                path.to_owned(),
                ThemeAssetDigest::from_digest(digest)
                    .content_url()
                    .to_string(),
            )
        })
        .collect();
    let revision = validated
        .compile(&asset_urls, ThemePackageLimits::default())
        .map_err(|source| SystemArtifactLoadError::Package { theme, source })?;
    if revision.stylesheet_content().bytes() != stylesheet {
        return Err(SystemArtifactLoadError::Invalid(format!(
            "compiled stylesheet disagrees for {}",
            theme.token()
        )));
    }
    let expected = format!(
        "theme={}\tsource={}\tsource_path={package_path}\trevision={}\tmime={}\tdigest={}\tbytes={}\tpath={stylesheet_path}",
        theme.token(),
        ThemeSourceDigest::from_digest(source_digest),
        ThemeRevisionDigest::from_digest(revision.revision_digest()),
        revision.stylesheet_content().mime(),
        ThemeStylesheetDigest::from_digest(revision.stylesheet_content().digest()),
        stylesheet.len(),
    );
    if record != Some(expected.as_str()) {
        return Err(SystemArtifactLoadError::Invalid(format!(
            "theme metadata disagrees for {}",
            theme.token()
        )));
    }
    Ok(BundledThemePackage {
        theme,
        source_digest,
        package_bytes,
        revision,
    })
}

/// Writes the compiler-minted system inventory as a build-only artifact tree.
///
/// The manifest is the producer-owned naming map: callers choose only the root,
/// never individual filenames, digests, or bytes. This tree is not a public
/// asset route; later installation/serving work consumes its typed inventory.
///
/// # Errors
///
/// Returns an error when replacing or writing the destination tree fails.
pub fn stage_system_artifact_inventory(
    inventory: &SystemArtifactInventory,
    root: &Path,
) -> Result<(), SystemArtifactStageError> {
    if root.exists() {
        fs::remove_dir_all(root).map_err(|source| SystemArtifactStageError {
            path: root.to_path_buf(),
            source,
        })?;
    }
    fs::create_dir_all(root.join("themes")).map_err(|source| SystemArtifactStageError {
        path: root.to_path_buf(),
        source,
    })?;
    // cov:ignore-start: themes/ was just created under the same fresh root; failure creating its packages/ sibling requires an intervening OS race or resource fault.
    fs::create_dir_all(root.join("packages")).map_err(|source| SystemArtifactStageError {
        path: root.to_path_buf(),
        source,
    })?;
    // cov:ignore-stop

    let application = inventory.application.content();
    write_staged(root.join("application.css"), application.bytes())?;
    let mut manifest = format!(
        "application\tmime={}\tdigest={}\tbytes={}\tpath=application.css\n",
        application.mime(),
        inventory.application.content_digest(),
        application.bytes().len(),
    );
    for package in inventory.themes() {
        let stylesheet = package.stylesheet_content();
        let path = format!("themes/{}.css", package.theme().token());
        let package_path = format!("packages/{}.zip", package.theme().token());
        write_staged(root.join(&path), stylesheet.bytes())?;
        write_staged(root.join(&package_path), package.package_bytes())?;
        // Formatting these closed values into a String cannot fail.
        let _ = writeln!(
            manifest,
            "theme={}\tsource={}\tsource_path={package_path}\trevision={}\tmime={}\tdigest={}\tbytes={}\tpath={path}",
            package.theme().token(),
            package.source_digest(),
            package.revision_digest(),
            stylesheet.mime(),
            package.stylesheet_digest(),
            stylesheet.bytes().len(),
        );
    }
    write_staged(root.join("inventory.txt"), manifest.as_bytes())
}

fn write_staged(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), SystemArtifactStageError> {
    let path = path.as_ref();
    fs::write(path, bytes).map_err(|source| SystemArtifactStageError {
        path: path.to_path_buf(),
        source,
    })
}

/// Compiles the system artifact inventory from the release's closed inputs.
///
/// The result is deterministic: Theme order is stable, the portable package
/// archive is exported canonically, and each bundled stylesheet passes the same
/// validation and compiler boundary as a custom package before its revision is
/// exposed.
///
/// # Errors
///
/// Returns an error when a shipped bundled package violates normal Theme Package
/// validation or CSS compilation.
pub fn compile_system_artifact_inventory() -> Result<SystemArtifactInventory, SystemThemeError> {
    compile_inventory(APPLICATION_CSS, [TERMINAL_CSS, STUDIO_CSS, READER_CSS])
}

fn compile_inventory(
    application_css: &[u8],
    stylesheets: [&[u8]; 3],
) -> Result<SystemArtifactInventory, SystemThemeError> {
    let application = ApplicationStylesheet {
        bytes: application_css.to_vec(),
    };
    let themes = [
        compile_bundled_theme(Theme::Terminal, TERMINAL_MANIFEST, stylesheets[0])?,
        compile_bundled_theme(Theme::Studio, STUDIO_MANIFEST, stylesheets[1])?,
        compile_bundled_theme(Theme::Reader, READER_MANIFEST, stylesheets[2])?,
    ];
    Ok(SystemArtifactInventory {
        application,
        themes,
    })
}

/// Qualification-only A/B system artifact inputs.
///
/// This module exists only in host tests and explicit internal qualification
/// builds. Normal releases cannot name a fixture or select alternate styling.
#[cfg(any(test, feature = "qualification"))]
pub mod qualification {
    use super::*;

    /// Closed fixture selection for the later deployment-transition proof.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Fixture {
        A,
        BApplication,
        BTheme,
    }

    impl Fixture {
        /// Parses only the three internal qualification fixture tokens.
        ///
        /// # Errors
        ///
        /// Returns an error for every token outside the closed A/B fixture set.
        pub fn parse(value: &str) -> Result<Self, String> {
            match value {
                "a" => Ok(Self::A),
                "b-app" => Ok(Self::BApplication),
                "b-theme" => Ok(Self::BTheme),
                _ => Err(format!(
                    "unknown system-artifact qualification fixture `{value}`"
                )),
            }
        }

        #[must_use]
        pub const fn token(self) -> &'static str {
            match self {
                Self::A => "a",
                Self::BApplication => "b-app",
                Self::BTheme => "b-theme",
            }
        }
    }

    /// Produces a fixture inventory through the same closed release producer.
    ///
    /// # Errors
    ///
    /// Returns the normal package-validation/compiler failure when a fixture
    /// source cannot pass the ordinary bundled Theme Package boundary.
    pub fn compile(fixture: Fixture) -> Result<SystemArtifactInventory, SystemThemeError> {
        const APPLICATION_DECLARATION: &[u8] = b"\n.j-topbar { border-bottom-color: #4f46e5; }\n";
        const STUDIO_DECLARATION: &[u8] = b"\nbody { --accent: #0f766e; }\n";

        match fixture {
            Fixture::A => compile_system_artifact_inventory(),
            Fixture::BApplication => {
                let mut application = APPLICATION_CSS.to_vec();
                application.extend_from_slice(APPLICATION_DECLARATION);
                compile_inventory(&application, [TERMINAL_CSS, STUDIO_CSS, READER_CSS])
            }
            Fixture::BTheme => {
                let mut studio = STUDIO_CSS.to_vec();
                studio.extend_from_slice(STUDIO_DECLARATION);
                compile_inventory(APPLICATION_CSS, [TERMINAL_CSS, &studio, READER_CSS])
            }
        }
    }
}

fn compile_bundled_theme(
    theme: Theme,
    manifest: &[u8],
    stylesheet: &[u8],
) -> Result<BundledThemePackage, SystemThemeError> {
    compile_bundled_theme_with_assets(theme, manifest, stylesheet, &BTreeMap::new())
}

/// Runs one bundled package through the ordinary archive, validation, canonical
/// source export, and compiler boundary. Asset URLs come only from the
/// validator-minted asset digests, so package CSS cannot select an identity.
fn compile_bundled_theme_with_assets(
    theme: Theme,
    manifest: &[u8],
    stylesheet: &[u8],
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<BundledThemePackage, SystemThemeError> {
    let archive = export_theme_package(manifest, stylesheet, assets)
        .map_err(|source| SystemThemeError { theme, source })?;
    let validated = validate_theme_package(&archive, ThemePackageLimits::default())
        .map_err(|source| SystemThemeError { theme, source })?;
    let source_digest = validated.source_digest();
    let asset_urls = validated
        .asset_digests()
        .map(|(path, digest)| {
            (
                path.to_owned(),
                ThemeAssetDigest::from_digest(digest)
                    .content_url()
                    .to_string(),
            )
        })
        .collect();
    let package_bytes = validated
        .export_archive()
        .map_err(|source| SystemThemeError { theme, source })?;
    let revision = validated
        .compile(&asset_urls, ThemePackageLimits::default())
        .map_err(|source| SystemThemeError { theme, source })?;
    Ok(BundledThemePackage {
        theme,
        source_digest,
        package_bytes,
        revision,
    })
}

/// Closed test-only inventories that share one validated package asset across
/// selected bundled roles. They never alter the trusted application role or
/// ordinary/qualification release inputs.
#[cfg(any(test, feature = "test-utils"))]
pub mod shared_asset_fixture {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SharingFixture {
        Both,
        One,
        Neither,
    }

    const ASSET_A: &str = "assets/shared-a.png";
    const ASSET_B: &str = "assets/shared-b.png";
    const ASSET_MANIFEST_TERMINAL: &[u8] = br#"{"schema":1,"name":"Terminal","style_contract":1,"assets":{"assets/shared-a.png":"image/png","assets/shared-b.png":"image/png"},"defaults":{"logo":"assets/shared-a.png","header":["assets/shared-a.png","assets/shared-b.png"]}}"#;
    const ASSET_MANIFEST_STUDIO: &[u8] = br#"{"schema":1,"name":"Studio","style_contract":1,"assets":{"assets/shared-a.png":"image/png","assets/shared-b.png":"image/png"},"defaults":{"logo":"assets/shared-a.png","header":["assets/shared-a.png","assets/shared-b.png"]}}"#;
    const ASSET_CSS: &[u8] = b"\nbody { background-image: url(\"assets/shared-a.png\"); }\n";

    /// Compiles a closed asset-sharing inventory from validated PNG package
    /// input supplied by a test fixture; it cannot construct application CSS.
    ///
    /// # Errors
    ///
    /// Returns ordinary package validation/compiler errors for invalid input.
    pub fn compile(
        fixture: SharingFixture,
        png: &[u8],
    ) -> Result<SystemArtifactInventory, SystemThemeError> {
        let mut inventory = compile_system_artifact_inventory()?;
        let assets = BTreeMap::from([
            (ASSET_A.to_owned(), png.to_vec()),
            (ASSET_B.to_owned(), png.to_vec()),
        ]);
        if matches!(fixture, SharingFixture::Both | SharingFixture::One) {
            let mut stylesheet = TERMINAL_CSS.to_vec();
            stylesheet.extend_from_slice(ASSET_CSS);
            inventory.themes[0] = compile_bundled_theme_with_assets(
                Theme::Terminal,
                ASSET_MANIFEST_TERMINAL,
                &stylesheet,
                &assets,
            )?;
        }
        if matches!(fixture, SharingFixture::Both) {
            let mut stylesheet = STUDIO_CSS.to_vec();
            stylesheet.extend_from_slice(ASSET_CSS);
            inventory.themes[1] = compile_bundled_theme_with_assets(
                Theme::Studio,
                ASSET_MANIFEST_STUDIO,
                &stylesheet,
                &assets,
            )?; // cov:ignore: Studio error continuation needs invalid closed fixture constants after canonical inputs and the identical PNG have validated in Terminal.
        }
        Ok(inventory)
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn bundled_themes_use_the_custom_package_compiler_and_have_stable_inventory_order() {
        let inventory = compile_system_artifact_inventory().expect("shipped packages compile");
        assert_eq!(
            inventory
                .themes()
                .map(BundledThemePackage::theme)
                .collect::<Vec<_>>(),
            [Theme::Terminal, Theme::Studio, Theme::Reader],
        );
        for package in inventory.themes() {
            let stylesheet = package.stylesheet_content();
            assert_eq!(stylesheet.mime(), "text/css; charset=utf-8");
            assert!(!stylesheet.bytes().is_empty());
            assert_eq!(
                ThemeStylesheetDigest::from_digest(stylesheet.digest()),
                package.stylesheet_digest(),
            );
            assert_ne!(package.revision_digest().as_ref(), "");
        }
    }

    #[test]
    fn application_role_is_closed_and_content_addressed() {
        let inventory = compile_system_artifact_inventory().expect("shipped packages compile");
        let application = inventory.application();
        let content = application.content();
        assert_eq!(content.mime(), "text/css; charset=utf-8");
        assert_eq!(
            application.content_digest(),
            ThemeContentDigest::from_digest(Sha256::digest(content.bytes()).into()),
        );
    }

    #[test]
    fn staged_inventory_couples_role_to_exact_compiled_content() {
        let inventory = compile_system_artifact_inventory().expect("shipped packages compile");
        let root = tempfile::tempdir().expect("temporary staging root");
        stage_system_artifact_inventory(&inventory, root.path()).expect("stage inventory");

        assert_eq!(
            fs::read(root.path().join("application.css")).expect("read application"),
            inventory.application().content().bytes(),
        );
        for package in inventory.themes() {
            assert_eq!(
                fs::read(
                    root.path()
                        .join("themes")
                        .join(format!("{}.css", package.theme().token())),
                )
                .expect("read bundled stylesheet"),
                package.stylesheet_content().bytes(),
            );
            assert_eq!(
                fs::read(
                    root.path()
                        .join("packages")
                        .join(format!("{}.zip", package.theme().token())),
                )
                .expect("read bundled package"),
                package.package_bytes(),
            );
        }
        let manifest = fs::read_to_string(root.path().join("inventory.txt")).expect("manifest");
        assert!(manifest.contains(&format!(
            "digest={}",
            inventory.theme(Theme::Studio).stylesheet_digest()
        )));
        assert!(manifest.contains(&format!(
            "source={}",
            inventory.theme(Theme::Studio).source_digest()
        )));
        assert!(manifest.contains(&format!(
            "revision={}",
            inventory.theme(Theme::Studio).revision_digest()
        )));
    }

    fn replay(package: &BundledThemePackage) {
        let validated =
            validate_theme_package(package.package_bytes(), ThemePackageLimits::default())
                .expect("staged source validates");
        assert_eq!(
            ThemeSourceDigest::from_digest(validated.source_digest()),
            package.source_digest(),
        );
        let replay = validated
            .compile(&BTreeMap::new(), ThemePackageLimits::default())
            .expect("staged source compiles");
        assert_eq!(
            ThemeRevisionDigest::from_digest(replay.revision_digest()),
            package.revision_digest(),
        );
        assert_eq!(
            replay.stylesheet_content().digest(),
            package.stylesheet_content().digest(),
        );
        assert_eq!(
            replay.stylesheet_content().bytes(),
            package.stylesheet_content().bytes(),
        );
    }

    #[test]
    fn staged_inventory_replays_exact_compiler_identity() {
        let inventory = compile_system_artifact_inventory().expect("shipped packages compile");
        let root = tempfile::tempdir().expect("temporary staging root");
        stage_system_artifact_inventory(&inventory, root.path()).expect("stage inventory");
        let loaded =
            load_system_artifact_inventory(&DirectorySystemArtifactSource::new(root.path()))
                .expect("staged inventory replays");
        assert_eq!(
            loaded.application().content_digest(),
            inventory.application().content_digest()
        );
        for theme in [Theme::Terminal, Theme::Studio, Theme::Reader] {
            assert_eq!(
                loaded.theme(theme).source_digest(),
                inventory.theme(theme).source_digest()
            );
            assert_eq!(
                loaded.theme(theme).revision_digest(),
                inventory.theme(theme).revision_digest()
            );
            assert_eq!(
                loaded.theme(theme).stylesheet_digest(),
                inventory.theme(theme).stylesheet_digest()
            );
            assert_eq!(
                loaded.theme(theme).package_bytes(),
                inventory.theme(theme).package_bytes()
            );
        }
    }

    fn assert_staged_inventory_replays(inventory: &SystemArtifactInventory) {
        let root = tempfile::tempdir().expect("temporary staging root");
        stage_system_artifact_inventory(inventory, root.path()).expect("stage inventory");
        let loaded =
            load_system_artifact_inventory(&DirectorySystemArtifactSource::new(root.path()))
                .expect("staged inventory replays");
        assert_eq!(
            loaded.application().content().mime(),
            inventory.application().content().mime()
        );
        assert_eq!(
            loaded.application().content().bytes(),
            inventory.application().content().bytes()
        );
        assert_eq!(
            loaded.application().content_digest(),
            inventory.application().content_digest()
        );
        for theme in [Theme::Terminal, Theme::Studio, Theme::Reader] {
            let loaded = loaded.theme(theme);
            let expected = inventory.theme(theme);
            assert_eq!(loaded.package_bytes(), expected.package_bytes());
            assert_eq!(loaded.source_digest(), expected.source_digest());
            assert_eq!(loaded.revision_digest(), expected.revision_digest());
            assert_eq!(loaded.stylesheet_digest(), expected.stylesheet_digest());
            assert_eq!(
                loaded.stylesheet_content().bytes(),
                expected.stylesheet_content().bytes()
            );
            assert_eq!(
                loaded.stylesheet_content().mime(),
                expected.stylesheet_content().mime()
            );
            assert_eq!(
                loaded.revision().assets().collect::<Vec<_>>(),
                expected.revision().assets().collect::<Vec<_>>(),
                "loaded package assets preserve path, MIME, digest, and exact bytes"
            );
        }
    }

    #[test]
    fn shared_asset_fixture_rejects_invalid_png_input() {
        assert!(
            shared_asset_fixture::compile(shared_asset_fixture::SharingFixture::Both, b"not a PNG")
                .is_err()
        );
    }

    #[test]
    fn qualification_fixture_tokens_round_trip_and_reject_unknown() {
        for (token, expected) in [
            ("a", qualification::Fixture::A),
            ("b-app", qualification::Fixture::BApplication),
            ("b-theme", qualification::Fixture::BTheme),
        ] {
            let actual = qualification::Fixture::parse(token).expect("known fixture parses");
            assert_eq!(actual, expected);
            assert_eq!(actual.token(), token);
        }
        assert!(qualification::Fixture::parse("unknown").is_err());
    }

    #[test]
    fn staged_loader_replays_a_b_application_b_theme_and_shared_assets() {
        let a = qualification::compile(qualification::Fixture::A).expect("A compiles");
        let b_application = qualification::compile(qualification::Fixture::BApplication)
            .expect("B application compiles");
        let b_theme =
            qualification::compile(qualification::Fixture::BTheme).expect("B theme compiles");
        let png = include_bytes!("../../testdata/theme-repository/minimal/preview.png");
        let shared = shared_asset_fixture::compile(shared_asset_fixture::SharingFixture::Both, png)
            .expect("asset-bearing fixture compiles");
        for inventory in [&a, &b_application, &b_theme, &shared] {
            assert_staged_inventory_replays(inventory);
        }
        assert!(
            shared
                .theme(Theme::Terminal)
                .revision()
                .assets()
                .next()
                .is_some()
        );
    }

    #[test]
    fn staged_inventory_rejects_extra_and_corrupt_artifacts() {
        let inventory = compile_system_artifact_inventory().expect("shipped packages compile");
        let root = tempfile::tempdir().expect("temporary staging root");
        stage_system_artifact_inventory(&inventory, root.path()).expect("stage inventory");
        fs::write(root.path().join("unexpected"), b"extra").expect("write extra artifact");
        assert!(
            load_system_artifact_inventory(&DirectorySystemArtifactSource::new(root.path()))
                .is_err()
        );
        fs::remove_file(root.path().join("unexpected")).expect("remove extra artifact");
        fs::write(root.path().join("themes/studio.css"), b"corrupt").expect("corrupt stylesheet");
        assert!(
            load_system_artifact_inventory(&DirectorySystemArtifactSource::new(root.path()))
                .is_err()
        );
    }

    #[test]
    fn staging_failures_preserve_existing_bytes_and_report_the_destination() {
        let inventory = compile_system_artifact_inventory().expect("closed inputs compile");
        let root = tempfile::tempdir().expect("temporary root");
        let file = root.path().join("file");
        fs::write(&file, b"existing bytes").expect("write obstruction");
        let error = stage_system_artifact_inventory(&inventory, &file)
            .expect_err("cannot replace file as directory");
        assert_eq!(error.path, file);
        assert_eq!(
            fs::read(&file).expect("original file survives"),
            b"existing bytes"
        );
        let nested = file.join("nested");
        let error = stage_system_artifact_inventory(&inventory, &nested)
            .expect_err("cannot create beneath a file");
        assert_eq!(error.path, nested);
        let error = write_staged(root.path(), b"cannot overwrite directory")
            .expect_err("writing directory fails");
        assert_eq!(error.path, root.path());
    }

    #[test]
    fn directory_source_reports_missing_root_and_missing_member() {
        let root = tempfile::tempdir().expect("temporary root");
        let source = DirectorySystemArtifactSource::new(root.path());
        assert!(
            matches!(source.read("missing"), Err(SystemArtifactLoadError::Read { path, .. }) if path == "missing")
        );
        let missing = root.path().join("missing");
        assert!(matches!(
            DirectorySystemArtifactSource::new(&missing).paths(),
            Err(SystemArtifactLoadError::Read { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn directory_source_rejects_noncanonical_non_utf8_and_symlink_members() {
        use std::os::unix::{ffi::OsStringExt, fs::symlink};
        for name in [
            std::ffi::OsString::from("bad\\name"),
            std::ffi::OsString::from_vec(vec![0xff]),
        ] {
            let root = tempfile::tempdir().expect("temporary root");
            fs::write(root.path().join(name), b"invalid member").expect("write member");
            assert!(matches!(
                DirectorySystemArtifactSource::new(root.path()).paths(),
                Err(SystemArtifactLoadError::Invalid(_))
            ));
        }
        let root = tempfile::tempdir().expect("temporary root");
        symlink("missing", root.path().join("link")).expect("write symlink");
        assert!(matches!(
            DirectorySystemArtifactSource::new(root.path()).paths(),
            Err(SystemArtifactLoadError::Invalid(_))
        ));
    }

    #[test]
    fn staged_loader_rejects_closed_inventory_mutation_matrix() {
        for (name, mutate) in inventory_mutation_cases() {
            assert_inventory_mutation_rejected(name, mutate.as_ref());
        }
    }

    type InventoryMutation = (&'static str, Box<dyn Fn(&Path)>);

    fn inventory_mutation_cases() -> Vec<InventoryMutation> {
        let mut cases: Vec<InventoryMutation> = vec![
            (
                "missing file",
                Box::new(|root| {
                    fs::remove_file(root.join("application.css")).expect("remove application");
                }),
            ),
            (
                "empty inventory",
                Box::new(|root| {
                    fs::write(root.join("inventory.txt"), b"").expect("empty inventory");
                }),
            ),
            (
                "malformed role",
                Box::new(|root| {
                    fs::write(root.join("inventory.txt"), b"malformed\n")
                        .expect("write malformed inventory");
                }),
            ),
            (
                "duplicate role",
                Box::new(|root| {
                    let mut text =
                        fs::read_to_string(root.join("inventory.txt")).expect("read inventory");
                    let application = text.lines().next().expect("application role").to_owned();
                    text.push_str(&application);
                    text.push('\n');
                    fs::write(root.join("inventory.txt"), text).expect("duplicate role");
                }),
            ),
            (
                "extra role",
                Box::new(|root| {
                    let mut text =
                        fs::read_to_string(root.join("inventory.txt")).expect("read inventory");
                    text.push_str("theme=extra\n");
                    fs::write(root.join("inventory.txt"), text).expect("extra role");
                }),
            ),
            (
                "reordered roles",
                Box::new(|root| {
                    let text =
                        fs::read_to_string(root.join("inventory.txt")).expect("read inventory");
                    let mut lines = text.lines().collect::<Vec<_>>();
                    lines.swap(1, 2);
                    fs::write(
                        root.join("inventory.txt"),
                        format!("{}\n", lines.join("\n")),
                    )
                    .expect("reorder roles");
                }),
            ),
            ("traversal metadata", Box::new(traversal_metadata)),
            (
                "corrupt source ZIP",
                Box::new(|root| {
                    fs::write(root.join("packages/studio.zip"), b"not a ZIP")
                        .expect("corrupt source zip");
                }),
            ),
            (
                "noncanonical ZIP transport",
                Box::new(|root| {
                    use std::io::Write as _;
                    fs::OpenOptions::new()
                        .append(true)
                        .open(root.join("packages/reader.zip"))
                        .expect("open package")
                        .write_all(b"trailer")
                        .expect("append transport bytes");
                }),
            ),
        ];
        // Change one field per case so another rejected field cannot hide a gap.
        for (name, role, field) in [
            ("application MIME", "application\t", "mime"),
            ("application digest", "application\t", "digest"),
            ("application length", "application\t", "bytes"),
            ("application path", "application\t", "path"),
            ("theme source", "theme=terminal\t", "source"),
            ("theme source path", "theme=terminal\t", "source_path"),
            ("theme revision", "theme=terminal\t", "revision"),
            ("theme MIME", "theme=terminal\t", "mime"),
            ("theme digest", "theme=terminal\t", "digest"),
            ("theme length", "theme=terminal\t", "bytes"),
            ("theme path", "theme=terminal\t", "path"),
        ] {
            cases.push((
                name,
                Box::new(move |root| wrong_role_metadata(root, role, field)),
            ));
        }
        cases
    }

    fn wrong_role_metadata(root: &Path, role: &str, field: &str) {
        let text = fs::read_to_string(root.join("inventory.txt")).expect("read inventory");
        let text = text
            .lines()
            .map(|line| {
                if line.starts_with(role) {
                    line.split('\t')
                        .map(|value| {
                            if value.starts_with(&format!("{field}=")) {
                                format!("{field}=invalid")
                            } else {
                                value.to_owned()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\t")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.join("inventory.txt"), format!("{text}\n")).expect("wrong role metadata");
    }

    fn traversal_metadata(root: &Path) {
        let text = fs::read_to_string(root.join("inventory.txt"))
            .expect("read inventory")
            .replacen("path=themes/terminal.css", "path=themes/../terminal.css", 1);
        fs::write(root.join("inventory.txt"), text).expect("traversal metadata");
    }

    fn assert_inventory_mutation_rejected(name: &str, mutate: &dyn Fn(&Path)) {
        let inventory = compile_system_artifact_inventory().expect("shipped packages compile");
        let root = tempfile::tempdir().expect("temporary staging root");
        stage_system_artifact_inventory(&inventory, root.path()).expect("stage inventory");
        mutate(root.path());
        let error =
            load_system_artifact_inventory(&DirectorySystemArtifactSource::new(root.path()))
                .expect_err(name);
        assert!(
            !error.to_string().is_empty(),
            "{name} reports a loader error"
        );
    }

    #[test]
    fn staged_portable_package_source_replays_compiler_minted_identity_for_a_and_b_theme() {
        let a = qualification::compile(qualification::Fixture::A).expect("A compiles");
        let b_theme =
            qualification::compile(qualification::Fixture::BTheme).expect("B-theme compiles");
        replay(a.theme(Theme::Studio));
        replay(b_theme.theme(Theme::Studio));
        assert_ne!(
            a.theme(Theme::Studio).package_bytes(),
            b_theme.theme(Theme::Studio).package_bytes(),
        );
        assert_ne!(
            a.theme(Theme::Studio).source_digest(),
            b_theme.theme(Theme::Studio).source_digest(),
        );
        assert_ne!(
            a.theme(Theme::Studio).revision_digest(),
            b_theme.theme(Theme::Studio).revision_digest(),
        );
    }

    #[test]
    fn qualification_variants_change_only_the_declared_artifact_address() {
        let a = qualification::compile(qualification::Fixture::A).expect("A compiles");
        let b_app =
            qualification::compile(qualification::Fixture::BApplication).expect("B-app compiles");
        let b_theme =
            qualification::compile(qualification::Fixture::BTheme).expect("B-theme compiles");

        assert_ne!(
            a.application().content_digest(),
            b_app.application().content_digest()
        );
        for theme in [Theme::Terminal, Theme::Studio, Theme::Reader] {
            assert_eq!(
                a.theme(theme).stylesheet_digest(),
                b_app.theme(theme).stylesheet_digest(),
            );
        }
        assert_eq!(
            a.application().content_digest(),
            b_theme.application().content_digest()
        );
        assert_eq!(
            a.theme(Theme::Terminal).stylesheet_digest(),
            b_theme.theme(Theme::Terminal).stylesheet_digest(),
        );
        assert_ne!(
            a.theme(Theme::Studio).stylesheet_digest(),
            b_theme.theme(Theme::Studio).stylesheet_digest(),
        );
        assert_eq!(
            a.theme(Theme::Reader).stylesheet_digest(),
            b_theme.theme(Theme::Reader).stylesheet_digest(),
        );
    }

    #[test]
    fn empty_asset_producer_input_preserves_shipped_package_identity() {
        let inventory = compile_system_artifact_inventory().expect("shipped packages compile");
        let direct = compile_bundled_theme_with_assets(
            Theme::Terminal,
            TERMINAL_MANIFEST,
            TERMINAL_CSS,
            &BTreeMap::new(),
        )
        .expect("empty asset producer input compiles");
        assert_eq!(
            direct.package_bytes(),
            inventory.theme(Theme::Terminal).package_bytes()
        );
        assert_eq!(
            direct.source_digest(),
            inventory.theme(Theme::Terminal).source_digest()
        );
        assert_eq!(
            direct.revision_digest(),
            inventory.theme(Theme::Terminal).revision_digest()
        );
    }

    #[test]
    fn normal_inventory_is_the_unaltered_qualification_fixture() {
        let normal = compile_system_artifact_inventory().expect("normal inventory compiles");
        let fixture =
            qualification::compile(qualification::Fixture::A).expect("A fixture compiles");
        assert_eq!(
            normal.application().content_digest(),
            fixture.application().content_digest()
        );
        for theme in [Theme::Terminal, Theme::Studio, Theme::Reader] {
            assert_eq!(
                normal.theme(theme).revision_digest(),
                fixture.theme(theme).revision_digest(),
            );
        }
    }
}
