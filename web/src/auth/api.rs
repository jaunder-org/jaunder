//! The **auth** vertical's API surface (ADR-0070, amended #530): the `#[server]`
//! session endpoints (`get_session`, `login`, `logout`) and their wire types,
//! dual-compiled. `mod.rs` re-exports these so external call sites and the
//! server-fn registrar keep the stable `crate::auth::…` paths.

use crate::error::WebResult;
// `Username` / `SessionLabel` are ordinary typed wire arguments; the password
// secret stays confined to the server boundary (ADR-0063).
use common::{MutationOutcome, session_label::SessionLabel, username::Username};

// One grouped `feature = "server"` support block for the `#[server]` bodies: the
// sibling `server` module's helpers plus the crate-level server-only dependencies.
#[cfg(feature = "server")]
use {
    super::server,
    crate::error::{InternalError, from_write_scope_error},
    host::metrics::{self, LoginOutcome},
    host::password::Password,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{SessionStorage, UserStorage, WriteScope, WriteScopeError},
    tracing::Instrument,
};

#[cfg(feature = "server")]
fn login_write_scope_error(error: WriteScopeError<InternalError>) -> InternalError {
    metrics::login(LoginOutcome::InternalError);
    from_write_scope_error(error)
}

#[cfg(feature = "server")]
fn finalize_login(
    outcome: MutationOutcome<(common::token::RawToken, super::SessionUser)>,
) -> MutationOutcome<super::SessionUser> {
    match outcome {
        MutationOutcome::Confirmed((raw_token, session)) => {
            metrics::login(LoginOutcome::Success);
            server::set_session_cookie(&raw_token);
            leptos_axum::redirect("/");
            MutationOutcome::Confirmed(session)
        }
        MutationOutcome::CommitIndeterminate((raw_token, session)) => {
            server::set_session_cookie(&raw_token);
            MutationOutcome::CommitIndeterminate(session)
        }
    }
}

/// Authenticates a user. Sets the `HttpOnly` `session` cookie and returns the
/// authenticated viewer's [`super::SessionUser`] — deliberately not the session token
/// (#533).
///
/// `label` is a typed wire arg (ADR-0065): the [`SessionLabel`] serde bridge trims it
/// and rejects whitespace-only or over-long values at decode. It has **no client-side
/// `Field<SessionLabel>` on the login form** — that form collects only username and
/// password, so a browser omits `label` entirely and always takes the User-Agent
/// branch below. ADR-0065's client-pre-validation requirement is therefore vacuous
/// here, not violated. (The app-password form, which *does* collect a label, has its
/// `Field::<SessionLabel>` in `crate::sessions::component`.) An omitted *or empty*
/// `label` decodes to `None`: the `Option` form layer absorbs a present-but-empty
/// field before `SessionLabel`'s deserializer runs.
#[macros::server(skip_all)]
pub async fn login(
    username: Username,
    password: common::password::ProfferedPassword,
    label: Option<SessionLabel>,
) -> WebResult<MutationOutcome<super::SessionUser>> {
    let write_scope = expect_context::<WriteScope>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();
    // `username` arrives as a validated typed wire arg. The proffered password is
    // accepted only at this boundary and immediately converted to the serde-free
    // domain secret (ADR-0063).
    let password = Password::try_from(password)?;
    // An explicit client-supplied label arrives already validated (typed wire arg),
    // so it is used as-is; otherwise derive a device name from the User-Agent.
    let session_label = if let Some(label) = label {
        label
    } else {
        let ua = leptos_axum::extract::<axum::http::HeaderMap>()
            .await
            .ok()
            .and_then(|headers| {
                headers
                    .get("user-agent")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        SessionLabel::from_lossy(&ua)
    };
    let authentication = match users
        .prepare_authentication(&username, &password)
        .instrument(tracing::info_span!("web.auth.login.prepare_authentication"))
        .await
    {
        Ok(authentication) => authentication,
        Err(error) => {
            metrics::login(storage::login_outcome(&error));
            return Err(error.into());
        }
    };
    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move {
                let record = users
                    .authenticate(transaction, authentication)
                    .instrument(tracing::info_span!("web.auth.login.authenticate_user"))
                    .await
                    .map_err(InternalError::from)?;
                let raw_token = sessions
                    .create_session(transaction, record.user_id, &session_label)
                    .instrument(tracing::info_span!("web.auth.login.create_session"))
                    .await
                    .map_err(InternalError::storage)?;
                Ok((
                    raw_token,
                    super::SessionUser {
                        username: record.username,
                        is_operator: record.is_operator,
                    },
                ))
            })
        })
        .await
        .map_err(login_write_scope_error)?;

    Ok(finalize_login(outcome))
}

/// Revokes the current session and clears the `session` cookie. Missing or stale
/// cookie-only credentials still clear the cookie; explicit Authorization failures
/// reject without clearing it.
#[macros::server]
pub async fn logout() -> WebResult<MutationOutcome<()>> {
    if let Some(auth) = server::optional_auth().await? {
        let write_scope = expect_context::<WriteScope>();
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        let outcome = write_scope
            .run(|transaction| {
                Box::pin(async move {
                    sessions
                        .revoke_session(transaction, &auth.token_hash)
                        .await
                        .map_err(InternalError::storage)
                })
            })
            .await
            .map_err(from_write_scope_error)?;
        server::clear_session_cookie();
        leptos_axum::redirect("/");
        return Ok(outcome);
    }
    server::clear_session_cookie();
    leptos_axum::redirect("/");
    Ok(MutationOutcome::Confirmed(()))
}

/// The viewer's session identity — username + operator flag — or `None` for
/// missing/stale cookie-only credentials. Explicit Authorization failures reject.
/// The single reconcile fetch behind the shared session context (#591), superseding
/// `current_user` + the reactive `current_user_is_operator`.
#[macros::server]
pub async fn get_session() -> WebResult<Option<super::SessionUser>> {
    let Some(auth) = server::optional_auth().await? else {
        return Ok(None);
    };
    let users = expect_context::<Arc<dyn UserStorage>>();
    let is_operator = users
        .get_user(auth.user_id)
        .await?
        .is_some_and(|u| u.is_operator);
    Ok(Some(super::SessionUser {
        username: auth.username,
        is_operator,
    }))
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{finalize_login, login_write_scope_error};
    use crate::auth::SessionUser;
    use crate::error::{WebError, project};
    use common::{
        MutationOutcome,
        test_support::{parse_raw_token, parse_username},
    };
    use host::error::{ErrorClass, ErrorKind};
    use leptos::prelude::Owner;

    #[test]
    fn login_begin_failure_emits_internal_metric_and_maps_to_a_masked_storage_error() {
        let error =
            login_write_scope_error(storage::WriteScopeError::Begin(sqlx::Error::PoolClosed));

        assert_eq!(error.kind(), ErrorKind::Storage);
        assert_eq!(error.class(), ErrorClass::Bug);
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Storage {
                message: "storage operation failed".to_string(),
            }
        );
    }

    #[test]
    fn finalize_login_preserves_indeterminate_session_and_sets_cookie() {
        Owner::new().with(|| {
            let outcome = finalize_login(MutationOutcome::CommitIndeterminate((
                parse_raw_token("token"),
                SessionUser {
                    username: parse_username("alice"),
                    is_operator: false,
                },
            )));

            assert!(matches!(
                outcome,
                MutationOutcome::CommitIndeterminate(session)
                    if session.username == parse_username("alice") && !session.is_operator
            ));
        });
    }
}
