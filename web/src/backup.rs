use crate::error::WebResult;
use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::{
    auth::require_auth,
    error::{InternalError, WebError},
};
#[cfg(feature = "ssr")]
use common::storage::{AppState, BACKUP_DESTINATION_PATH_KEY};
#[cfg(feature = "ssr")]
use std::sync::Arc;

#[cfg(feature = "ssr")]
fn backup_destination_configured(destination: Option<&str>) -> bool {
    destination
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

#[server(endpoint = "/backup_warning_visible")]
#[cfg_attr(
    feature = "ssr",
    tracing::instrument(name = "web.backup.warning_visible")
)]
pub async fn backup_warning_visible() -> WebResult<bool> {
    crate::web_server_fn!("backup_warning_visible", => {
        let auth = match require_auth().await {
            Ok(auth) => auth,
            Err(error) if matches!(error.public(), WebError::Unauthorized) => return Ok(false),
            Err(error) => return Err(error),
        };

        let state = expect_context::<Arc<AppState>>();
        let Some(user) = state
            .users
            .get_user(auth.user_id)
            .await
            .map_err(InternalError::storage)?
        else {
            return Ok(false);
        };

        if !user.is_operator {
            return Ok(false);
        }

        let destination = state
            .site_config
            .get(BACKUP_DESTINATION_PATH_KEY)
            .await
            .map_err(InternalError::storage)?;

        Ok(!backup_destination_configured(destination.as_deref()))
    })
}

#[cfg(test)]
mod tests {
    use super::backup_destination_configured;

    #[test]
    fn backup_destination_configured_rejects_empty_values() {
        assert!(!backup_destination_configured(None));
        assert!(!backup_destination_configured(Some("")));
        assert!(!backup_destination_configured(Some("  ")));
    }

    #[test]
    fn backup_destination_configured_accepts_nonempty_values() {
        assert!(backup_destination_configured(Some("/srv/backups")));
    }
}
