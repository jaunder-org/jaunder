use std::sync::Arc;

use anyhow::Context;
use common::display_name::DisplayName;
use common::email::Email;
use common::invite::InviteTtlHours;
use common::mailer::{EmailMessage, MailSender};
use common::session_label::SessionLabel;
use common::tagged_url::{self, MailConfirmUrl};
use common::token::RawToken;
use common::username::Username;
use host::password::Password;
use host::smtp_config::SmtpConfig;
use storage::OperatorStatus;

use crate::cli::StorageArgs;
use crate::mailer::LettreMailSender;

use super::support;

async fn create_command_user(
    write_scope: &storage::WriteScope,
    users: Arc<dyn storage::UserStorage>,
    username: Username,
    password: Password,
    display_name: Option<DisplayName>,
    is_operator: OperatorStatus,
) -> anyhow::Result<common::ids::UserId> {
    let password = storage::prepare_password(password)
        .await
        .context("failed to create user")?;
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                users
                    .create_user(
                        transaction,
                        &username,
                        &password,
                        display_name.as_ref(),
                        is_operator,
                    )
                    .await
            })
        })
        .await
        .map_err(anyhow::Error::from)
        .context("failed to create user")?;
    support::require_confirmed_mutation(outcome, "user creation")
}

/// Creates a new user in the database.
///
/// # Errors
///
/// Returns an error if the database cannot be opened, or if the user creation
/// fails (e.g., duplicate username).
pub async fn cmd_user_create(
    storage: &StorageArgs,
    username: &Username,
    password: Option<Password>,
    display_name: Option<&DisplayName>,
    is_operator: bool,
) -> anyhow::Result<()> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime)
        .await
        .context(support::INIT_FIRST_CONTEXT)?;

    let password = if let Some(p) = password {
        p
    } else {
        // cov:ignore-start
        let p1 = rpassword::prompt_password("Password: ")?;
        let p2 = rpassword::prompt_password("Confirm password: ")?;
        if p1 != p2 {
            return Err(anyhow::anyhow!("passwords do not match"));
        }
        p1.parse::<Password>().map_err(|e| anyhow::anyhow!("{e}"))?
        // cov:ignore-stop
    };

    let user_id = create_command_user(
        &state.write_scope,
        Arc::clone(&state.users),
        username.clone(),
        password,
        display_name.cloned(),
        if is_operator {
            OperatorStatus::OPERATOR
        } else {
            OperatorStatus::STANDARD
        },
    )
    .await?;

    // CLI user creation bypasses the site registration policy entirely.
    host::metrics::registration(
        host::metrics::RegistrationSource::Cli,
        host::metrics::RegistrationPolicy::CliBypass,
        host::metrics::RegistrationResult::Ok,
    );

    println!("Created user '{username}' with id {}", i64::from(user_id));
    Ok(())
}

/// Mints an app password (a labelled session token) for an existing user and
/// returns the raw token. This is the only out-of-process minter (see ADR-0035).
///
/// # Errors
///
/// Returns an error if the user does not exist or the session cannot be created.
pub async fn app_password_create(
    write_scope: &storage::WriteScope,
    users: &dyn storage::UserStorage,
    sessions: Arc<dyn storage::SessionStorage>,
    username: &Username,
    label: SessionLabel,
) -> anyhow::Result<RawToken> {
    let user = users
        .get_user_by_username(username)
        .await
        .context("failed to look up user")?
        .ok_or_else(|| anyhow::anyhow!("no such user '{username}'"))?;
    // No validation here: the signature carries it. `SessionLabel` cannot be built from
    // an invalid string, so there is nothing left to check and no step to remember.
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                sessions
                    .create_session(transaction, user.user_id, &label)
                    .await
            })
        })
        .await
        .map_err(anyhow::Error::from)
        .context("failed to create app password")?;
    support::require_confirmed_mutation(outcome, "app password")
}

/// CLI wrapper: opens the database, mints an app password, prints it to stdout.
///
/// # Errors
///
/// Returns an error if the database cannot be opened or minting fails.
pub async fn cmd_app_password_create(
    storage: &StorageArgs,
    username: &Username,
    label: &SessionLabel,
) -> anyhow::Result<()> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime)
        .await
        .context(support::INIT_FIRST_CONTEXT)?;
    let token = app_password_create(
        &state.write_scope,
        state.users(),
        Arc::clone(&state.sessions),
        username,
        label.clone(),
    )
    .await?;
    println!("{token}");
    Ok(())
}

/// Generates a new invitation code.
///
/// # Errors
///
/// Returns an error if the database cannot be opened, or if the invitation
/// cannot be saved.
pub async fn cmd_user_invite(
    storage: &StorageArgs,
    expires_in: Option<InviteTtlHours>,
) -> anyhow::Result<()> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime)
        .await
        .context(support::INIT_FIRST_CONTEXT)?;

    // The 1..=336 bound lives in `InviteTtlHours` (clap rejects an out-of-range `--expires-in`
    // at parse), so no in-body overflow check is needed.
    let expires_at = common::time::UtcInstant::from(
        chrono::Utc::now() + chrono::Duration::hours(expires_in.unwrap_or_default().value()),
    );

    let invites = Arc::clone(&state.invites);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { invites.create_invite(transaction, expires_at).await })
        })
        .await
        .map_err(anyhow::Error::from)
        .context("failed to create invite")?;
    let code = support::require_confirmed_mutation(outcome, "invite creation")?;
    host::metrics::invite(host::metrics::InviteEvent::Created);
    // Deliberate operator-facing reveal via `AsRef` (InviteCode has no Display/serde). With a
    // configured base URL, print a ready-to-send invitation link; otherwise the bare code.
    match state.site_config().get_identity().await?.base_url {
        Some(base_url) => {
            let register_url: MailConfirmUrl = tagged_url::compose(&base_url, "/register");
            println!("{register_url}?invite_code={}", code.as_ref());
        }
        None => println!("{}", code.as_ref()),
    }
    Ok(())
}

/// Sends a test email using the configured SMTP settings.
///
/// # Errors
///
/// Returns an error if SMTP is not configured, or if the test email cannot be
/// sent.
pub async fn cmd_smtp_test(storage: &StorageArgs, to: &Email) -> anyhow::Result<()> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime)
        .await
        .context(support::INIT_FIRST_CONTEXT)?;

    smtp_test_with(state.site_config(), to, |config| {
        Ok(Box::new(LettreMailSender::from_config(config)?) as Box<dyn MailSender>)
    })
    .await
}

async fn smtp_test_with(
    site_config: &dyn storage::SiteConfigStorage,
    to: &Email,
    build_smtp: impl FnOnce(&SmtpConfig) -> Result<Box<dyn MailSender>, crate::mailer::BuildMailerError>,
) -> anyhow::Result<()> {
    let smtp_config = storage::load_smtp_config(site_config)
        .await
        .context("SMTP is misconfigured")?
        .ok_or_else(|| anyhow::anyhow!("SMTP is not configured"))?;

    let mailer = build_smtp(&smtp_config).context("failed to build SMTP transport")?;

    let message = EmailMessage {
        from: None,
        to: vec![to.clone()],
        subject: "Jaunder SMTP test".to_owned(),
        body_text:
            "This is a test message from Jaunder. If you received it, SMTP is working correctly."
                .to_owned(),
    };

    mailer
        .send_email(&message)
        .await
        .context("failed to send test email")?;

    println!("Test email sent successfully to {to}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use common::smtp_tls_mode::SmtpTlsMode;
    use common::test_support::{parse_email, parse_invite_ttl_hours};
    use host::config_key::SiteConfigKey;
    use storage::StorageRuntimeConfig;
    use storage::test_support::confirmed;
    use tempfile::TempDir;

    use super::super::test_support::{assert_command_source, sqlite_storage_args};

    fn smtp_config() -> SmtpConfig {
        SmtpConfig {
            host: "mail.example.com".parse().expect("valid host"),
            port: common::smtp_port::SmtpPort::default(),
            tls_mode: SmtpTlsMode::StartTls,
            username: None,
            password: None,
            sender: "Jaunder <noreply@example.com>"
                .parse()
                .expect("valid sender"),
        }
    }

    fn transport_build_error() -> lettre::transport::smtp::Error {
        lettre::transport::smtp::client::TlsParametersBuilder::new("mail.example.com".to_owned())
            .set_min_tls_version(lettre::transport::smtp::client::TlsVersion::Tlsv10)
            .build_rustls()
            .err()
            .expect("rustls rejects TLS 1.0")
    }

    struct FailingMailSender;

    #[async_trait::async_trait]
    impl MailSender for FailingMailSender {
        async fn send_email(
            &self,
            _message: &EmailMessage,
        ) -> Result<(), common::mailer::MailError> {
            Err(common::mailer::MailError::Send(Box::new(
                transport_build_error(),
            )))
        }
    }

    #[tokio::test]
    async fn command_source_chain_smtp_config_read() {
        let mut store = storage::MockSiteConfigStorage::new();
        store.expect_get_smtp_config().return_once(|| {
            Err(sqlx::Error::Io(io::Error::other(
                "injected SMTP config read failure",
            )))
        });

        let error = smtp_test_with(&store, &parse_email("to@example.com"), |_| unreachable!())
            .await
            .unwrap_err();

        assert_command_source::<sqlx::Error>(&error, "SMTP is misconfigured");
    }

    #[tokio::test]
    async fn command_source_chain_smtp_invalid_sender() {
        let mut store = storage::MockSiteConfigStorage::new();
        store
            .expect_get_smtp_config()
            .return_once(|| Ok(Some(smtp_config())));

        let error = smtp_test_with(&store, &parse_email("to@example.com"), |_| {
            let source = "not a mailbox"
                .parse::<lettre::message::Mailbox>()
                .expect_err("invalid mailbox yields lettre address error");
            Err(crate::mailer::BuildMailerError::InvalidSender(source))
        })
        .await
        .unwrap_err();

        assert_command_source::<lettre::address::AddressError>(
            &error,
            "failed to build SMTP transport",
        );
    }

    #[tokio::test]
    async fn command_source_chain_smtp_transport_build() {
        let mut store = storage::MockSiteConfigStorage::new();
        store
            .expect_get_smtp_config()
            .return_once(|| Ok(Some(smtp_config())));

        let error = smtp_test_with(&store, &parse_email("to@example.com"), |_| {
            Err(crate::mailer::BuildMailerError::Transport(
                transport_build_error(),
            ))
        })
        .await
        .unwrap_err();

        assert_command_source::<lettre::transport::smtp::Error>(
            &error,
            "failed to build SMTP transport",
        );
    }

    #[tokio::test]
    async fn command_source_chain_smtp_send() {
        let mut store = storage::MockSiteConfigStorage::new();
        store
            .expect_get_smtp_config()
            .return_once(|| Ok(Some(smtp_config())));

        let error = smtp_test_with(&store, &parse_email("to@example.com"), |_| {
            Ok(Box::new(FailingMailSender) as Box<dyn MailSender>)
        })
        .await
        .unwrap_err();

        assert_command_source::<lettre::transport::smtp::Error>(
            &error,
            "failed to send test email",
        );
    }

    #[tokio::test]
    async fn cmd_user_invite_creates_invite_expiring_in_the_future() {
        let temp = TempDir::new().expect("temp dir");
        let storage_args = sqlite_storage_args(&temp);
        let state = storage::open_database(&storage_args.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");

        let before = common::time::UtcInstant::now();
        cmd_user_invite(&storage_args, Some(parse_invite_ttl_hours("24")))
            .await
            .expect("create invite");

        let invites = state.invites.list_invites().await.expect("list invites");
        assert_eq!(invites.len(), 1, "exactly one invite must be created");
        assert!(
            invites[0].expires_at > before,
            "invite must expire in the future, got: {}",
            invites[0].expires_at
        );
    }

    #[tokio::test]
    async fn cmd_user_invite_with_base_url_configured_prints_link() {
        // Exercises the base-URL branch of the reveal: when a base URL is set, the
        // command prints a ready-to-send invitation link rather than the bare code.
        let temp = TempDir::new().expect("temp dir");
        let storage_args = sqlite_storage_args(&temp);
        let state = storage::open_database(&storage_args.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        let config = Arc::clone(&state.site_config);
        confirmed(
            state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        config
                            .set(
                                transaction,
                                SiteConfigKey::SiteBaseUrl,
                                "https://example.com",
                            )
                            .await
                    })
                })
                .await
                .expect("set base_url"),
        );

        cmd_user_invite(&storage_args, Some(parse_invite_ttl_hours("24")))
            .await
            .expect("create invite");

        let invites = state.invites.list_invites().await.expect("list invites");
        assert_eq!(invites.len(), 1, "exactly one invite must be created");
    }
}
