//! Typed wire API for operator-managed SMTP relay settings.

use crate::error::WebResult;
use common::MutationOutcome;
use common::smtp_host::SmtpHost;
use common::smtp_password::ProfferedSmtpPassword;
use common::smtp_port::SmtpPort;
use common::smtp_sender::SmtpSender;
use common::smtp_tls_mode::SmtpTlsMode;
use common::smtp_username::SmtpUsername;
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::{auth, error},
    host::smtp_config::{SmtpConfigUpdate, SmtpCredentialsUpdate},
    host::smtp_password::SmtpPassword,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{SiteConfigStorage, SmtpConfigUpdateError, WriteScope},
};

/// The secret-free SMTP configuration presented to the operator.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub enabled: bool,
    pub host: Option<SmtpHost>,
    pub port: SmtpPort,
    pub tls_mode: SmtpTlsMode,
    pub sender: SmtpSender,
    pub authentication_enabled: bool,
    pub username: Option<SmtpUsername>,
    pub password_configured: bool,
}

/// One cohesive SMTP settings mutation. The password is inbound-only and is
/// converted to the host secret type as soon as the server receives it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateSettingsRequest {
    pub enabled: bool,
    pub host: Option<SmtpHost>,
    pub port: SmtpPort,
    pub tls_mode: SmtpTlsMode,
    pub sender: SmtpSender,
    pub authentication_enabled: bool,
    pub username: Option<SmtpUsername>,
    pub password: Option<ProfferedSmtpPassword>,
}

#[macros::server(skip_all)]
pub async fn get_settings() -> WebResult<Settings> {
    auth::require_operator().await?;
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let Some(config) = site_config
        .get_smtp_config()
        .await
        .map_err(error::InternalError::storage)?
    else {
        return Ok(Settings::default());
    };

    let password_configured = config.password.is_some();
    Ok(Settings {
        enabled: true,
        host: Some(config.host),
        port: config.port,
        tls_mode: config.tls_mode,
        sender: config.sender,
        authentication_enabled: config.username.is_some() || password_configured,
        username: config.username,
        password_configured,
    })
}

#[cfg(feature = "server")]
fn map_smtp_update_error(error: SmtpConfigUpdateError) -> error::InternalError {
    match error {
        SmtpConfigUpdateError::MissingStoredPassword => {
            error::InternalError::conflict("SMTP authentication changed; enter the password again")
        }
        SmtpConfigUpdateError::Database(error) => error::InternalError::storage(error),
    }
}

#[macros::server(skip_all)]
pub async fn update_settings(request: UpdateSettingsRequest) -> WebResult<MutationOutcome<()>> {
    auth::require_operator().await?;

    let UpdateSettingsRequest {
        enabled,
        host,
        port,
        tls_mode,
        sender,
        authentication_enabled,
        username,
        password,
    } = request;
    let password = password
        .map(SmtpPassword::try_from)
        .transpose()
        .map_err(|_| error::InternalError::validation("invalid SMTP password"))?;

    let update = if enabled {
        let host = host.ok_or_else(|| error::InternalError::validation("SMTP host is required"))?;
        let credentials = if authentication_enabled {
            let username = username
                .ok_or_else(|| error::InternalError::validation("SMTP username is required"))?;
            match password {
                Some(password) => SmtpCredentialsUpdate::Replace { username, password },
                None => SmtpCredentialsUpdate::Keep { username },
            }
        } else {
            if password.is_some() {
                return Err(error::InternalError::validation(
                    "an SMTP password cannot be supplied while authentication is disabled",
                ));
            }
            SmtpCredentialsUpdate::Unauthenticated
        };
        SmtpConfigUpdate::Enabled {
            host,
            port,
            tls_mode,
            sender,
            credentials,
        }
    } else {
        if password.is_some() {
            return Err(error::InternalError::validation(
                "an SMTP password cannot be supplied while SMTP is disabled",
            ));
        }
        SmtpConfigUpdate::Disabled
    };

    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                site_config
                    .update_smtp_config(transaction, &update)
                    .await
                    .map_err(map_smtp_update_error)
            })
        })
        .await
        .map_err(error::from_write_scope_error)
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::map_smtp_update_error;
    use crate::error::ErrorKind;
    use storage::SmtpConfigUpdateError;

    #[test]
    fn update_error_mapping_preserves_conflict_and_storage_classes() {
        assert_eq!(
            map_smtp_update_error(SmtpConfigUpdateError::MissingStoredPassword).kind(),
            ErrorKind::Conflict
        );
        assert_eq!(
            map_smtp_update_error(SmtpConfigUpdateError::Database(sqlx::Error::RowNotFound)).kind(),
            ErrorKind::Storage
        );
    }
}
