//! Viewer-independent public theme resolution.

use common::{
    ids::{ThemeId, UserId},
    media::{self, ContentHash, Filename, MediaSource},
    root_relative_url::RootRelativeUrl,
    theme::{
        PublicThemeRoute, PublicThemeSelection, PublishedThemeIdentity, PublishedThemePresentation,
        Theme, ThemeHeaderPool, ThemeImageRole, ThemePoolEntry, ThemeRevisionDigest,
    },
};
use serde::Deserialize;

use crate::{ThemeOwner, ThemePackageAsset, ThemeRoleBinding, ThemeStorage};

/// Ownership scope for one public presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicThemeOwner {
    /// Aggregate pages use the operator-selected site theme.
    Site,
    /// Author-owned pages may override the site theme.
    Author(UserId),
}

/// Resolve the effective presentation for one public route.
///
/// A missing, unpublished, or deleted site selection falls back to Studio. An
/// invalid author selection inherits that already-resolved site presentation.
/// Malformed persisted bindings and database read failures deliberately propagate
/// rather than being rendered as a false fallback.
///
/// # Errors
///
/// Returns an underlying database error when a selection, theme record, or binding
/// cannot be read.
pub async fn resolve_public_theme(
    owner: PublicThemeOwner,
    route: &PublicThemeRoute,
    themes: &dyn ThemeStorage,
) -> Result<PublishedThemePresentation, sqlx::Error> {
    let site = resolve_selection(ThemeOwner::Site, route, themes)
        .await?
        .unwrap_or_else(|| PublishedThemePresentation::built_in(Theme::Studio));
    match owner {
        PublicThemeOwner::Site => Ok(site),
        PublicThemeOwner::Author(user_id) => {
            Ok(
                resolve_selection(ThemeOwner::Author(user_id), route, themes)
                    .await?
                    .unwrap_or(site),
            )
        }
    }
}

async fn resolve_selection(
    owner: ThemeOwner,
    route: &PublicThemeRoute,
    themes: &dyn ThemeStorage,
) -> Result<Option<PublishedThemePresentation>, sqlx::Error> {
    match themes.selection(owner).await? {
        None => Ok(None),
        Some(PublicThemeSelection::BuiltIn(theme)) => {
            Ok(Some(PublishedThemePresentation::built_in(theme)))
        }
        Some(PublicThemeSelection::Custom(theme_id)) => {
            resolve_custom(owner, theme_id, route, themes).await
        }
    }
}
const PACKAGED_DEFAULT_HEADER_SEED: [u8; 32] = [0; 32];

async fn resolve_custom(
    owner: ThemeOwner,
    theme_id: ThemeId,
    route: &PublicThemeRoute,
    themes: &dyn ThemeStorage,
) -> Result<Option<PublishedThemePresentation>, sqlx::Error> {
    let Some(entry) = themes
        .list_themes(owner)
        .await?
        .into_iter()
        .find(|entry| entry.id == theme_id)
    else {
        return Ok(None);
    };
    let Some(current_digest) = entry.current_revision else {
        return Ok(None);
    };
    let Some(revision) = themes
        .list_revisions(owner, theme_id)
        .await?
        .into_iter()
        .find(|revision| revision.digest == current_digest)
    else {
        return Ok(None);
    };
    let assets = themes
        .revision_assets(owner, theme_id, &revision.digest)
        .await?;
    let logo = themes
        .role_binding(owner, theme_id, ThemeImageRole::Logo)
        .await?;
    let header = themes
        .role_binding(owner, theme_id, ThemeImageRole::Header)
        .await?;
    let defaults = manifest_defaults(&revision.manifest);
    let logo_default = defaults
        .as_ref()
        .and_then(|defaults| defaults.logo.as_ref());
    let header_defaults = defaults
        .as_ref()
        .and_then(|defaults| defaults.header.as_deref());
    let header_pool = if matches!(&header, Some(ThemeRoleBinding::HeaderPool { .. })) {
        Some(themes.header_pool(owner, theme_id).await?)
    } else {
        None
    };
    let stylesheet_url =
        RootRelativeUrl::try_from(format!("/theme/{}", revision.stylesheet_digest))
            .map_err(|_| sqlx::Error::RowNotFound)?;
    let logo_url = resolve_role(
        logo.as_ref(),
        &assets,
        &revision.digest,
        route,
        None,
        logo_default.map(std::slice::from_ref),
    );
    let header_url = resolve_role(
        header.as_ref(),
        &assets,
        &revision.digest,
        route,
        header_pool.as_deref(),
        header_defaults,
    );
    if role_is_invalid(logo.as_ref(), logo_url.as_ref())
        || role_is_invalid(header.as_ref(), header_url.as_ref())
    {
        return Ok(None);
    }
    Ok(Some(PublishedThemePresentation {
        identity: PublishedThemeIdentity::Custom(theme_id),
        revision: Some(revision.digest.clone()),
        stylesheet_url,
        logo_url,
        header_url,
    }))
}
fn resolve_role_with_package_url(
    binding: Option<&ThemeRoleBinding>,
    package_url: &dyn Fn(&str) -> Option<RootRelativeUrl>,
    revision: &ThemeRevisionDigest,
    route: &PublicThemeRoute,
    pool_entries: Option<&[crate::ThemeHeaderPoolEntry]>,
    packaged_defaults: Option<&[String]>,
) -> Option<RootRelativeUrl> {
    match binding? {
        ThemeRoleBinding::PackagedDefault { role, .. } => match role {
            ThemeImageRole::Logo => packaged_defaults?
                .first()
                .and_then(|path| package_url(path)),
            ThemeImageRole::Header => {
                unreachable!("header defaults resolve before package URL lookup")
            }
        },
        ThemeRoleBinding::ExplicitAbsent { .. } => None,
        ThemeRoleBinding::PackageAsset { package_path, .. } => package_url(package_path),
        ThemeRoleBinding::Media { media, .. } => {
            Some(media::url(&media.source, &media.sha256, &media.filename))
        }
        ThemeRoleBinding::HeaderPool {
            pool_revision,
            shuffle_seed,
            ..
        } => {
            let entries = pool_entries?;
            let pool =
                ThemeHeaderPool::new(entries.iter().map(pool_entry).collect::<Option<Vec<_>>>()?)
                    .ok()?;
            if pool_revision != pool.revision() {
                return None;
            }
            resolve_pool_entry(pool.select(route, revision, shuffle_seed), package_url)
        }
    }
}

fn resolve_packaged_header_default(
    packaged_defaults: Option<&[String]>,
    package_url: &dyn Fn(&str) -> Option<RootRelativeUrl>,
    revision: &ThemeRevisionDigest,
    route: &PublicThemeRoute,
) -> Option<RootRelativeUrl> {
    let pool = ThemeHeaderPool::new(
        packaged_defaults?
            .iter()
            .cloned()
            .map(ThemePoolEntry::Package)
            .collect(),
    )
    .ok()?;
    resolve_pool_entry(
        pool.select(route, revision, &PACKAGED_DEFAULT_HEADER_SEED),
        package_url,
    )
}

fn resolve_pool_entry(
    entry: &ThemePoolEntry,
    package_url: &dyn Fn(&str) -> Option<RootRelativeUrl>,
) -> Option<RootRelativeUrl> {
    match entry {
        ThemePoolEntry::Package(path) => package_url(path),
        ThemePoolEntry::Media {
            source,
            digest,
            filename,
        } => media_url_parts(source, digest.as_ref(), filename),
    }
}

fn resolve_role(
    binding: Option<&ThemeRoleBinding>,
    assets: &[ThemePackageAsset],
    revision: &ThemeRevisionDigest,
    route: &PublicThemeRoute,
    pool_entries: Option<&[crate::ThemeHeaderPoolEntry]>,
    packaged_defaults: Option<&[String]>,
) -> Option<RootRelativeUrl> {
    let package_url = |path: &str| package_url(assets, path);
    match binding {
        Some(ThemeRoleBinding::PackagedDefault {
            role: ThemeImageRole::Header,
            ..
        }) => resolve_packaged_header_default(packaged_defaults, &package_url, revision, route),
        _ => resolve_role_with_package_url(
            binding,
            &package_url,
            revision,
            route,
            pool_entries,
            packaged_defaults,
        ),
    }
}

fn role_is_invalid(binding: Option<&ThemeRoleBinding>, url: Option<&RootRelativeUrl>) -> bool {
    binding.is_some_and(|binding| !binding.is_explicit_absent() && url.is_none())
}

/// Resolves the effective logo and header for an owned draft preview.
///
/// The same role and pool policy as public presentations applies, but callers
/// supply owner-authenticated draft package URLs instead of immutable revision URLs.
///
/// # Errors
///
/// Returns a storage error for missing or corrupt bindings and pool reads.
pub async fn resolve_draft_theme_images(
    owner: ThemeOwner,
    theme_id: ThemeId,
    manifest: &[u8],
    package_asset_urls: &std::collections::BTreeMap<String, String>,
    revision: &ThemeRevisionDigest,
    route: &PublicThemeRoute,
    themes: &dyn ThemeStorage,
) -> Result<(Option<RootRelativeUrl>, Option<RootRelativeUrl>), sqlx::Error> {
    let logo = themes
        .role_binding(owner, theme_id, ThemeImageRole::Logo)
        .await?;
    let header = themes
        .role_binding(owner, theme_id, ThemeImageRole::Header)
        .await?;
    let header_pool = if matches!(&header, Some(ThemeRoleBinding::HeaderPool { .. })) {
        Some(themes.header_pool(owner, theme_id).await?)
    } else {
        None
    };
    let defaults = manifest_defaults(manifest);
    let logo_default = defaults
        .as_ref()
        .and_then(|defaults| defaults.logo.as_ref());
    let header_defaults = defaults
        .as_ref()
        .and_then(|defaults| defaults.header.as_deref());
    let package_url = |path: &str| {
        package_asset_urls
            .get(path)
            .and_then(|url| url.parse::<RootRelativeUrl>().ok())
    };
    let logo_url = resolve_role_with_package_url(
        logo.as_ref(),
        &package_url,
        revision,
        route,
        None,
        logo_default.map(std::slice::from_ref),
    );
    let header_url = match header.as_ref() {
        Some(ThemeRoleBinding::PackagedDefault {
            role: ThemeImageRole::Header,
            ..
        }) => resolve_packaged_header_default(header_defaults, &package_url, revision, route),
        _ => resolve_role_with_package_url(
            header.as_ref(),
            &package_url,
            revision,
            route,
            header_pool.as_deref(),
            header_defaults,
        ),
    };
    if role_is_invalid(logo.as_ref(), logo_url.as_ref())
        || role_is_invalid(header.as_ref(), header_url.as_ref())
    {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok((logo_url, header_url))
}

fn package_url(assets: &[ThemePackageAsset], path: &str) -> Option<RootRelativeUrl> {
    let asset = assets.iter().find(|asset| asset.path == path)?;
    format!("/theme/{}", asset.digest).parse().ok()
}

fn media_url_parts(source: &str, digest: &str, filename: &str) -> Option<RootRelativeUrl> {
    let source: MediaSource = source.parse().ok()?;
    let digest: ContentHash = digest.parse().ok()?;
    let filename: Filename = filename.parse().ok()?;
    Some(media::url(&source, &digest, &filename))
}

fn pool_entry(entry: &crate::ThemeHeaderPoolEntry) -> Option<ThemePoolEntry> {
    match (
        entry.package_path.as_deref(),
        entry.media_user_id,
        entry.media_source.as_deref(),
        entry.media_digest.as_ref(),
        entry.media_filename.as_deref(),
    ) {
        (Some(path), None, None, None, None) => Some(ThemePoolEntry::Package(path.to_owned())),
        (None, Some(_), Some(source), Some(digest), Some(filename)) => {
            Some(ThemePoolEntry::Media {
                source: source.to_owned(),
                digest: digest.clone(),
                filename: filename.to_owned(),
            })
        }
        _ => None,
    }
}

#[derive(Deserialize)]
struct ManifestDefaultsDocument {
    defaults: ManifestDefaults,
}

#[derive(Deserialize)]
struct ManifestDefaults {
    logo: Option<String>,
    header: Option<Vec<String>>,
}

fn manifest_defaults(manifest: &[u8]) -> Option<ManifestDefaults> {
    serde_json::from_slice::<ManifestDefaultsDocument>(manifest)
        .ok()
        .map(|manifest| manifest.defaults)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ThemeCatalogEntry, ThemeRevision};
    use std::sync::Arc;

    use crate::test_support::{Backend, SeedUser, backends, confirmed};
    use rstest::*;
    use rstest_reuse::*;

    fn builtin(theme: Theme) -> PublishedThemePresentation {
        PublishedThemePresentation::built_in(theme)
    }

    // guard:no-backend — exercises resolver policy through the aggregate storage mock
    #[tokio::test]
    async fn site_owner_uses_site_selection() {
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_selection().return_once(|owner| {
            assert_eq!(owner, ThemeOwner::Site);
            Ok(Some(PublicThemeSelection::BuiltIn(Theme::Terminal)))
        });

        assert_eq!(
            resolve_public_theme(PublicThemeOwner::Site, &PublicThemeRoute::site(), &themes)
                .await
                .unwrap(),
            builtin(Theme::Terminal)
        );
    }

    // guard:no-backend — exercises resolver policy through the aggregate storage mock
    #[tokio::test]
    async fn author_without_override_inherits_site_selection() {
        let author = UserId::from(7);
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_selection().returning(move |owner| {
            Ok(match owner {
                ThemeOwner::Site => Some(PublicThemeSelection::BuiltIn(Theme::Terminal)),
                ThemeOwner::Author(user_id) if user_id == author => None,
                ThemeOwner::Author(_) => {
                    unreachable!("resolver requests only the specified author")
                }
            })
        });

        assert_eq!(
            resolve_public_theme(
                PublicThemeOwner::Author(author),
                &PublicThemeRoute::site(),
                &themes,
            )
            .await
            .unwrap(),
            builtin(Theme::Terminal)
        );
    }

    // guard:no-backend — exercises resolver policy through the aggregate storage mock
    #[tokio::test]
    async fn author_override_wins_over_site_selection() {
        let author = UserId::from(7);
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_selection().returning(move |owner| {
            Ok(match owner {
                ThemeOwner::Site => Some(PublicThemeSelection::BuiltIn(Theme::Terminal)),
                ThemeOwner::Author(user_id) if user_id == author => {
                    Some(PublicThemeSelection::BuiltIn(Theme::Reader))
                }
                ThemeOwner::Author(_) => {
                    unreachable!("resolver requests only the specified author")
                }
            })
        });

        assert_eq!(
            resolve_public_theme(
                PublicThemeOwner::Author(author),
                &PublicThemeRoute::site(),
                &themes,
            )
            .await
            .unwrap(),
            builtin(Theme::Reader)
        );
    }

    // guard:no-backend — exercises resolver policy through the aggregate storage mock
    #[tokio::test]
    async fn missing_site_selection_falls_back_to_studio() {
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_selection().return_once(|_| Ok(None));

        assert_eq!(
            resolve_public_theme(PublicThemeOwner::Site, &PublicThemeRoute::site(), &themes)
                .await
                .unwrap(),
            builtin(Theme::Studio)
        );
    }

    // guard:no-backend — exercises resolver policy through the aggregate storage mock
    #[tokio::test]
    async fn selection_read_failure_propagates() {
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_selection().return_once(|_| {
            Err(sqlx::Error::Io(std::io::Error::other(
                "injected theme selection read failure",
            )))
        });

        assert!(
            resolve_public_theme(PublicThemeOwner::Site, &PublicThemeRoute::site(), &themes)
                .await
                .is_err()
        );
    }

    fn custom_entry(
        owner: ThemeOwner,
        theme_id: ThemeId,
        current_revision: Option<&str>,
    ) -> ThemeCatalogEntry {
        ThemeCatalogEntry {
            id: theme_id,
            owner,
            name: "Custom".into(),
            current_revision: current_revision.map(|digest| digest.parse().unwrap()),
        }
    }

    fn custom_revision(theme_id: ThemeId, digest: &str, stylesheet_digest: &str) -> ThemeRevision {
        ThemeRevision {
            theme_id,
            digest: digest.parse().unwrap(),
            stylesheet_digest: stylesheet_digest.parse().unwrap(),
            manifest: Vec::new(),
        }
    }

    // guard:no-backend — exercises resolver policy through the aggregate storage mock
    #[tokio::test]
    async fn unpublished_custom_site_selection_falls_back_to_studio() {
        let theme_id = ThemeId::from(9);
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_selection()
            .return_once(move |_| Ok(Some(PublicThemeSelection::Custom(theme_id))));
        themes
            .expect_list_themes()
            .return_once(move |_| Ok(vec![custom_entry(ThemeOwner::Site, theme_id, None)]));

        assert_eq!(
            resolve_public_theme(PublicThemeOwner::Site, &PublicThemeRoute::site(), &themes)
                .await
                .unwrap(),
            builtin(Theme::Studio)
        );
    }

    // guard:no-backend — exercises resolver policy through the aggregate storage mock
    #[tokio::test]
    async fn deleted_or_stale_custom_selection_falls_back_to_studio() {
        let deleted_id = ThemeId::from(9);
        let stale_id = ThemeId::from(10);
        for (theme_id, entries) in [
            (deleted_id, Vec::new()),
            (
                stale_id,
                vec![custom_entry(
                    ThemeOwner::Site,
                    stale_id,
                    Some(&"a".repeat(64)),
                )],
            ),
        ] {
            let mut themes = crate::MockThemeStorage::new();
            themes
                .expect_selection()
                .return_once(move |_| Ok(Some(PublicThemeSelection::Custom(theme_id))));
            themes
                .expect_list_themes()
                .return_once(move |_| Ok(entries));
            if theme_id == stale_id {
                themes.expect_list_revisions().return_once(move |_, _| {
                    Ok(vec![custom_revision(
                        theme_id,
                        &"b".repeat(64),
                        &"c".repeat(64),
                    )])
                });
            }

            assert_eq!(
                resolve_public_theme(PublicThemeOwner::Site, &PublicThemeRoute::site(), &themes)
                    .await
                    .unwrap(),
                builtin(Theme::Studio)
            );
        }
    }

    // guard:no-backend — exercises resolver policy through the aggregate storage mock
    #[tokio::test]
    async fn selected_custom_theme_advances_to_its_current_published_revision() {
        let theme_id = ThemeId::from(9);
        let revision_digest = "a".repeat(64);
        let stylesheet_digest = "b".repeat(64);
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_selection()
            .times(1)
            .return_once(move |_| Ok(Some(PublicThemeSelection::Custom(theme_id))));
        let current_revision = revision_digest.clone();
        themes.expect_list_themes().return_once(move |_| {
            Ok(vec![custom_entry(
                ThemeOwner::Site,
                theme_id,
                Some(&current_revision),
            )])
        });
        themes.expect_list_revisions().return_once(move |_, _| {
            Ok(vec![custom_revision(
                theme_id,
                &revision_digest,
                &stylesheet_digest,
            )])
        });
        themes
            .expect_revision_assets()
            .return_once(move |_, _, _| Ok(Vec::new()));
        themes
            .expect_role_binding()
            .times(2)
            .returning(|_, _, _| Ok(None));

        assert_eq!(
            resolve_public_theme(PublicThemeOwner::Site, &PublicThemeRoute::site(), &themes)
                .await
                .unwrap(),
            PublishedThemePresentation {
                identity: PublishedThemeIdentity::Custom(theme_id),
                revision: Some("a".repeat(64).parse().unwrap()),
                stylesheet_url: format!("/theme/{}", "b".repeat(64)).parse().unwrap(),
                logo_url: None,
                header_url: None,
            }
        );
    }

    // guard:no-backend — exercises custom pool retrieval through the aggregate storage mock
    #[tokio::test]
    async fn selected_custom_theme_resolves_its_persisted_header_pool() {
        let theme_id = ThemeId::from(9);
        let entries = vec![crate::ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: Some("images/header-a.png".to_owned()),
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
        }];
        let pool = ThemeHeaderPool::new(
            entries
                .iter()
                .map(pool_entry)
                .collect::<Option<Vec<_>>>()
                .unwrap(),
        )
        .unwrap();
        let binding = ThemeRoleBinding::HeaderPool {
            theme_id,
            pool_revision: pool.revision().clone(),
            shuffle_seed: [0; 32],
        };
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_selection()
            .return_once(move |_| Ok(Some(PublicThemeSelection::Custom(theme_id))));
        themes.expect_list_themes().return_once(move |_| {
            Ok(vec![custom_entry(
                ThemeOwner::Site,
                theme_id,
                Some(&"a".repeat(64)),
            )])
        });
        themes.expect_list_revisions().return_once(move |_, _| {
            Ok(vec![custom_revision(
                theme_id,
                &"a".repeat(64),
                &"b".repeat(64),
            )])
        });
        themes
            .expect_revision_assets()
            .return_once(|_, _, _| Ok(assets()));
        themes.expect_role_binding().returning(move |_, _, role| {
            Ok((role == ThemeImageRole::Header).then_some(binding.clone()))
        });
        themes
            .expect_header_pool()
            .return_once(move |_, _| Ok(entries));

        let presentation =
            resolve_public_theme(PublicThemeOwner::Site, &PublicThemeRoute::site(), &themes)
                .await
                .expect("complete persisted header pool resolves");
        assert_eq!(
            presentation.header_url.unwrap().as_ref(),
            format!("/theme/{}", "d".repeat(64))
        );
    }

    // guard:no-backend — malformed persistence must reach the storage error boundary
    #[tokio::test]
    async fn corrupt_persisted_custom_site_role_propagates_error() {
        let theme_id = ThemeId::from(9);
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_selection()
            .return_once(move |_| Ok(Some(PublicThemeSelection::Custom(theme_id))));
        themes.expect_list_themes().return_once(move |_| {
            Ok(vec![custom_entry(
                ThemeOwner::Site,
                theme_id,
                Some(&"a".repeat(64)),
            )])
        });
        themes.expect_list_revisions().return_once(move |_, _| {
            Ok(vec![custom_revision(
                theme_id,
                &"a".repeat(64),
                &"b".repeat(64),
            )])
        });
        themes
            .expect_revision_assets()
            .return_once(|_, _, _| Ok(Vec::new()));
        themes
            .expect_role_binding()
            .once()
            .returning(|_, _, _| Err(sqlx::Error::RowNotFound));

        assert!(
            resolve_public_theme(PublicThemeOwner::Site, &PublicThemeRoute::site(), &themes)
                .await
                .is_err()
        );
    }

    // guard:no-backend — an unresolved custom role inherits the resolved site presentation
    #[tokio::test]
    async fn invalid_custom_author_role_inherits_site_theme() {
        let author = UserId::from(7);
        let theme_id = ThemeId::from(9);
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_selection().returning(move |owner| {
            Ok(match owner {
                ThemeOwner::Site => Some(PublicThemeSelection::BuiltIn(Theme::Terminal)),
                ThemeOwner::Author(user_id) if user_id == author => {
                    Some(PublicThemeSelection::Custom(theme_id))
                }
                ThemeOwner::Author(_) => {
                    unreachable!("resolver requests only the specified author")
                }
            })
        });
        themes.expect_list_themes().return_once(move |_| {
            Ok(vec![custom_entry(
                ThemeOwner::Author(author),
                theme_id,
                Some(&"a".repeat(64)),
            )])
        });
        themes.expect_list_revisions().return_once(move |_, _| {
            Ok(vec![custom_revision(
                theme_id,
                &"a".repeat(64),
                &"b".repeat(64),
            )])
        });
        themes
            .expect_revision_assets()
            .return_once(|_, _, _| Ok(Vec::new()));
        themes.expect_role_binding().returning(move |_, _, role| {
            Ok(
                (role == ThemeImageRole::Logo).then_some(ThemeRoleBinding::PackageAsset {
                    theme_id,
                    role: ThemeImageRole::Logo,
                    package_path: "missing.png".to_owned(),
                }),
            )
        });

        assert_eq!(
            resolve_public_theme(
                PublicThemeOwner::Author(author),
                &PublicThemeRoute::author(&"alice".parse().unwrap()),
                &themes,
            )
            .await
            .unwrap(),
            builtin(Theme::Terminal)
        );
    }

    fn assets() -> Vec<ThemePackageAsset> {
        vec![
            ThemePackageAsset {
                path: "images/logo.png".to_owned(),
                digest: "c".repeat(64).parse().unwrap(),
                mime: "image/png".to_owned(),
            },
            ThemePackageAsset {
                path: "images/header-a.png".to_owned(),
                digest: "d".repeat(64).parse().unwrap(),
                mime: "image/png".to_owned(),
            },
            ThemePackageAsset {
                path: "images/header-b.png".to_owned(),
                digest: "e".repeat(64).parse().unwrap(),
                mime: "image/png".to_owned(),
            },
        ]
    }

    fn revision() -> ThemeRevisionDigest {
        "a".repeat(64).parse().unwrap()
    }

    #[test]
    fn fixed_package_role_uses_current_revision_asset_digest() {
        let binding = ThemeRoleBinding::PackageAsset {
            theme_id: ThemeId::from(9),
            role: ThemeImageRole::Logo,
            package_path: "images/logo.png".to_owned(),
        };
        assert_eq!(
            resolve_role(
                Some(&binding),
                &assets(),
                &revision(),
                &PublicThemeRoute::site(),
                None,
                None,
            )
            .unwrap()
            .as_ref(),
            format!("/theme/{}", "c".repeat(64))
        );
    }

    #[test]
    fn fixed_media_role_uses_canonical_media_url() {
        let binding = ThemeRoleBinding::Media {
            theme_id: ThemeId::from(9),
            role: ThemeImageRole::Header,
            user_id: UserId::from(1),

            media: common::media::MediaRef {
                source: "upload".parse().unwrap(),
                sha256: "b".repeat(64).parse().unwrap(),
                filename: "hero.jpg".parse().unwrap(),
            },
        };
        assert_eq!(
            resolve_role(
                Some(&binding),
                &assets(),
                &revision(),
                &PublicThemeRoute::site(),
                None,
                None,
            )
            .unwrap()
            .as_ref(),
            format!("/media/upload/bb/bb/{}/hero.jpg", "b".repeat(64))
        );
    }
    #[test]
    fn header_default_binding_uses_the_packaged_header_pool_policy() {
        let defaults = vec!["images/header-a.png".to_owned()];
        let binding = ThemeRoleBinding::PackagedDefault {
            theme_id: ThemeId::from(9),
            role: ThemeImageRole::Header,
        };
        assert_eq!(
            resolve_role(
                Some(&binding),
                &assets(),
                &revision(),
                &PublicThemeRoute::site(),
                None,
                Some(&defaults),
            )
            .unwrap()
            .as_ref(),
            format!("/theme/{}", "d".repeat(64))
        );
    }

    #[test]
    fn packaged_header_defaults_select_from_complete_pool_per_route() {
        let defaults = vec![
            "images/header-a.png".to_owned(),
            "images/header-b.png".to_owned(),
        ];
        let package_url = |path: &str| package_url(&assets(), path);
        let site = resolve_packaged_header_default(
            Some(&defaults),
            &package_url,
            &revision(),
            &PublicThemeRoute::site(),
        );
        let site_again = resolve_packaged_header_default(
            Some(&defaults),
            &package_url,
            &revision(),
            &PublicThemeRoute::site(),
        );
        assert_eq!(site, site_again);
        assert!(
            [
                format!("/theme/{}", "d".repeat(64)),
                format!("/theme/{}", "e".repeat(64)),
            ]
            .contains(&site.unwrap().to_string())
        );
        let selections = [
            PublicThemeRoute::site(),
            PublicThemeRoute::author(&"alice".parse().unwrap()),
            PublicThemeRoute::author(&"bob".parse().unwrap()),
            PublicThemeRoute::site_tag(&"rust".parse().unwrap()),
        ]
        .into_iter()
        .map(|route| {
            resolve_packaged_header_default(Some(&defaults), &package_url, &revision(), &route)
                .unwrap()
                .to_string()
        })
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(selections.len(), 2);
    }

    #[test]
    fn mixed_header_pool_is_route_stable() {
        let entries = vec![crate::ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: Some("images/logo.png".to_owned()),
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
        }];
        let pool = ThemeHeaderPool::new(
            entries
                .iter()
                .map(pool_entry)
                .collect::<Option<Vec<_>>>()
                .unwrap(),
        )
        .unwrap();
        let binding = ThemeRoleBinding::HeaderPool {
            theme_id: ThemeId::from(9),
            pool_revision: pool.revision().clone(),
            shuffle_seed: [7; 32],
        };
        let route = PublicThemeRoute::author(&"alice".parse().unwrap());
        assert_eq!(
            resolve_role(
                Some(&binding),
                &assets(),
                &revision(),
                &route,
                Some(&entries),
                None,
            ),
            resolve_role(
                Some(&binding),
                &assets(),
                &revision(),
                &route,
                Some(&entries),
                None,
            )
        );
    }

    #[test]
    fn role_resolution_rejects_stale_pools_and_preserves_explicit_absence() {
        let stale_pool = ThemeRoleBinding::HeaderPool {
            theme_id: ThemeId::from(9),
            pool_revision: "f".repeat(64).parse().unwrap(),
            shuffle_seed: [0; 32],
        };
        let entries = vec![crate::ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: Some("images/logo.png".to_owned()),
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
        }];
        assert_eq!(
            resolve_role(
                Some(&stale_pool),
                &assets(),
                &revision(),
                &PublicThemeRoute::site(),
                Some(&entries),
                None,
            ),
            None,
            "a persisted pool revision must match its canonical entries"
        );
        let absent = ThemeRoleBinding::ExplicitAbsent {
            theme_id: ThemeId::from(9),
            role: ThemeImageRole::Logo,
        };
        assert_eq!(
            resolve_role(
                Some(&absent),
                &assets(),
                &revision(),
                &PublicThemeRoute::site(),
                None,
                None,
            ),
            None
        );
        assert!(!role_is_invalid(Some(&absent), None));
    }

    #[test]
    fn package_defaults_and_pool_media_require_complete_valid_inputs() {
        let logo_default = ThemeRoleBinding::PackagedDefault {
            theme_id: ThemeId::from(9),
            role: ThemeImageRole::Logo,
        };
        assert_eq!(
            resolve_role(
                Some(&logo_default),
                &assets(),
                &revision(),
                &PublicThemeRoute::site(),
                None,
                Some(&["images/logo.png".to_owned()]),
            )
            .unwrap()
            .as_ref(),
            format!("/theme/{}", "c".repeat(64))
        );
        assert!(media_url_parts("invalid", &"b".repeat(64), "hero.jpg").is_none());
        assert!(
            pool_entry(&crate::ThemeHeaderPoolEntry {
                ordinal: 0,
                package_path: Some("images/logo.png".to_owned()),
                media_user_id: Some(UserId::from(1)),
                media_source: None,
                media_digest: None,
                media_filename: None,
            })
            .is_none()
        );
        let media = ThemePoolEntry::Media {
            source: "upload".to_owned(),
            digest: "b".repeat(64).parse().unwrap(),
            filename: "hero.jpg".to_owned(),
        };
        assert_eq!(
            resolve_pool_entry(&media, &|_| None).unwrap().as_ref(),
            format!("/media/upload/bb/bb/{}/hero.jpg", "b".repeat(64))
        );
    }

    // guard:no-backend — draft URL resolution is isolated from relational persistence
    #[tokio::test]
    async fn draft_preview_resolves_packaged_header_defaults_and_pool_media() {
        let theme_id = ThemeId::from(9);
        let revision = revision();
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_role_binding().returning(move |_, _, role| {
            Ok(
                (role == ThemeImageRole::Header).then_some(ThemeRoleBinding::PackagedDefault {
                    theme_id,
                    role: ThemeImageRole::Header,
                }),
            )
        });
        let urls = std::collections::BTreeMap::from([
            ("images/header-a.png".to_owned(), "/draft/a".to_owned()),
            ("images/header-b.png".to_owned(), "/draft/b".to_owned()),
        ]);
        let (_, header) = resolve_draft_theme_images(
            ThemeOwner::Site,
            theme_id,
            br#"{"defaults":{"header":["images/header-a.png","images/header-b.png"]}}"#,
            &urls,
            &revision,
            &PublicThemeRoute::site(),
            &themes,
        )
        .await
        .expect("complete package defaults resolve for draft preview");
        assert!(matches!(header.unwrap().as_ref(), "/draft/a" | "/draft/b"));

        let entry = crate::ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: None,
            media_user_id: Some(UserId::from(1)),
            media_source: Some("upload".to_owned()),
            media_digest: Some("b".repeat(64).parse().unwrap()),
            media_filename: Some("hero.jpg".to_owned()),
        };
        assert!(matches!(
            pool_entry(&entry),
            Some(ThemePoolEntry::Media { .. })
        ));
    }

    // guard:no-backend — draft pool retrieval and invalid-role rejection are resolver policy
    #[tokio::test]
    async fn draft_preview_rejects_unresolvable_persisted_header_pool() {
        let theme_id = ThemeId::from(9);
        let entries = vec![crate::ThemeHeaderPoolEntry {
            ordinal: 0,
            package_path: Some("images/header-a.png".to_owned()),
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
        }];
        let pool = ThemeHeaderPool::new(
            entries
                .iter()
                .map(pool_entry)
                .collect::<Option<Vec<_>>>()
                .unwrap(),
        )
        .unwrap();
        let binding = ThemeRoleBinding::HeaderPool {
            theme_id,
            pool_revision: pool.revision().clone(),
            shuffle_seed: [0; 32],
        };
        let mut themes = crate::MockThemeStorage::new();
        themes.expect_role_binding().returning(move |_, _, role| {
            Ok((role == ThemeImageRole::Header).then_some(binding.clone()))
        });
        themes
            .expect_header_pool()
            .once()
            .return_once(move |_, _| Ok(entries));

        assert!(
            resolve_draft_theme_images(
                ThemeOwner::Site,
                theme_id,
                b"{}",
                &std::collections::BTreeMap::new(),
                &revision(),
                &PublicThemeRoute::site(),
                &themes,
            )
            .await
            .is_err(),
            "a persisted binding whose selected asset has no draft URL is invalid"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn persisted_selections_preserve_site_author_precedence(#[case] backend: Backend) {
        let env = backend.setup().await;
        let author = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await;
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .set_selection(
                                transaction,
                                ThemeOwner::Site,
                                Some(PublicThemeSelection::BuiltIn(Theme::Terminal)),
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );

        assert_eq!(
            resolve_public_theme(
                PublicThemeOwner::Site,
                &PublicThemeRoute::site(),
                env.themes().as_ref(),
            )
            .await
            .unwrap(),
            builtin(Theme::Terminal)
        );
        assert_eq!(
            resolve_public_theme(
                PublicThemeOwner::Author(author.user_id),
                &PublicThemeRoute::site(),
                env.themes().as_ref(),
            )
            .await
            .unwrap(),
            builtin(Theme::Terminal)
        );

        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .set_selection(
                                transaction,
                                ThemeOwner::Author(author.user_id),
                                Some(PublicThemeSelection::BuiltIn(Theme::Reader)),
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );

        assert_eq!(
            resolve_public_theme(
                PublicThemeOwner::Author(author.user_id),
                &PublicThemeRoute::site(),
                env.themes().as_ref(),
            )
            .await
            .unwrap(),
            builtin(Theme::Reader)
        );
    }
}
