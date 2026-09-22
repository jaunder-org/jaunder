//! Cross-store account mutations.
//!
//! This module owns the transaction-local orchestration for account flows that
//! span storage traits. Callers own the [`WriteTransaction`] through
//! [`WriteScope`](crate::WriteScope); these functions neither begin nor commit a
//! transaction. Their dependencies are the exact object-safe stores needed by
//! each flow so the composition root does not leak into application code.

use common::{
    content_license::ContentLicense,
    display_name::DisplayName,
    ids::UserId,
    session_label::SessionLabel,
    token::{RawToken, TokenHash},
    username::Username,
};
use host::{
    config_key::UserConfigKey, feed, invite::InviteCode, passkey::Credential, password::Password,
};
use thiserror::Error;

use crate::users;
use crate::{
    CreateUserError, FeedEventError, FeedEventStorage, InviteStorage, OperatorStatus,
    PasskeyCredentialId, PasskeyStorage, PasswordResetStorage, PostStorage, ProfileUpdate,
    SessionStorage, UserConfigStorage, UserStorage, WriteTransaction,
};

/// Errors returned by the cross-store Passkey mutation primitives.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct PasskeyMutationError(#[from] pub sqlx::Error);

/// Enqueues every feed affected by a changed User-level public projection.
async fn enqueue_user_feed_events(
    transaction: &mut WriteTransaction,
    posts: &dyn PostStorage,
    feed_events: &dyn FeedEventStorage,
    user_id: UserId,
    username: &Username,
    now: common::time::UtcInstant,
) -> Result<(), sqlx::Error> {
    let Some(tags) = posts
        .feed_affecting_post_tags_for_user(transaction, user_id, now)
        .await?
    else {
        return Ok(());
    };
    let paths = feed::affected_feed_urls(username, tags.0.iter());
    feed_events
        .enqueue_many(transaction, &paths)
        .await
        .map_err(|error| match error {
            FeedEventError::Db(error) => error,
        })
}

/// Updates a profile and atomically enqueues events when its Display Name changes.
///
/// # Errors
///
/// Returns an error if the locked User read, profile update, affected-Post read,
/// or feed-event enqueue fails. The caller's write scope then rolls back the mutation.
pub async fn update_profile_with_feed_events(
    transaction: &mut WriteTransaction,
    users: &dyn UserStorage,
    posts: &dyn PostStorage,
    feed_events: &dyn FeedEventStorage,
    user_id: UserId,
    update: &ProfileUpdate<'_>,
    now: common::time::UtcInstant,
) -> Result<(), sqlx::Error> {
    let user = users
        .get_user_for_update(transaction, user_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;
    let changed = user.display_name != update.display_name.cloned();
    users.update_profile(transaction, user_id, update).await?;
    if changed {
        enqueue_user_feed_events(
            transaction,
            posts,
            feed_events,
            user_id,
            &user.username,
            now,
        )
        .await?;
    }
    Ok(())
}

/// One requested User-wide Content License mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentLicenseUpdate {
    pub user_id: UserId,
    pub license: ContentLicense,
}

/// Updates Content License and atomically enqueues events when it semantically changes.
///
/// # Errors
///
/// Returns an error if the locked User/configuration read, configuration update,
/// affected-Post read, or feed-event enqueue fails. The caller's write scope then
/// rolls back the mutation.
pub async fn update_content_license_with_feed_events(
    transaction: &mut WriteTransaction,
    users: &dyn UserStorage,
    user_config: &dyn UserConfigStorage,
    posts: &dyn PostStorage,
    feed_events: &dyn FeedEventStorage,
    update: ContentLicenseUpdate,
    now: common::time::UtcInstant,
) -> Result<(), sqlx::Error> {
    let user = users
        .get_user_for_update(transaction, update.user_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;
    let current = user_config
        .get_content_license_for_update(transaction, update.user_id)
        .await?;
    user_config
        .set(
            transaction,
            update.user_id,
            UserConfigKey::ContentLicense,
            update.license.as_ref(),
        )
        .await?;
    if current != update.license {
        enqueue_user_feed_events(
            transaction,
            posts,
            feed_events,
            update.user_id,
            &user.username,
            now,
        )
        .await?;
    }
    Ok(())
}

/// Applies a verified Passkey assertion, records credential use, and creates its
/// ordinary `Session` in the same caller-owned transaction.
///
/// The passed credential is a trusted adapter result: it has already folded
/// backup flags and retained the counter high-water mark. A missing row is an
/// error so no `Session` can be minted for a deleted credential.
///
/// # Errors
///
/// Returns an error if the credential update or `Session` creation fails. The
/// caller's write scope then rolls back all mutations.
pub async fn finalize_passkey_authentication(
    transaction: &mut WriteTransaction,
    passkeys: &dyn PasskeyStorage,
    sessions: &dyn SessionStorage,
    user_id: UserId,
    credential: &Credential,
    session_label: &SessionLabel,
) -> Result<RawToken, PasskeyMutationError> {
    if !passkeys
        .update_credential_after_authentication(transaction, user_id, credential)
        .await?
    {
        return Err(PasskeyMutationError(sqlx::Error::RowNotFound));
    }
    sessions
        .create_session(transaction, user_id, session_label)
        .await
        .map_err(PasskeyMutationError)
}

/// Deletes one owned Passkey and revokes every other `Session` for its User while
/// retaining the cookie `Session` that authorized the mutation.
///
/// # Errors
///
/// Returns an error if the credential deletion or `Session` revocation fails.
/// The caller's write scope then rolls back both mutations.
pub async fn delete_passkey_and_revoke_other_sessions(
    transaction: &mut WriteTransaction,
    passkeys: &dyn PasskeyStorage,
    sessions: &dyn SessionStorage,
    user_id: UserId,
    credential_id: &PasskeyCredentialId,
    current_session: &TokenHash,
) -> Result<bool, PasskeyMutationError> {
    let deleted = passkeys
        .delete_credential(transaction, user_id, credential_id)
        .await?;
    if deleted {
        sessions
            .revoke_all_for_user_except(transaction, user_id, current_session)
            .await?;
    }
    Ok(deleted)
}

/// Errors returned by [`register_with_invite`].
#[derive(Debug, Error)]
pub enum RegisterWithInviteError {
    /// The provided invite code does not exist.
    #[error("invite code not found")]
    InviteNotFound,
    /// The provided invite code has expired.
    #[error("invite code has expired")]
    InviteExpired,
    /// The provided invite code has already been consumed.
    #[error("invite code has already been used")]
    InviteAlreadyUsed,
    /// The requested username is already in use.
    #[error("username is already taken")]
    UsernameTaken,
    /// An unexpected storage or password-preparation error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<RegisterWithInviteError> for host::error::InternalError {
    fn from(error: RegisterWithInviteError) -> Self {
        use host::error::InternalError;
        match error {
            RegisterWithInviteError::UsernameTaken => {
                InternalError::conflict("username is already taken")
            }
            RegisterWithInviteError::InviteNotFound => {
                InternalError::validation("invite code not found")
            }
            RegisterWithInviteError::InviteExpired => {
                InternalError::validation("invite code has expired")
            }
            RegisterWithInviteError::InviteAlreadyUsed => {
                InternalError::validation("invite code has already been used")
            }
            RegisterWithInviteError::Internal(error) => InternalError::storage(error),
        }
    }
}

impl From<crate::UseInviteError> for RegisterWithInviteError {
    fn from(error: crate::UseInviteError) -> Self {
        match error {
            crate::UseInviteError::NotFound => Self::InviteNotFound,
            crate::UseInviteError::Expired => Self::InviteExpired,
            crate::UseInviteError::AlreadyUsed => Self::InviteAlreadyUsed,
            crate::UseInviteError::Internal(error) => Self::Internal(error),
        }
    }
}

impl From<CreateUserError> for RegisterWithInviteError {
    fn from(error: CreateUserError) -> Self {
        match error {
            CreateUserError::UsernameTaken => Self::UsernameTaken,
            CreateUserError::Internal(error) => Self::Internal(error),
        }
    }
}

/// Errors returned by [`confirm_password_reset`].
#[derive(Debug, Error)]
pub enum ConfirmPasswordResetError {
    /// The reset token does not exist.
    #[error("token not found")]
    NotFound,
    /// The reset token has expired.
    #[error("token has expired")]
    Expired,
    /// The reset token has already been consumed.
    #[error("token has already been used")]
    AlreadyUsed,
    /// An unexpected storage or password-preparation error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<ConfirmPasswordResetError> for host::error::InternalError {
    fn from(error: ConfirmPasswordResetError) -> Self {
        use host::error::InternalError;
        match error {
            ConfirmPasswordResetError::NotFound => InternalError::validation("token not found"),
            ConfirmPasswordResetError::Expired => InternalError::validation("token has expired"),
            ConfirmPasswordResetError::AlreadyUsed => {
                InternalError::validation("token has already been used")
            }
            ConfirmPasswordResetError::Internal(error) => InternalError::storage(error),
        }
    }
}

impl From<crate::UsePasswordResetError> for ConfirmPasswordResetError {
    fn from(error: crate::UsePasswordResetError) -> Self {
        match error {
            crate::UsePasswordResetError::NotFound => Self::NotFound,
            crate::UsePasswordResetError::Expired => Self::Expired,
            crate::UsePasswordResetError::AlreadyUsed => Self::AlreadyUsed,
            crate::UsePasswordResetError::Internal(error) => Self::Internal(error),
        }
    }
}

/// Values required by [`register_with_invite`].
///
/// The caller owns all referenced values for the duration of the registration
/// operation. Storage dependencies remain function parameters so this carrier
/// describes only the registration request.
pub struct RegisterWithInviteInput<'a> {
    /// Username assigned to the newly created user.
    pub username: &'a Username,
    /// Plaintext password to prepare before creating the user.
    pub password: &'a Password,
    /// Optional display name assigned to the newly created user.
    pub display_name: Option<&'a DisplayName>,
    /// Privilege state assigned to the newly created user.
    pub is_operator: OperatorStatus,
    /// Capability that authorizes the registration.
    pub invite_code: &'a InviteCode,
}

/// Creates a user and attributes a still-valid invite to that user.
///
/// The validity precheck intentionally happens before Argon2. The claim is a
/// conditional write after user insertion, so a concurrent claimant loses with
/// `InviteAlreadyUsed`; returning the error lets the caller-owned scope roll
/// the inserted user back.
///
/// # Errors
///
/// Returns [`RegisterWithInviteError::InviteNotFound`] when the invite code is
/// unknown, [`RegisterWithInviteError::InviteExpired`] when it has expired, or
/// [`RegisterWithInviteError::InviteAlreadyUsed`] when another user has claimed
/// it. Returns [`RegisterWithInviteError::UsernameTaken`] when the username is
/// already registered, and [`RegisterWithInviteError::Internal`] if password
/// preparation or storage fails.
#[tracing::instrument(
    name = "storage.account_mutations.register_with_invite",
    skip(transaction, users, invites, input),
    fields(username = %input.username)
)]
pub async fn register_with_invite(
    transaction: &mut WriteTransaction,
    users: &dyn UserStorage,
    invites: &dyn InviteStorage,
    input: RegisterWithInviteInput<'_>,
) -> Result<UserId, RegisterWithInviteError> {
    invites
        .precheck_invite(input.invite_code)
        .await
        .map_err(RegisterWithInviteError::from)?;

    // An invite is a high-entropy capability. After its read-only precheck,
    // Argon2 may run before the transactional user insertion.
    let prepared_password = users::prepare_password(input.password.clone())
        .await
        .map_err(|error| RegisterWithInviteError::Internal(sqlx::Error::Io(error)))?;
    let user_id = users
        .create_user(
            transaction,
            input.username,
            &prepared_password,
            input.display_name,
            input.is_operator,
        )
        .await
        .map_err(RegisterWithInviteError::from)?;
    invites
        .claim_invite(transaction, input.invite_code, user_id)
        .await
        .map_err(RegisterWithInviteError::from)?;
    Ok(user_id)
}

/// Consumes a reset token, replaces its user's password, and revokes every
/// session belonging to that user in the caller-owned transaction.
///
/// # Errors
///
/// Returns [`ConfirmPasswordResetError::NotFound`] when the token is unknown,
/// [`ConfirmPasswordResetError::Expired`] when it has expired, or
/// [`ConfirmPasswordResetError::AlreadyUsed`] when it was previously consumed.
/// Returns [`ConfirmPasswordResetError::Internal`] if password preparation or a
/// storage mutation fails.
#[tracing::instrument(
    name = "storage.account_mutations.confirm_password_reset",
    skip(transaction, password_resets, users, sessions, raw_token, new_password)
)]
pub async fn confirm_password_reset(
    transaction: &mut WriteTransaction,
    password_resets: &dyn PasswordResetStorage,
    users: &dyn UserStorage,
    sessions: &dyn SessionStorage,
    raw_token: &RawToken,
    new_password: &Password,
) -> Result<UserId, ConfirmPasswordResetError> {
    let user_id = password_resets
        .use_password_reset(transaction, raw_token)
        .await
        .map_err(ConfirmPasswordResetError::from)?;

    // Reset tokens are high-entropy capabilities: only a successful claim
    // reaches password preparation and the following mutations.
    let prepared_password = users::prepare_password(new_password.clone())
        .await
        .map_err(|error| ConfirmPasswordResetError::Internal(sqlx::Error::Io(error)))?;
    users
        .set_password(transaction, user_id, &prepared_password)
        .await
        .map_err(ConfirmPasswordResetError::Internal)?;
    sessions
        .revoke_all_for_user(transaction, user_id)
        .await
        .map_err(ConfirmPasswordResetError::Internal)?;
    Ok(user_id)
}

#[cfg(test)]
mod tests {
    use crate::test_support::{Backend, SeedUser, backends, confirmed};
    use common::test_support::parse_session_label;
    use host::token;
    use rstest::*;
    use rstest_reuse::*;

    use super::*;
    use host::error::{ErrorKind, InternalError};

    #[test]
    fn registration_errors_map_to_their_public_error_kinds() {
        for error in [
            RegisterWithInviteError::InviteNotFound,
            RegisterWithInviteError::InviteExpired,
            RegisterWithInviteError::InviteAlreadyUsed,
        ] {
            let mapped: InternalError = error.into();
            assert_eq!(mapped.kind(), ErrorKind::Validation);
        }

        let mapped: InternalError = RegisterWithInviteError::UsernameTaken.into();
        assert_eq!(mapped.kind(), ErrorKind::Conflict);

        let mapped: InternalError =
            RegisterWithInviteError::Internal(sqlx::Error::RowNotFound).into();
        assert_eq!(mapped.kind(), ErrorKind::Storage);
    }

    #[test]
    fn subordinate_internal_errors_remain_internal() {
        assert!(matches!(
            RegisterWithInviteError::from(crate::UseInviteError::Internal(
                sqlx::Error::RowNotFound
            )),
            RegisterWithInviteError::Internal(sqlx::Error::RowNotFound)
        ));
        assert!(matches!(
            RegisterWithInviteError::from(CreateUserError::Internal(sqlx::Error::RowNotFound)),
            RegisterWithInviteError::Internal(sqlx::Error::RowNotFound)
        ));
        assert!(matches!(
            ConfirmPasswordResetError::from(crate::UsePasswordResetError::Internal(
                sqlx::Error::RowNotFound
            )),
            ConfirmPasswordResetError::Internal(sqlx::Error::RowNotFound)
        ));
    }

    #[test]
    fn create_user_conflict_remains_a_registration_conflict() {
        assert!(matches!(
            RegisterWithInviteError::from(CreateUserError::UsernameTaken),
            RegisterWithInviteError::UsernameTaken
        ));
    }

    #[test]
    fn reset_token_state_errors_map_to_client_validation() {
        for error in [
            ConfirmPasswordResetError::NotFound,
            ConfirmPasswordResetError::Expired,
            ConfirmPasswordResetError::AlreadyUsed,
        ] {
            let mapped: InternalError = error.into();
            assert_eq!(mapped.kind(), ErrorKind::Validation);
        }

        let mapped: InternalError =
            ConfirmPasswordResetError::Internal(sqlx::Error::RowNotFound).into();
        assert_eq!(mapped.kind(), ErrorKind::Storage);
    }
    #[derive(Clone, Copy)]
    enum PasskeyMutationFault {
        CredentialUpdate,
        SessionCreation,
        CredentialDeletion,
        OtherSessionRevocation,
    }

    async fn install_passkey_mutation_fault(
        pool: &crate::test_support::CloseablePool,
        fault: PasskeyMutationFault,
    ) {
        let (sqlite_trigger, postgres_function, postgres_trigger) = match fault {
            PasskeyMutationFault::CredentialUpdate => (
                "CREATE TRIGGER passkey_mutation_fault BEFORE UPDATE ON passkey_credentials \
                 BEGIN SELECT RAISE(ABORT, 'test-injected passkey mutation fault'); END",
                "CREATE FUNCTION passkey_mutation_fault() RETURNS trigger LANGUAGE plpgsql AS \
                 $$ BEGIN RAISE EXCEPTION 'test-injected passkey mutation fault'; END; $$",
                "CREATE TRIGGER passkey_mutation_fault BEFORE UPDATE ON passkey_credentials \
                 FOR EACH ROW EXECUTE FUNCTION passkey_mutation_fault()",
            ),
            PasskeyMutationFault::SessionCreation => (
                "CREATE TRIGGER passkey_mutation_fault BEFORE INSERT ON sessions \
                 BEGIN SELECT RAISE(ABORT, 'test-injected passkey mutation fault'); END",
                "CREATE FUNCTION passkey_mutation_fault() RETURNS trigger LANGUAGE plpgsql AS \
                 $$ BEGIN RAISE EXCEPTION 'test-injected passkey mutation fault'; END; $$",
                "CREATE TRIGGER passkey_mutation_fault BEFORE INSERT ON sessions \
                 FOR EACH ROW EXECUTE FUNCTION passkey_mutation_fault()",
            ),
            PasskeyMutationFault::CredentialDeletion => (
                "CREATE TRIGGER passkey_mutation_fault BEFORE DELETE ON passkey_credentials \
                 BEGIN SELECT RAISE(ABORT, 'test-injected passkey mutation fault'); END",
                "CREATE FUNCTION passkey_mutation_fault() RETURNS trigger LANGUAGE plpgsql AS \
                 $$ BEGIN RAISE EXCEPTION 'test-injected passkey mutation fault'; END; $$",
                "CREATE TRIGGER passkey_mutation_fault BEFORE DELETE ON passkey_credentials \
                 FOR EACH ROW EXECUTE FUNCTION passkey_mutation_fault()",
            ),
            PasskeyMutationFault::OtherSessionRevocation => (
                "CREATE TRIGGER passkey_mutation_fault BEFORE DELETE ON sessions \
                 BEGIN SELECT RAISE(ABORT, 'test-injected passkey mutation fault'); END",
                "CREATE FUNCTION passkey_mutation_fault() RETURNS trigger LANGUAGE plpgsql AS \
                 $$ BEGIN RAISE EXCEPTION 'test-injected passkey mutation fault'; END; $$",
                "CREATE TRIGGER passkey_mutation_fault BEFORE DELETE ON sessions \
                 FOR EACH ROW EXECUTE FUNCTION passkey_mutation_fault()",
            ),
        };

        crate::with_closeable_pool!(pool, backend_pool, {
            if matches!(pool, crate::test_support::CloseablePool::Sqlite(_)) {
                sqlx::query(sqlite_trigger)
                    .execute(backend_pool)
                    .await
                    .unwrap();
            } else {
                // cov:ignore-start: The backend-parametric PostgreSQL fault test executes this macro-expanded branch, but LLVM source coverage does not attribute that monomorphized execution to these shared source lines.
                sqlx::query(postgres_function)
                    .execute(backend_pool)
                    .await
                    .unwrap();
                sqlx::query(postgres_trigger)
                    .execute(backend_pool)
                    .await
                    .unwrap();
                // cov:ignore-stop
            }
        });
    }

    async fn seed_passkey(
        env: &crate::test_support::TestEnv,
    ) -> (UserId, Credential, PasskeyCredentialId) {
        let user_id = SeedUser::new()
            .seed(env.users(), env.write_scope())
            .await
            .user_id;
        let credential = crate::test_support::passkey_credential_fixture();
        let credential_id = PasskeyCredentialId::from_credential(&credential);
        let stored_credential = credential.clone();
        let label = "Laptop".parse().unwrap();
        let passkeys = env.passkeys();
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .insert_credential(transaction, user_id, &label, &stored_credential)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        (user_id, credential, credential_id)
    }

    async fn create_session(
        env: &crate::test_support::TestEnv,
        user_id: UserId,
        label: &str,
    ) -> common::token::RawToken {
        let label = parse_session_label(label);
        let sessions = env.sessions();
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(
                        async move { sessions.create_session(transaction, user_id, &label).await },
                    )
                })
                .await
                .unwrap(),
        )
    }

    #[apply(backends)]
    #[tokio::test]
    async fn passkey_authentication_missing_credential_for_user_mints_no_session(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let (_, credential, _) = seed_passkey(&env).await;
        let absent_user = SeedUser::new()
            .seed(env.users(), env.write_scope())
            .await
            .user_id;
        let passkeys = env.passkeys();
        let sessions = env.sessions();
        let label = parse_session_label("Passkey test");
        let error = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    finalize_passkey_authentication(
                        transaction,
                        passkeys.as_ref(),
                        sessions.as_ref(),
                        absent_user,
                        &credential,
                        &label,
                    )
                    .await
                })
            })
            .await
            .expect_err("an absent credential/user pairing is rejected");

        assert!(matches!(
            error,
            crate::WriteScopeError::Operation(PasskeyMutationError(sqlx::Error::RowNotFound))
        ));
        assert!(
            env.sessions()
                .list_sessions(absent_user)
                .await
                .unwrap()
                .is_empty(),
            "a failed credential update must not mint a Session"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn passkey_authentication_credential_update_failure_leaves_no_session_or_update(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let (user_id, credential, credential_id) = seed_passkey(&env).await;
        install_passkey_mutation_fault(env.base.pool(), PasskeyMutationFault::CredentialUpdate)
            .await;

        let passkeys = env.passkeys();
        let sessions = env.sessions();
        let label = parse_session_label("Passkey test");
        assert!(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        finalize_passkey_authentication(
                            transaction,
                            passkeys.as_ref(),
                            sessions.as_ref(),
                            user_id,
                            &credential,
                            &label,
                        )
                        .await
                    })
                })
                .await
                .is_err()
        );

        assert_eq!(
            env.sessions().list_sessions(user_id).await.unwrap().len(),
            0
        );
        assert_eq!(
            env.passkeys()
                .credential_for_user(user_id, &credential_id)
                .await
                .unwrap()
                .unwrap()
                .last_used_at,
            None,
            "a failed credential update cannot leave authentication metadata behind"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn passkey_authentication_session_creation_failure_rolls_back_credential_update(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let (user_id, credential, credential_id) = seed_passkey(&env).await;
        install_passkey_mutation_fault(env.base.pool(), PasskeyMutationFault::SessionCreation)
            .await;

        let passkeys = env.passkeys();
        let sessions = env.sessions();
        let label = parse_session_label("Passkey test");
        assert!(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        finalize_passkey_authentication(
                            transaction,
                            passkeys.as_ref(),
                            sessions.as_ref(),
                            user_id,
                            &credential,
                            &label,
                        )
                        .await
                    })
                })
                .await
                .is_err()
        );

        assert_eq!(
            env.sessions().list_sessions(user_id).await.unwrap().len(),
            0
        );
        assert_eq!(
            env.passkeys()
                .credential_for_user(user_id, &credential_id)
                .await
                .unwrap()
                .unwrap()
                .last_used_at,
            None,
            "the completed credential update rolls back when Session creation fails"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn passkey_authentication_commit_acknowledgement_loss_preserves_both_mutations(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let (user_id, credential, credential_id) = seed_passkey(&env).await;
        let scope = env
            .write_scope()
            .with_commit_acknowledgement_loss_after_commit_for_test();
        let passkeys = env.passkeys();
        let sessions = env.sessions();
        let label = parse_session_label("Passkey test");

        let outcome = scope
            .run(move |transaction| {
                Box::pin(async move {
                    finalize_passkey_authentication(
                        transaction,
                        passkeys.as_ref(),
                        sessions.as_ref(),
                        user_id,
                        &credential,
                        &label,
                    )
                    .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            common::MutationOutcome::CommitIndeterminate(_)
        ));
        assert_eq!(
            env.sessions().list_sessions(user_id).await.unwrap().len(),
            1
        );
        assert!(
            env.passkeys()
                .credential_for_user(user_id, &credential_id)
                .await
                .unwrap()
                .unwrap()
                .last_used_at
                .is_some(),
            "the completed credential update remains durable despite acknowledgement loss"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn passkey_deletion_failure_leaves_credential_and_sessions_intact(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let (user_id, _, credential_id) = seed_passkey(&env).await;
        let current = create_session(&env, user_id, "Current browser").await;
        create_session(&env, user_id, "Other browser").await;
        install_passkey_mutation_fault(env.base.pool(), PasskeyMutationFault::CredentialDeletion)
            .await;
        let current_hash = token::hash(&current).unwrap();
        let preserved_current_hash = current_hash.clone();
        let preserved_credential_id = credential_id.clone();
        let passkeys = env.passkeys();
        let sessions = env.sessions();

        assert!(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        delete_passkey_and_revoke_other_sessions(
                            transaction,
                            passkeys.as_ref(),
                            sessions.as_ref(),
                            user_id,
                            &credential_id,
                            &current_hash,
                        )
                        .await
                    })
                })
                .await
                .is_err()
        );
        assert!(
            env.passkeys()
                .credential_for_user(user_id, &preserved_credential_id)
                .await
                .unwrap()
                .is_some()
        );
        let sessions = env.sessions().list_sessions(user_id).await.unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(
            sessions
                .iter()
                .any(|session| session.token_hash == preserved_current_hash),
            "the authorizing Session remains active"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn passkey_session_revocation_failure_rolls_back_credential_deletion(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let (user_id, _, credential_id) = seed_passkey(&env).await;
        let current = create_session(&env, user_id, "Current browser").await;
        create_session(&env, user_id, "Other browser").await;
        install_passkey_mutation_fault(
            env.base.pool(),
            PasskeyMutationFault::OtherSessionRevocation,
        )
        .await;
        let current_hash = token::hash(&current).unwrap();
        let preserved_current_hash = current_hash.clone();
        let preserved_credential_id = credential_id.clone();
        let passkeys = env.passkeys();
        let sessions = env.sessions();

        assert!(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        delete_passkey_and_revoke_other_sessions(
                            transaction,
                            passkeys.as_ref(),
                            sessions.as_ref(),
                            user_id,
                            &credential_id,
                            &current_hash,
                        )
                        .await
                    })
                })
                .await
                .is_err()
        );
        assert!(
            env.passkeys()
                .credential_for_user(user_id, &preserved_credential_id)
                .await
                .unwrap()
                .is_some(),
            "the deletion before revocation is rolled back"
        );
        let sessions = env.sessions().list_sessions(user_id).await.unwrap();
        assert_eq!(
            sessions.len(),
            2,
            "the other Session remains active after failed revocation"
        );
        assert!(
            sessions
                .iter()
                .any(|session| session.token_hash == preserved_current_hash),
            "the authorizing Session remains active"
        );
    }
}
