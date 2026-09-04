//! Viewer-independent public theme resolution.

use common::{ids::UserId, theme::Theme};

use crate::{SiteConfigStorage, UserConfigStorage};

/// Ownership scope for one public presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicThemeOwner {
    /// Aggregate pages use the operator-selected site theme.
    Site,
    /// Author-owned pages may override the site theme.
    Author(UserId),
}

/// Resolve the effective theme for one public presentation.
///
/// Missing and malformed stored values are normalized by the typed accessors.
/// Operational read errors propagate rather than rendering a false default.
///
/// # Errors
///
/// Returns the underlying database error when a required configuration read
/// fails.
pub async fn resolve_public_theme(
    owner: PublicThemeOwner,
    site_config: &dyn SiteConfigStorage,
    user_config: &dyn UserConfigStorage,
) -> Result<Theme, sqlx::Error> {
    let site_theme = site_config.get_theme().await?;
    match owner {
        PublicThemeOwner::Site => Ok(site_theme),
        PublicThemeOwner::Author(user_id) => Ok(crate::get_theme_override(user_config, user_id)
            .await?
            .unwrap_or(site_theme)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // guard:no-backend — exercises the resolver through storage trait mocks only
    #[tokio::test]
    async fn site_owner_uses_site_theme_without_reading_author_config() {
        let mut site_config = crate::MockSiteConfigStorage::new();
        site_config
            .expect_get_theme()
            .return_once(|| Ok(Theme::Terminal));
        let user_config = crate::MockUserConfigStorage::new();

        assert_eq!(
            resolve_public_theme(PublicThemeOwner::Site, &site_config, &user_config)
                .await
                .unwrap(),
            Theme::Terminal
        );
    }

    // guard:no-backend — exercises the resolver through storage trait mocks only
    #[tokio::test]
    async fn author_without_override_inherits_site_theme() {
        let author = UserId::from(7);
        let mut site_config = crate::MockSiteConfigStorage::new();
        site_config
            .expect_get_theme()
            .return_once(|| Ok(Theme::Terminal));
        let mut user_config = crate::MockUserConfigStorage::new();
        user_config.expect_get().return_once(move |user_id, _| {
            assert_eq!(user_id, author);
            Ok(None)
        });

        assert_eq!(
            resolve_public_theme(PublicThemeOwner::Author(author), &site_config, &user_config)
                .await
                .unwrap(),
            Theme::Terminal
        );
    }

    // guard:no-backend — exercises the resolver through storage trait mocks only
    #[tokio::test]
    async fn author_override_wins_over_site_theme() {
        let author = UserId::from(7);
        let mut site_config = crate::MockSiteConfigStorage::new();
        site_config
            .expect_get_theme()
            .return_once(|| Ok(Theme::Terminal));
        let mut user_config = crate::MockUserConfigStorage::new();
        user_config.expect_get().return_once(move |user_id, _| {
            assert_eq!(user_id, author);
            Ok(Some("reader".to_owned()))
        });

        assert_eq!(
            resolve_public_theme(PublicThemeOwner::Author(author), &site_config, &user_config)
                .await
                .unwrap(),
            Theme::Reader
        );
    }

    // guard:no-backend — exercises the resolver through storage trait mocks only
    #[tokio::test]
    async fn site_theme_read_failure_propagates() {
        let mut site_config = crate::MockSiteConfigStorage::new();
        site_config.expect_get_theme().return_once(|| {
            Err(sqlx::Error::Io(std::io::Error::other(
                "injected site theme read failure",
            )))
        });
        let user_config = crate::MockUserConfigStorage::new();

        assert!(
            resolve_public_theme(PublicThemeOwner::Site, &site_config, &user_config)
                .await
                .is_err()
        );
    }
}
