//! Canonical host-side view of a staged qualification system-artifact inventory.

use std::{collections::BTreeMap, path::Path};

use anyhow::Result;
use host::system_theme::{
    DirectorySystemArtifactSource, SystemArtifactInventory, load_system_artifact_inventory,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InventoryContent {
    pub digest: String,
    pub url: String,
    pub mime: String,
    pub bytes: u64,
    pub sha256: String,
    pub etag: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ThemeInventory {
    pub source_digest: String,
    pub revision_digest: String,
    pub stylesheet: InventoryContent,
    pub assets: BTreeMap<String, InventoryContent>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct QualificationInventory {
    pub application: InventoryContent,
    pub themes: BTreeMap<String, ThemeInventory>,
}

pub(crate) fn load_qualification_inventory(root: &Path) -> Result<QualificationInventory> {
    let inventory = load_system_artifact_inventory(&DirectorySystemArtifactSource::new(root))?;
    Ok(map_inventory(&inventory))
}
fn content(content: host::theme_package::CompiledThemeContent<'_>) -> InventoryContent {
    content_fields(content.digest(), content.mime(), content.bytes())
}

fn content_fields(digest: [u8; 32], mime: &str, bytes: &[u8]) -> InventoryContent {
    let etag = host::etag::from_sha256(digest).to_string();
    let digest = crate::digest::lowercase_hex(digest);
    let sha256 = crate::digest::lowercase_hex(Sha256::digest(bytes));
    InventoryContent {
        url: format!("/theme/{digest}"),
        etag,
        digest,
        mime: mime.into(),
        bytes: bytes.len() as u64,
        sha256,
    }
}
pub(crate) fn map_inventory(inventory: &SystemArtifactInventory) -> QualificationInventory {
    let application = content(inventory.application().content());
    let themes = inventory
        .themes()
        .map(|theme| {
            let assets = theme
                .revision()
                .assets()
                .map(|(path, mime, bytes, digest)| {
                    (path.to_owned(), content_fields(digest, mime, bytes))
                })
                .collect();
            (
                theme.theme().token().to_owned(),
                ThemeInventory {
                    source_digest: theme.source_digest().to_string(),
                    revision_digest: theme.revision_digest().to_string(),
                    stylesheet: content(theme.stylesheet_content()),
                    assets,
                },
            )
        })
        .collect();
    QualificationInventory {
        application,
        themes,
    }
}
pub(crate) fn compare_qualification_inventories(
    a: &QualificationInventory,
    b: &QualificationInventory,
    application_changed: bool,
    studio_changed: bool,
) -> Result<()> {
    if (a.application == b.application) != !application_changed {
        anyhow::bail!("application inventory change disagrees with fixture")
    };
    let names = ["terminal", "studio", "reader"];
    if application_changed && studio_changed
        || a.themes.len() != names.len()
        || b.themes.len() != names.len()
    {
        anyhow::bail!("qualification inventory has an invalid fixture or theme set");
    }
    for name in names {
        let original = a
            .themes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("baseline inventory omits {name}"))?;
        let candidate = b
            .themes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("candidate inventory omits {name}"))?;
        if name == "studio" && studio_changed {
            if original.source_digest == candidate.source_digest
                || original.revision_digest == candidate.revision_digest
                || original.stylesheet == candidate.stylesheet
                || original.assets != candidate.assets
            {
                anyhow::bail!("Studio-only fixture must change source/revision/CSS, not assets");
            }
        } else if original != candidate {
            anyhow::bail!("theme inventory change disagrees with fixture for {name}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use host::system_theme::{
        qualification::{Fixture, compile},
        stage_system_artifact_inventory,
    };
    use tempfile::tempdir;
    fn staged(f: Fixture) -> QualificationInventory {
        let temp = tempdir().unwrap();
        stage_system_artifact_inventory(&compile(f).unwrap(), temp.path()).unwrap();
        load_qualification_inventory(temp.path()).unwrap()
    }
    #[test]
    fn maps_canonical_mime_etag_and_fixture_delta() {
        let a = staged(Fixture::A);
        let app = staged(Fixture::BApplication);
        let theme = staged(Fixture::BTheme);
        assert_eq!(a.application.mime, "text/css; charset=utf-8");
        assert!(a.application.etag.starts_with("\"sha256-"));
        compare_qualification_inventories(&a, &app, true, false).unwrap();
        compare_qualification_inventories(&a, &theme, false, true).unwrap();
        assert!(compare_qualification_inventories(&a, &app, false, false).is_err());
        let mut unexpected_asset = theme.clone();
        unexpected_asset
            .themes
            .get_mut("studio")
            .unwrap()
            .assets
            .insert("unexpected.png".into(), a.application.clone());
        assert!(compare_qualification_inventories(&a, &unexpected_asset, false, true).is_err());
        let mut unchanged_revision = theme.clone();
        unchanged_revision
            .themes
            .get_mut("studio")
            .unwrap()
            .revision_digest = a.themes["studio"].revision_digest.clone();
        assert!(compare_qualification_inventories(&a, &unchanged_revision, false, true).is_err());
        let mut missing_theme = a.clone();
        missing_theme.themes.remove("reader");
        assert!(
            compare_qualification_inventories(&missing_theme, &missing_theme, false, false)
                .is_err()
        );
    }
}
