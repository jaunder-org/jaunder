//! Deterministic system styling artifacts produced from closed release inputs.
//!
//! Bundled public Themes deliberately take the same portable archive, validation,
//! and CSS-compilation path as owner-authored Theme Packages. Application CSS is
//! not a Theme Package: it styles protected/private application structure and is
//! consequently admitted only through the private, compiled-in application role
//! below. Both roles expose exact content bytes and digests without inventing a
//! second package compiler or hashing scheme.

use std::{collections::BTreeMap, fmt::Write as _, fs, path::Path};

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
            .unwrap_or_else(|| unreachable!("the closed Theme enum has a bundled package"))
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
    fs::create_dir_all(root.join("packages")).map_err(|source| SystemArtifactStageError {
        path: root.to_path_buf(),
        source,
    })?;

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
    const ASSET_MANIFEST_TERMINAL: &[u8] = br#"{"schema":1,"name":"Terminal","style_contract":1,"assets":{"assets/shared-a.png":"image/png","assets/shared-b.png":"image/png"},"defaults":{"header":["assets/shared-a.png","assets/shared-b.png"]}}"#;
    const ASSET_MANIFEST_STUDIO: &[u8] = br#"{"schema":1,"name":"Studio","style_contract":1,"assets":{"assets/shared-a.png":"image/png","assets/shared-b.png":"image/png"},"defaults":{"header":["assets/shared-a.png","assets/shared-b.png"]}}"#;
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
            )?;
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
