//! Viewer-independent public theme resolution.

use common::{
    ids::{ThemeId, UserId},
    root_relative_url::RootRelativeUrl,
    theme::{PublicThemeSelection, PublishedThemeIdentity, PublishedThemePresentation, Theme},
};

use crate::{ThemeOwner, ThemeStorage};

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
    themes: &dyn ThemeStorage,
) -> Result<PublishedThemePresentation, sqlx::Error> {
    let site = resolve_selection(ThemeOwner::Site, themes)
        .await?
        .unwrap_or_else(|| PublishedThemePresentation::built_in(Theme::Studio));
    match owner {
        PublicThemeOwner::Site => Ok(site),
        PublicThemeOwner::Author(user_id) => {
            Ok(resolve_selection(ThemeOwner::Author(user_id), themes)
                .await?
                .unwrap_or(site))
        }
    }
}

async fn resolve_selection(
    owner: ThemeOwner,
    themes: &dyn ThemeStorage,
) -> Result<Option<PublishedThemePresentation>, sqlx::Error> {
    match themes.selection(owner).await? {
        None => Ok(None),
        Some(PublicThemeSelection::BuiltIn(theme)) => {
            Ok(Some(PublishedThemePresentation::built_in(theme)))
        }
        Some(PublicThemeSelection::Custom(theme_id)) => {
            resolve_custom(owner, theme_id, themes).await
        }
    }
}

async fn resolve_custom(
    owner: ThemeOwner,
    theme_id: ThemeId,
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
    let stylesheet_url =
        RootRelativeUrl::try_from(format!("/themes/{}", revision.stylesheet_digest))
            .map_err(|_| sqlx::Error::RowNotFound)?;
    Ok(Some(PublishedThemePresentation {
        identity: PublishedThemeIdentity::Custom(theme_id),
        revision: Some(revision.digest),
        stylesheet_url,
        logo_url: None,
        header_url: None,
    }))
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
            resolve_public_theme(PublicThemeOwner::Site, &themes)
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
            resolve_public_theme(PublicThemeOwner::Author(author), &themes)
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
            resolve_public_theme(PublicThemeOwner::Author(author), &themes)
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
            resolve_public_theme(PublicThemeOwner::Site, &themes)
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
            resolve_public_theme(PublicThemeOwner::Site, &themes)
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
            resolve_public_theme(PublicThemeOwner::Site, &themes)
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
                resolve_public_theme(PublicThemeOwner::Site, &themes)
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

        assert_eq!(
            resolve_public_theme(PublicThemeOwner::Site, &themes)
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
            resolve_public_theme(PublicThemeOwner::Site, env.state.themes.as_ref())
                .await
                .unwrap(),
            builtin(Theme::Terminal)
        );
        assert_eq!(
            resolve_public_theme(
                PublicThemeOwner::Author(author.user_id),
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
                env.state.themes.as_ref(),
            )
            .await
            .unwrap(),
            builtin(Theme::Reader)
        );
    }
}
