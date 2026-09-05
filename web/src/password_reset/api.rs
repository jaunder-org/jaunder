//! Password-reset vertical — API surface: the reset `#[server]` endpoints and
//! their wire arg types (ADR-0070). Re-exported from `mod.rs`.

#[cfg(feature = "server")]
use {
    crate::error::{self, InternalError, SwallowedSource},
    crate::mail,
    chrono::Duration,
    common::mailer::{EmailMessage, MailSender},
    common::tagged_url::{self, MailConfirmUrl},
    common::time::UtcInstant,
    host::{
        error::{ErrorClass, ErrorKind},
        metrics::{self, EmailKind, PasswordResetEvent},
        password,
    },
    leptos::prelude::*,
    std::sync::Arc,
    storage::{
        PasswordResetStorage, SessionStorage, SiteConfigStorage, UserStorage, WriteScope,
        account_mutations,
    },
    tracing::instrument::WithSubscriber,
};

use crate::error::WebResult;
// `Username` / `Email` / `ProfferedPassword` / `RawToken` are ungated: they type
// request wire arguments, so generated inputs reference them on both client and
// server builds.
use common::email::Email;
use common::{MutationOutcome, password::ProfferedPassword, token::RawToken, username::Username};
use std::str::FromStr;

/// The validated identifier submitted to begin a password reset.
///
/// Classification is deliberate: `@` commits input to the email parser, so a
/// malformed email-looking identifier cannot be accepted as a username.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum PasswordResetIdentifier {
    Username(Username),
    Email(Email),
}

/// Error returned when a password-reset identifier fails its selected parser.
#[derive(Debug, thiserror::Error)]
pub enum InvalidPasswordResetIdentifier {
    #[error(transparent)]
    Username(#[from] common::username::InvalidUsername),
    #[error(transparent)]
    Email(#[from] common::email::InvalidEmail),
}
impl InvalidPasswordResetIdentifier {
    /// Stable validation feedback safe to return across the wire boundary.
    #[must_use]
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::Username(_) => "username must be non-empty and match [a-z0-9_-]+",
            Self::Email(_) => "invalid email address",
        }
    }

    /// Bounded reason code for decode-failure telemetry.
    #[must_use]
    pub fn telemetry_code(&self) -> &'static str {
        match self {
            Self::Username(_) => "invalid_username",
            Self::Email(_) => "invalid_email",
        }
    }
}

impl FromStr for PasswordResetIdentifier {
    type Err = InvalidPasswordResetIdentifier;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.contains('@') {
            Ok(Self::Email(value.parse()?))
        } else {
            Ok(Self::Username(value.parse()?))
        }
    }
}

impl TryFrom<String> for PasswordResetIdentifier {
    type Error = InvalidPasswordResetIdentifier;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<PasswordResetIdentifier> for String {
    fn from(value: PasswordResetIdentifier) -> Self {
        match value {
            PasswordResetIdentifier::Username(username) => username.into(),
            PasswordResetIdentifier::Email(email) => email.into(),
        }
    }
}

#[cfg(feature = "server")]
fn finalize_password_reset(outcome: &MutationOutcome<common::ids::UserId>) -> MutationOutcome<()> {
    match outcome {
        MutationOutcome::Confirmed(user_id) => {
            tracing::info!(
                credential.kind = "password_reset",
                credential.outcome = "consumed",
                user.id = %user_id,
                "credential consumed"
            );
            metrics::password_reset(PasswordResetEvent::Completed);
            MutationOutcome::Confirmed(())
        }
        MutationOutcome::CommitIndeterminate(_) => MutationOutcome::CommitIndeterminate(()),
    }
}

/// Reports detached reset-request failures without ever rendering an identifier,
/// token, or mail address into exported telemetry.
#[cfg(feature = "server")]
fn report_request_failure(error: &InternalError) {
    error::report_swallowed(
        error.kind(),
        error.class(),
        "web.password_reset.request",
        SwallowedSource::Redacted,
    );
}

/// Reports a detached storage lookup that has no reviewed safe source chain.
#[cfg(feature = "server")]
fn report_request_storage_failure() {
    error::report_swallowed(
        ErrorKind::Storage,
        ErrorClass::Bug,
        "web.password_reset.request",
        SwallowedSource::Redacted,
    );
}

/// Performs the best-effort work accepted by [`request`] after its public response
/// has been returned. Each eligible recipient is isolated so one failed token or
/// mail send does not prevent a shared Email's other verified Users from recovery.
#[cfg(feature = "server")]
async fn deliver_reset_messages(
    identifier: PasswordResetIdentifier,
    users: Arc<dyn UserStorage>,
    write_scope: WriteScope,
    password_resets: Arc<dyn PasswordResetStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
    mailer: Arc<dyn MailSender>,
) {
    let matched_users = match identifier {
        PasswordResetIdentifier::Username(username) => {
            if let Ok(user) = users.get_user_by_username(&username).await {
                user.into_iter().collect()
            } else {
                report_request_storage_failure();
                return;
            }
        }
        PasswordResetIdentifier::Email(email) => {
            if let Ok(users) = users.get_users_by_email(&email).await {
                users
            } else {
                report_request_storage_failure();
                return;
            }
        }
    };
    let recipients: Vec<_> = matched_users
        .into_iter()
        .filter_map(|user| {
            user.email_verified
                .is_verified()
                .then(|| user.email.map(|email| (user.user_id, email)))
                .flatten()
        })
        .collect();
    if recipients.is_empty() {
        return;
    }

    // Resolve configuration only in detached work. A request therefore never
    // turns base-URL availability into an account-enumeration response signal.
    let base_url = match mail::require_base_url(site_config.as_ref()).await {
        Ok(base_url) => base_url,
        Err(error) => {
            report_request_failure(&error);
            return;
        }
    };
    let reset_url: MailConfirmUrl = tagged_url::compose(&base_url, "/reset-password");

    for (user_id, verified_email) in recipients {
        let password_resets = Arc::clone(&password_resets);
        let expires_at = UtcInstant::from(chrono::Utc::now() + Duration::hours(1));
        let outcome = match write_scope
            .run(|transaction| {
                Box::pin(async move {
                    password_resets
                        .create_password_reset(transaction, user_id, expires_at)
                        .await
                        .map_err(InternalError::storage)
                })
            })
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                report_request_failure(&error::from_write_scope_error(error));
                continue;
            }
        };

        // Both outcomes carry a usable token: WriteScope already reported a lost
        // commit acknowledgement, and a duplicate report here would be noise.
        let link = format!("{reset_url}?token={}", outcome.value());
        let message = EmailMessage {
            from: None,
            to: vec![verified_email],
            subject: "Reset your password".to_string(),
            body_text: format!(
                "Click the link below to reset your password:\n\n{link}\n\nThis link expires in 1 hour."
            ),
        };
        if let Err(error) =
            mail::send_recording_metrics(mailer.as_ref(), &message, EmailKind::PasswordReset).await
        {
            report_request_failure(&error);
            continue;
        }
        if matches!(outcome, MutationOutcome::Confirmed(_)) {
            metrics::password_reset(PasswordResetEvent::Requested);
        }
    }
}

#[macros::server(skip_all)]
pub async fn request(identifier: PasswordResetIdentifier) -> WebResult<()> {
    // Context values are cloned before spawning; the detached future cannot
    // retain a request owner, context lookup, or borrowed write transaction.
    let users = expect_context::<Arc<dyn UserStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let password_resets = expect_context::<Arc<dyn PasswordResetStorage>>();
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let mailer = expect_context::<Arc<dyn MailSender>>();

    // Dropping the handle is deliberate: response latency and worker lifetime
    // are independent at this best-effort boundary.
    std::mem::drop(tokio::spawn(
        deliver_reset_messages(
            identifier,
            users,
            write_scope,
            password_resets,
            site_config,
            mailer,
        )
        .with_current_subscriber(),
    ));
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfirmPasswordResetRequest {
    pub token: RawToken,
    pub new_password: ProfferedPassword,
}

#[macros::server(skip_all)]
pub async fn confirm(request: ConfirmPasswordResetRequest) -> WebResult<MutationOutcome<()>> {
    let ConfirmPasswordResetRequest {
        token,
        new_password,
    } = request;
    let write_scope = expect_context::<WriteScope>();
    let password_resets = expect_context::<Arc<dyn PasswordResetStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();

    // `new_password` is the inbound-secret twin (ADR-0063); convert into the
    // serde-free domain `Password` at the boundary. `token` is a `RawToken` wire
    // arg — its serde bridge already rejected a malformed shape on decode.
    let password = password::Password::try_from(new_password)?;

    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move {
                account_mutations::confirm_password_reset(
                    transaction,
                    password_resets.as_ref(),
                    users.as_ref(),
                    sessions.as_ref(),
                    &token,
                    &password,
                )
                .await
                .map_err(Into::into)
            })
        })
        .await
        .map_err(error::from_write_scope_error)?;
    Ok(finalize_password_reset(&outcome))
}

#[cfg(test)]
mod tests {
    use super::{InvalidPasswordResetIdentifier, PasswordResetIdentifier};
    use common::{email::Email, username::Username};

    #[test]
    fn password_reset_identifier_classifies_and_canonicalizes_input() {
        assert_eq!(
            "Alice".parse::<PasswordResetIdentifier>().unwrap(),
            PasswordResetIdentifier::Username("alice".parse::<Username>().unwrap())
        );
        assert_eq!(
            "Local.Part@EXAMPLE.COM"
                .parse::<PasswordResetIdentifier>()
                .unwrap(),
            PasswordResetIdentifier::Email("Local.Part@example.com".parse::<Email>().unwrap())
        );
    }

    #[test]
    fn password_reset_identifier_rejects_the_selected_invalid_variant() {
        assert!(matches!(
            "alice@".parse::<PasswordResetIdentifier>(),
            Err(InvalidPasswordResetIdentifier::Email(_))
        ));
        assert!(matches!(
            "alice.example".parse::<PasswordResetIdentifier>(),
            Err(InvalidPasswordResetIdentifier::Username(_))
        ));
    }

    #[cfg(feature = "server")]
    #[test]
    fn password_reset_indeterminate_outcome_preserves_uncertainty_and_erases_consumption() {
        let outcome = super::finalize_password_reset(
            &common::MutationOutcome::CommitIndeterminate(common::ids::UserId::from(42)),
        );

        assert!(matches!(
            outcome,
            common::MutationOutcome::CommitIndeterminate(())
        ));
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    use super::{PasswordResetIdentifier, request};
    use async_trait::async_trait;
    use common::{
        ids::UserId,
        mailer::{EmailMessage, MailError, MailSender},
        site::SiteIdentity,
        test_support::{parse_email, parse_raw_token, parse_site_title, parse_url, parse_username},
        time::UtcInstant,
    };
    use leptos::prelude::{Owner, provide_context};
    use std::sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    };
    use storage::{
        EmailVerified, MockPasswordResetStorage, MockSiteConfigStorage, MockUserStorage,
        PasswordResetStorage, SiteConfigStorage, UserRecord, UserStorage,
        test_support::mock_write_scope_with_commit_acknowledgement_loss,
    };

    struct TerminalMailer {
        terminal: Sender<()>,
    }

    #[async_trait]
    impl MailSender for TerminalMailer {
        async fn send_email(&self, _: &EmailMessage) -> Result<(), MailError> {
            assert!(
                self.terminal.send(()).is_ok(),
                "test waits for worker terminal"
            );
            Ok(())
        }
    }

    fn verified_user() -> UserRecord {
        UserRecord {
            user_id: UserId::from(1),
            username: parse_username("alice"),
            display_name: None,
            bio: None,
            created_at: UtcInstant::now(),
            last_authenticated_at: None,
            email: Some(parse_email("alice@example.com")),
            email_verified: EmailVerified::VERIFIED,
            is_operator: storage::OperatorStatus::STANDARD,
        }
    }
    fn site_config() -> MockSiteConfigStorage {
        let mut site_config = MockSiteConfigStorage::new();
        site_config.expect_get_identity().returning(|| {
            Ok(SiteIdentity {
                title: parse_site_title("Jaunder"),
                base_url: Some(parse_url("https://example.com/")),
            })
        });
        site_config
    }

    // The lookup blocks a Tokio worker after entering. The direct server-function
    // seam must nevertheless resolve before account-dependent work is released.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn request_returns_before_gated_lookup_and_mails_indeterminate_token_after_release() {
        let owner = Owner::new();
        owner.set();
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx): (Sender<()>, Receiver<()>) = channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let mut users = MockUserStorage::new();
        users.expect_get_user_by_username().return_once(move |_| {
            entered_tx.send(()).expect("worker reports lookup entry");
            release_rx
                .lock()
                .expect("release mutex")
                .recv()
                .expect("test releases lookup");
            Ok(Some(verified_user()))
        });
        provide_context(Arc::new(users) as Arc<dyn UserStorage>);
        provide_context(mock_write_scope_with_commit_acknowledgement_loss());
        let mut password_resets = MockPasswordResetStorage::new();
        password_resets
            .expect_create_password_reset()
            .return_once(|_, _, _| Ok(parse_raw_token("token")));
        provide_context(Arc::new(password_resets) as Arc<dyn PasswordResetStorage>);
        provide_context(Arc::new(site_config()) as Arc<dyn SiteConfigStorage>);
        let (terminal_tx, terminal_rx) = channel();
        provide_context(Arc::new(TerminalMailer {
            terminal: terminal_tx,
        }) as Arc<dyn MailSender>);

        assert_eq!(
            request(PasswordResetIdentifier::Username(parse_username("alice")))
                .await
                .expect("valid request is accepted"),
            ()
        );
        entered_rx
            .recv()
            .expect("lookup entered while request was complete");
        release_tx.send(()).expect("release detached lookup");
        terminal_rx.recv().expect("worker terminal after release");
        drop(owner);
    }
}
