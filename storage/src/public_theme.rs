//! Viewer-independent public theme resolution.

use common::{
    ids::{ThemeId, UserId},
    media::{self, ContentHash, Filename, MediaSource},
    root_relative_url::RootRelativeUrl,
    theme::{
        PublicThemeRoute, PublicThemeSelection, PublishedThemeIdentity, PublishedThemePresentation,
        Theme, ThemeHeaderPool, ThemeImageBindingMode, ThemeImageRole, ThemePoolEntry,
        ThemeRevisionDigest,
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
/// A missing, unpublished, deleted, or corrupt site selection falls back to Studio. An
/// invalid author selection inherits that already-resolved site presentation. Storage failures
/// deliberately propagate rather than being rendered as a false fallback.
///
/// # Errors
///
/// Returns an underlying database error when a selection or theme record cannot be read.
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
enum RoleBindingRead {
    Valid(Option<ThemeRoleBinding>),
    Corrupt,
}

async fn read_role_binding(
    themes: &dyn ThemeStorage,
    owner: ThemeOwner,
    theme_id: ThemeId,
    role: ThemeImageRole,
) -> Result<RoleBindingRead, sqlx::Error> {
    match themes.role_binding(owner, theme_id, role).await {
        Ok(binding) => Ok(RoleBindingRead::Valid(binding)),
        Err(sqlx::Error::RowNotFound) => Ok(RoleBindingRead::Corrupt),
        Err(error) => Err(error),
    }
}

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
    let logo = match read_role_binding(themes, owner, theme_id, ThemeImageRole::Logo).await? {
        RoleBindingRead::Valid(binding) => binding,
        RoleBindingRead::Corrupt => return Ok(None),
    };
    let header = match read_role_binding(themes, owner, theme_id, ThemeImageRole::Header).await? {
        RoleBindingRead::Valid(binding) => binding,
        RoleBindingRead::Corrupt => return Ok(None),
    };
    let defaults = manifest_defaults(&revision.manifest);
    let logo_default = defaults
        .as_ref()
        .and_then(|defaults| defaults.logo.as_deref());
    let header_default = defaults
        .as_ref()
        .and_then(|defaults| defaults.header.as_deref())
        .and_then(|header| header.first().map(String::as_str));
    if (logo
        .as_ref()
        .is_some_and(|binding| binding.mode == ThemeImageBindingMode::PackagedDefault)
        && logo_default
            .and_then(|path| package_url(&assets, path))
            .is_none())
        || (header
            .as_ref()
            .is_some_and(|binding| binding.mode == ThemeImageBindingMode::PackagedDefault)
            && header_default
                .and_then(|path| package_url(&assets, path))
                .is_none())
    {
        return Ok(None);
    }
    let header_pool = if header
        .as_ref()
        .is_some_and(|binding| binding.mode == ThemeImageBindingMode::HeaderPool)
    {
        Some(themes.header_pool(owner, theme_id).await?)
    } else {
        None
    };
    let stylesheet_url =
        RootRelativeUrl::try_from(format!("/themes/{}", revision.stylesheet_digest))
            .map_err(|_| sqlx::Error::RowNotFound)?;
    let logo_url = resolve_role(
        logo.as_ref(),
        &assets,
        &revision.digest,
        route,
        None,
        logo_default,
    );
    let header_url = resolve_role(
        header.as_ref(),
        &assets,
        &revision.digest,
        route,
        header_pool.as_deref(),
        header_default,
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

fn resolve_role(
    binding: Option<&ThemeRoleBinding>,
    assets: &[ThemePackageAsset],
    revision: &ThemeRevisionDigest,
    route: &PublicThemeRoute,
    pool_entries: Option<&[crate::ThemeHeaderPoolEntry]>,
    packaged_default: Option<&str>,
) -> Option<RootRelativeUrl> {
    let binding = binding?;
    match binding.mode {
        ThemeImageBindingMode::PackagedDefault => {
            packaged_default.and_then(|path| package_url(assets, path))
        }
        ThemeImageBindingMode::ExplicitAbsent => None,
        ThemeImageBindingMode::PackageAsset => binding
            .package_path
            .as_deref()
            .and_then(|path| package_url(assets, path)),
        ThemeImageBindingMode::Media => media_url(binding),
        ThemeImageBindingMode::HeaderPool => {
            if binding.role != ThemeImageRole::Header {
                return None;
            }
            let entries = pool_entries?;
            let pool =
                ThemeHeaderPool::new(entries.iter().map(pool_entry).collect::<Option<Vec<_>>>()?)
                    .ok()?;
            let shuffle_seed = binding.shuffle_seed?;
            if binding.pool_revision.as_ref() != Some(pool.revision()) {
                return None;
            }
            match pool.select(route, revision, &shuffle_seed) {
                ThemePoolEntry::Package(path) => package_url(assets, path),
                ThemePoolEntry::Media {
                    source,
                    digest,
                    filename,
                } => media_url_parts(source, digest.as_ref(), filename),
            }
        }
    }
}

fn role_is_invalid(binding: Option<&ThemeRoleBinding>, url: Option<&RootRelativeUrl>) -> bool {
    binding.is_some_and(|binding| {
        binding.mode != ThemeImageBindingMode::ExplicitAbsent && url.is_none()
    })
}

fn package_url(assets: &[ThemePackageAsset], path: &str) -> Option<RootRelativeUrl> {
    let asset = assets.iter().find(|asset| asset.path == path)?;
    format!("/themes/{}", asset.digest).parse().ok()
}

fn media_url(binding: &ThemeRoleBinding) -> Option<RootRelativeUrl> {
    media_url_parts(
        binding.media_source.as_deref()?,
        binding.media_digest.as_ref()?.as_ref(),
        binding.media_filename.as_deref()?,
    )
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
        entry.media_source.as_deref(),
        entry.media_digest.as_ref(),
        entry.media_filename.as_deref(),
    ) {
        (Some(path), None, None, None) => Some(ThemePoolEntry::Package(path.to_owned())),
        (None, Some(source), Some(digest), Some(filename)) => Some(ThemePoolEntry::Media {
            source: source.to_owned(),
            digest: digest.clone(),
            filename: filename.to_owned(),
        }),
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
                ThemeOwner::Author(_) => unreachable!(),
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
                ThemeOwner::Author(_) => unreachable!(),
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
                stylesheet_url: format!("/themes/{}", "b".repeat(64)).parse().unwrap(),
                logo_url: None,
                header_url: None,
            }
        );
    }

    // guard:no-backend — corrupt persisted role data must not leak a partial custom theme
    #[tokio::test]
    async fn corrupt_persisted_custom_site_role_falls_back_to_studio() {
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

        assert_eq!(
            resolve_public_theme(PublicThemeOwner::Site, &PublicThemeRoute::site(), &themes)
                .await
                .unwrap(),
            builtin(Theme::Studio)
        );
    }

    // guard:no-backend — an invalid author custom theme inherits the resolved site presentation
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
                ThemeOwner::Author(_) => unreachable!(),
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
            let mut binding = binding(ThemeImageRole::Logo, ThemeImageBindingMode::PackageAsset);
            binding.package_path = Some("missing.png".to_owned());
            Ok((role == ThemeImageRole::Logo).then_some(binding))
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
        vec![ThemePackageAsset {
            path: "images/logo.png".to_owned(),
            digest: "c".repeat(64).parse().unwrap(),
            mime: "image/png".to_owned(),
        }]
    }

    fn revision() -> ThemeRevisionDigest {
        "a".repeat(64).parse().unwrap()
    }

    fn binding(role: ThemeImageRole, mode: ThemeImageBindingMode) -> ThemeRoleBinding {
        ThemeRoleBinding {
            theme_id: ThemeId::from(9),
            role,
            mode,
            package_path: None,
            media_user_id: None,
            media_source: None,
            media_digest: None,
            media_filename: None,
            pool_revision: None,
            shuffle_seed: None,
        }
    }

    #[test]
    fn absent_role_has_no_url() {
        assert_eq!(
            resolve_role(
                None,
                &assets(),
                &revision(),
                &PublicThemeRoute::site(),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn fixed_package_role_uses_current_revision_asset_digest() {
        let mut binding = binding(ThemeImageRole::Logo, ThemeImageBindingMode::PackageAsset);
        binding.package_path = Some("images/logo.png".to_owned());
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
            format!("/themes/{}", "c".repeat(64))
        );
    }

    #[test]
    fn fixed_media_role_uses_canonical_media_url() {
        let mut binding = binding(ThemeImageRole::Header, ThemeImageBindingMode::Media);
        binding.media_user_id = Some(UserId::from(1));
        binding.media_source = Some("upload".to_owned());
        binding.media_digest = Some("b".repeat(64).parse().unwrap());
        binding.media_filename = Some("hero.jpg".to_owned());
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
    fn mixed_header_pool_is_route_stable() {
        let assets = assets();
        let entries = vec![
            crate::ThemeHeaderPoolEntry {
                ordinal: 0,
                package_path: Some("images/logo.png".to_owned()),
                media_user_id: None,
                media_source: None,
                media_digest: None,
                media_filename: None,
            },
            crate::ThemeHeaderPoolEntry {
                ordinal: 1,
                package_path: None,
                media_user_id: Some(UserId::from(1)),
                media_source: Some("upload".to_owned()),
                media_digest: Some("b".repeat(64).parse().unwrap()),
                media_filename: Some("hero.jpg".to_owned()),
            },
        ];
        let pool = ThemeHeaderPool::new(
            entries
                .iter()
                .map(pool_entry)
                .collect::<Option<Vec<_>>>()
                .unwrap(),
        )
        .unwrap();
        let mut binding = binding(ThemeImageRole::Header, ThemeImageBindingMode::HeaderPool);
        binding.pool_revision = Some(pool.revision().clone());
        binding.shuffle_seed = Some([7; 32]);
        let route = PublicThemeRoute::author(&"alice".parse().unwrap());
        let first = resolve_role(
            Some(&binding),
            &assets,
            &revision(),
            &route,
            Some(&entries),
            None,
        );
        let second = resolve_role(
            Some(&binding),
            &assets,
            &revision(),
            &route,
            Some(&entries),
            None,
        );
        assert_eq!(first, second);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn persisted_selections_preserve_site_author_precedence(#[case] backend: Backend) {
        let env = backend.setup().await;
        let author = SeedUser::new().seed(&env.state).await;
        let themes = Arc::clone(&env.state.themes);
        confirmed(
            env.state
                .write_scope
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
                env.state.themes.as_ref(),
            )
            .await
            .unwrap(),
            builtin(Theme::Terminal)
        );
        assert_eq!(
            resolve_public_theme(
                PublicThemeOwner::Author(author.user_id),
                &PublicThemeRoute::site(),
                env.state.themes.as_ref(),
            )
            .await
            .unwrap(),
            builtin(Theme::Terminal)
        );

        let themes = Arc::clone(&env.state.themes);
        confirmed(
            env.state
                .write_scope
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
                env.state.themes.as_ref(),
            )
            .await
            .unwrap(),
            builtin(Theme::Reader)
        );
    }
}
