//! The **auth** vertical's API surface (ADR-0070, amended #530): the `#[server]`
//! session endpoints (`get_session`, `login`, `logout`) and their wire types,
//! dual-compiled. `mod.rs` re-exports these so external call sites and the
//! server-fn registrar keep the stable `crate::auth::…` paths.

use crate::error::WebResult;
// `Username` / `ProfferedPassword` / `SessionLabel` are ungated: they are wire-arg
// types of `login`, so the `#[server]`-generated arg struct references them on both
// the client and server builds. `RawToken` is deliberately *not* here: the session
// token does not cross the wire (#533), so it is a server-only value that the
// `#[server]` body infers from `create_session`. The rule is recorded in
// docs/adr/0107-web-session-establishment-is-cookie-only.md.
use common::password::ProfferedPassword;
use common::session_label::SessionLabel;
use common::username::Username;

// One grouped `feature = "server"` support block for the `#[server]` bodies: the
// sibling `server` module's helpers plus the crate-level server-only dependencies.
#[cfg(feature = "server")]
use {
    super::server::{clear_session_cookie, optional_auth, set_session_cookie},
    host::password::Password,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{SessionStorage, UserStorage},
    tracing::Instrument,
};

/// Caller-supplied credentials and optional device label for one login attempt.
///
/// Keeping the cohesive request together makes username/password transposition a
/// compile error and lets the CSR form dispatch values it has already parsed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoginRequest {
    pub username: Username,
    pub password: ProfferedPassword,
    pub label: Option<SessionLabel>,
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
pub async fn login(request: LoginRequest) -> WebResult<super::SessionUser> {
    let LoginRequest {
        username,
        password,
        label,
    } = request;
    let users = expect_context::<Arc<dyn UserStorage>>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();
    // `username` / `password` arrive already validated: typed wire args whose serde
    // bridge routes through their validating `FromStr`, client-pre-validated via
    // `<ValidatedInput<_>>` (ADR-0065). `ProfferedPassword` is the inbound-secret
    // twin of the serde-free `Password` (ADR-0063); convert into it here.
    let password = Password::try_from(password)?;
    let record = match users
        .authenticate(&username, &password)
        .instrument(tracing::info_span!("web.auth.login.authenticate_user"))
        .await
    {
        Ok(record) => {
            host::metrics::login(host::metrics::LoginOutcome::Success);
            record
        }
        Err(error) => {
            host::metrics::login(storage::login_outcome(&error));
            return Err(error.into());
        }
    };

    // An explicit client-supplied label arrives already validated (typed wire arg),
    // so it is used as-is; otherwise derive a device name from the User-Agent.
    let session_label = if let Some(label) = label {
        label
    } else {
        // The User-Agent is an internally derived hint, not submitted input, so it
        // goes through the lossy door (ADR-0063 §2): `from_lossy` trims, bounds it at
        // MAX_SESSION_LABEL_CHARS, and supplies its own fallback label when there is
        // no usable header. Both the cap and the default live in `SessionLabel` —
        // never duplicated here, not even as a comment that could go stale.
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

    let raw_token = sessions
        .create_session(record.user_id, &session_label)
        .instrument(tracing::info_span!("web.auth.login.create_session"))
        .await?;

    set_session_cookie(&raw_token);
    leptos_axum::redirect("/");
    // The session travels only in the HttpOnly cookie set above (#533) — `raw_token`
    // is never returned. The authenticated `UserRecord` supplies the complete marker
    // seed without another query.
    Ok(super::SessionUser {
        username: record.username,
        is_operator: record.is_operator,
    })
}

/// Revokes the current session and clears the `session` cookie. Missing or stale
/// cookie-only credentials still clear the cookie; explicit Authorization failures
/// reject without clearing it.
#[macros::server]
pub async fn logout() -> WebResult<()> {
    if let Some(auth) = optional_auth().await? {
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        sessions.revoke_session(&auth.token_hash).await?;
    }
    clear_session_cookie();
    leptos_axum::redirect("/");
    Ok(())
}

/// The viewer's session identity — username + operator flag — or `None` for
/// missing/stale cookie-only credentials. Explicit Authorization failures reject.
/// The single reconcile fetch behind the shared session context (#591), superseding
/// `current_user` + the reactive `current_user_is_operator`.
#[macros::server]
pub async fn get_session() -> WebResult<Option<super::SessionUser>> {
    let Some(auth) = optional_auth().await? else {
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
