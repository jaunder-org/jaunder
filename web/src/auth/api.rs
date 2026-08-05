//! The **auth** vertical's API surface (ADR-0070, amended #530): the `#[server]`
//! session endpoints (`get_session`, `login`, `logout`) and their wire types,
//! dual-compiled. `mod.rs` re-exports these so external call sites and the
//! server-fn registrar keep the stable `crate::auth::…` paths.

use crate::error::WebResult;
// `Username` / `ProfferedPassword` / `SessionLabel` are ungated: they are wire-arg
// types of `login`, so the `#[server]`-generated arg struct references them on both
// the client and server builds. `RawToken` is ungated for the same reason — it is
// `login`'s wire *return* type, named in the `#[server]` signature on both builds.
use common::password::ProfferedPassword;
use common::session_label::SessionLabel;
use common::token::RawToken;
use common::username::Username;

// One grouped `feature = "server"` support block for the `#[server]` bodies: the
// sibling `server` module's helpers plus the crate-level server-only dependencies.
#[cfg(feature = "server")]
use {
    super::server::{clear_session_cookie, require_auth, set_session_cookie},
    common::password::Password,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{SessionStorage, UserStorage},
    tracing::Instrument,
};

/// `login`'s success payload: the raw session token (unchanged) plus the viewer's
/// operator flag, so the client writes a complete marker immediately (flash-free
/// first login, #591). Web-only wire type — the elisp frontend uses HTTP Basic auth,
/// not this endpoint.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LoginResponse {
    pub token: RawToken,
    pub is_operator: bool,
}

/// Authenticates a user.  Returns a [`LoginResponse`] (the freshly minted session
/// [`RawToken`] + the viewer's operator flag) and sets the `session` cookie.
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
#[macros::server(skip(password, label))]
pub async fn login(
    username: Username,
    password: ProfferedPassword,
    label: Option<SessionLabel>,
) -> WebResult<LoginResponse> {
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
        let ua = leptos_axum::extract::<axum::http::HeaderMap>()
            .await
            .ok()
            .and_then(|headers| {
                headers
                    .get("user-agent")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Unknown device".to_string());
        let ua = if ua.len() > 200 {
            ua.chars().take(200).collect::<String>()
        } else {
            ua
        };
        SessionLabel::from_lossy(&ua)
    };

    let raw_token = sessions
        .create_session(record.user_id, &session_label)
        .instrument(tracing::info_span!("web.auth.login.create_session"))
        .await?;

    set_session_cookie(&raw_token);
    leptos_axum::redirect("/");
    // `record` is the authenticated `UserRecord`, which already carries
    // `is_operator` (storage `UserRecord`) — no extra query. `raw_token` is the
    // typed `RawToken` (#578); `LoginResponse` carries it plus the marker seed.
    Ok(LoginResponse {
        token: raw_token,
        is_operator: record.is_operator,
    })
}

/// Revokes the current session and clears the `session` cookie.
#[macros::server]
pub async fn logout() -> WebResult<()> {
    if let Ok(auth) = require_auth().await {
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        sessions.revoke_session(&auth.token_hash).await?;
    }
    clear_session_cookie();
    leptos_axum::redirect("/");
    Ok(())
}

/// The viewer's session identity — username + operator flag — or `None` when
/// anonymous/expired. The single reconcile fetch behind the shared session context
/// (#591), superseding `current_user` + the reactive `current_user_is_operator`.
#[macros::server]
pub async fn get_session() -> WebResult<Option<super::SessionUser>> {
    let auth = match require_auth().await {
        Ok(auth) => auth,
        Err(error) if error.kind() == crate::error::ErrorKind::Auth => return Ok(None),
        Err(error) => return Err(error),
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
