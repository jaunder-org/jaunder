use crate::error::{InternalError, InternalResult};
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use common::ids::UserId;
use common::token::{RawToken, TokenHash};
use common::username::Username;
use host::auth::{self, CredentialResolutionError, CredentialTransport};
use leptos::prelude::expect_context;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use storage::{SessionStorage, UserStorage};

// `CookieSettings` lives in `host` (pure config data); re-exported so the
// `web::auth::CookieSettings` path (the `server` crate provides it into leptos
// context) keeps resolving.
pub use host::auth::CookieSettings;

/// Request-scoped signal that successful explicit authentication should retire
/// an ambient browser session cookie from the response.
#[derive(Clone, Default)]
pub struct SessionCookieRetirement(Arc<AtomicBool>);

impl SessionCookieRetirement {
    pub fn request(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn requested(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

// ---------------------------------------------------------------------------
// User
// ---------------------------------------------------------------------------

/// The authenticated user extracted from a valid session cookie or Bearer token.
#[derive(Debug)]
pub struct User {
    pub user_id: UserId,
    pub username: Username,
    pub token_hash: TokenHash,
}

#[derive(Debug)]
pub enum Rejection {
    MissingToken,
    InvalidAuthorization,
    MissingSessionStorage,
    Session {
        transport: CredentialTransport,
        error: storage::SessionAuthError,
    },
    BasicUsernameMismatch,
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        match self {
            Rejection::MissingSessionStorage
            | Rejection::Session {
                error: storage::SessionAuthError::Internal(_),
                ..
            } => StatusCode::INTERNAL_SERVER_ERROR,
            Rejection::MissingToken
            | Rejection::InvalidAuthorization
            | Rejection::BasicUsernameMismatch
            | Rejection::Session {
                error:
                    storage::SessionAuthError::InvalidToken
                    | storage::SessionAuthError::SessionNotFound,
                ..
            } => StatusCode::UNAUTHORIZED,
        }
        .into_response()
    }
}

impl<S> FromRequestParts<S> for User
where
    S: Send + Sync,
{
    type Rejection = Rejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let resolution = auth::resolve_credential(&parts.headers).map_err(|error| match error {
            CredentialResolutionError::Missing => Rejection::MissingToken,
            CredentialResolutionError::InvalidAuthorization => Rejection::InvalidAuthorization,
        })?;
        let session_cookie_present = resolution.session_cookie_present;
        let credential = resolution.credential;
        let transport = credential.transport;
        let retire_cookie = parts.extensions.get::<SessionCookieRetirement>().cloned();
        let sessions = parts
            .extensions
            .get::<Arc<dyn SessionStorage>>()
            .ok_or(Rejection::MissingSessionStorage)?;

        match sessions.authenticate(&credential.token).await {
            Ok(record) => {
                host::metrics::session_validation(host::metrics::SessionOutcome::Ok);
                verify_basic_username(&record.username, credential.expected_username.as_ref())?;
                if session_cookie_present
                    && transport != CredentialTransport::Cookie
                    && let Some(retirement) = retire_cookie
                {
                    retirement.request();
                }
                Ok(User {
                    user_id: record.user_id,
                    username: record.username,
                    token_hash: record.token_hash,
                })
            }
            Err(error) => {
                host::metrics::session_validation(storage::session_outcome(&error));
                Err(Rejection::Session { transport, error })
            }
        }
    }
}

/// Inner implementation of [`require_auth`] — accepts the already-extracted
/// `Parts` option so pure unit tests can exercise the missing-context path.
///
/// # Errors
///
/// Returns `Err` if `parts` is `None` (missing Leptos request context) or if
/// the session token is absent, invalid, or not found in storage.
pub async fn require_auth_with_parts(parts: Option<Parts>) -> InternalResult<User> {
    let mut parts = parts.ok_or_else(|| {
        InternalError::server_message("missing request Parts context in require_auth")
    })?;
    User::from_request_parts(&mut parts, &())
        .await
        .map_err(auth_rejection_error)
}

/// Extracts the authenticated user inside a Leptos server function.
/// Returns an internal auth error when no valid session is present.
///
/// # Errors
///
/// Returns `Err` if the request is not authenticated (missing or invalid session token).
#[tracing::instrument(name = "web.auth.require_auth")]
pub async fn require_auth() -> InternalResult<User> {
    require_auth_with_parts(leptos::context::use_context::<Parts>()).await
}

/// Resolves an optional authenticated user inside a Leptos server function.
///
/// Missing credentials and failed cookie-only credentials resolve to `None`.
/// Failures attributable to an explicit `Authorization` credential reject.
///
/// # Errors
///
/// Returns an authentication error when a present Authorization credential
/// cannot be resolved or authenticated, and propagates infrastructure failures.
pub(crate) async fn optional_auth() -> InternalResult<Option<User>> {
    let mut parts = leptos::context::use_context::<Parts>()
        .ok_or_else(|| InternalError::server_message("missing request Parts context"))?;
    match User::from_request_parts(&mut parts, &()).await {
        Ok(auth) => Ok(Some(auth)),
        Err(
            Rejection::MissingToken
            | Rejection::Session {
                transport: CredentialTransport::Cookie,
                error:
                    storage::SessionAuthError::InvalidToken | storage::SessionAuthError::SessionNotFound,
            },
        ) => Ok(None),
        Err(error) => Err(auth_rejection_error(error)),
    }
}

/// Authorizes an operator-only server function: the caller must be authenticated
/// (`require_auth`) **and** carry the `is_operator` flag. This is the hard guard
/// (it *errors* `Unauthorized` for a non-operator) — distinct from the soft,
/// `Ok(false)`-returning operator checks the warning-banner endpoints use. It
/// lives here beside `require_auth`, not in any one vertical, because it is a
/// shared authorization primitive (backup + site both gate on it).
///
/// # Errors
///
/// Returns `Err(Unauthorized)` if the caller is unauthenticated, the user no
/// longer exists, or the user is not an operator.
pub async fn require_operator() -> InternalResult<()> {
    let auth = require_auth().await?;
    let users = expect_context::<Arc<dyn UserStorage>>();
    let Some(user) = users.get_user(auth.user_id).await? else {
        return Err(InternalError::unauthorized("user does not exist"));
    };

    if !user.is_operator {
        return Err(InternalError::unauthorized("operator access required"));
    }

    Ok(())
}

/// Soft operator check for the warning-banner endpoints: `Ok(true)` when the
/// caller is an authenticated operator, and `Ok(false)` for a non-operator or
/// missing/stale cookie-only credentials. Failures attributable to an explicit
/// `Authorization` credential reject.
///
/// # Errors
///
/// Returns an authentication error when a present Authorization credential
/// cannot be resolved or authenticated, and propagates infrastructure failures.
pub async fn is_operator_soft() -> InternalResult<bool> {
    let Some(auth) = optional_auth().await? else {
        return Ok(false);
    };
    let users = expect_context::<Arc<dyn UserStorage>>();
    Ok(users
        .get_user(auth.user_id)
        .await?
        .is_some_and(|u| u.is_operator))
}

pub(crate) fn auth_rejection_error(error: Rejection) -> InternalError {
    match error {
        Rejection::MissingToken => InternalError::unauthorized("missing session token"),
        Rejection::InvalidAuthorization => {
            InternalError::unauthorized("invalid authorization header")
        }
        Rejection::MissingSessionStorage => {
            InternalError::server_message("missing SessionStorage context")
        }
        Rejection::BasicUsernameMismatch => {
            InternalError::unauthorized("basic auth username mismatch")
        }
        Rejection::Session {
            error: storage::SessionAuthError::InvalidToken,
            ..
        } => InternalError::unauthorized("invalid session token"),
        Rejection::Session {
            error: storage::SessionAuthError::SessionNotFound,
            ..
        } => InternalError::unauthorized("session not found"),
        Rejection::Session {
            error: storage::SessionAuthError::Internal(error),
            ..
        } => InternalError::storage(error),
    }
}

// ---------------------------------------------------------------------------
// Basic-auth username check (thin Rejection wrapper over common's core)
// ---------------------------------------------------------------------------

/// Verifies that an app-password (Basic auth) request authenticated as the
/// user it claimed. Cookie/Bearer requests carry no expected username and
/// always pass. The check is a direct `Username` equality — case-insensitive,
/// because `Username::from_str` lowercases at construction (see
/// `common::username`), so a differently-cased Basic username still matches a
/// valid session (#344).
///
/// # Errors
///
/// Returns [`Rejection::BasicUsernameMismatch`] when the Basic username
/// does not match the authenticated session's user.
fn verify_basic_username(
    authenticated: &Username,
    expected: Option<&Username>,
) -> Result<(), Rejection> {
    match expected {
        Some(expected) if authenticated != expected => Err(Rejection::BasicUsernameMismatch),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Cookie helpers (leptos/axum adapters over host's pure header builders)
// ---------------------------------------------------------------------------

pub fn set_session_cookie(raw_token: &RawToken) {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;

    let secure = use_context::<CookieSettings>().is_none_or(|settings| settings.secure);

    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = host::auth::session_cookie_header(raw_token, secure);
        if let Ok(val) = axum::http::HeaderValue::from_str(&cookie) {
            opts.insert_header(axum::http::header::SET_COOKIE, val);
        }
    }
}

pub fn clear_session_cookie() {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;

    let secure = use_context::<CookieSettings>().is_none_or(|settings| settings.secure);

    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = host::auth::clear_session_cookie_header(secure);
        if let Ok(val) = axum::http::HeaderValue::from_str(&cookie) {
            opts.insert_header(axum::http::header::SET_COOKIE, val);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::{parse_raw_token, parse_username};
    use leptos::prelude::{Owner, provide_context};
    // `require_operator`, `Arc`, `UserStorage`, `UserId` arrive via `super::*`; only the
    // operator test's fixtures are new here.
    use crate::test_support::auth_parts;
    use storage::MockUserStorage;

    // guard:no-backend — mock store
    #[tokio::test]
    async fn require_operator_rejects_when_user_absent() {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "ghost"));
        let mut users = MockUserStorage::new();
        users.expect_get_user().returning(|_uid| Ok(None));
        provide_context(Arc::new(users) as Arc<dyn UserStorage>);

        let result = require_operator().await;
        drop(owner);
        let err = result.unwrap_err();
        assert!(matches!(
            crate::error::project(err.kind(), err.public_message()),
            crate::error::WebError::Unauthorized
        ));
    }

    #[test]
    fn verify_basic_username_passes_without_expected_username() {
        let user = parse_username("alice");
        assert!(verify_basic_username(&user, None).is_ok());
    }

    #[test]
    fn verify_basic_username_passes_on_match() {
        let user = parse_username("alice");
        let expected = parse_username("alice");
        assert!(verify_basic_username(&user, Some(&expected)).is_ok());
    }

    #[test]
    fn verify_basic_username_match_is_case_insensitive() {
        // #344: Username lowercases at construction, so a differently-cased Basic
        // username still matches the authenticated session's user.
        let authenticated = parse_username("alice");
        let expected = parse_username("Alice"); // normalizes to "alice"
        assert!(verify_basic_username(&authenticated, Some(&expected)).is_ok());
    }

    #[test]
    fn verify_basic_username_rejects_mismatch() {
        let user = parse_username("alice");
        let expected = parse_username("mallory");
        assert!(matches!(
            verify_basic_username(&user, Some(&expected)),
            Err(Rejection::BasicUsernameMismatch)
        ));
    }

    #[test]
    fn set_session_cookie_without_response_options_context_is_noop() {
        Owner::new().with(|| {
            provide_context(CookieSettings { secure: true });
            set_session_cookie(&parse_raw_token("token"));
        });
    }

    #[test]
    fn clear_session_cookie_without_response_options_context_is_noop() {
        Owner::new().with(|| {
            provide_context(CookieSettings { secure: true });
            clear_session_cookie();
        });
    }

    #[test]
    fn auth_rejection_into_response_renders_500_for_missing_session_storage() {
        let response = Rejection::MissingSessionStorage.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn auth_rejection_into_response_renders_500_for_session_internal_error() {
        let response = Rejection::Session {
            transport: CredentialTransport::Bearer,
            error: storage::SessionAuthError::Internal(sqlx::Error::PoolClosed),
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn auth_rejection_error_maps_each_variant_to_expected_internal_error() {
        for rejection in [
            Rejection::MissingToken,
            Rejection::InvalidAuthorization,
            Rejection::BasicUsernameMismatch,
        ] {
            let error = auth_rejection_error(rejection);
            assert!(matches!(
                crate::error::project(error.kind(), error.public_message()),
                crate::error::WebError::Unauthorized
            ));
        }

        let missing_state = auth_rejection_error(Rejection::MissingSessionStorage);
        assert!(matches!(
            crate::error::project(missing_state.kind(), missing_state.public_message()),
            crate::error::WebError::Server { .. }
        ));

        for error in [
            storage::SessionAuthError::InvalidToken,
            storage::SessionAuthError::SessionNotFound,
        ] {
            let rejection = auth_rejection_error(Rejection::Session {
                transport: CredentialTransport::Bearer,
                error,
            });
            assert!(matches!(
                crate::error::project(rejection.kind(), rejection.public_message()),
                crate::error::WebError::Unauthorized
            ));
        }

        let internal = auth_rejection_error(Rejection::Session {
            transport: CredentialTransport::Bearer,
            error: storage::SessionAuthError::Internal(sqlx::Error::PoolClosed),
        });
        assert!(matches!(
            crate::error::project(internal.kind(), internal.public_message()),
            crate::error::WebError::Storage { .. }
        ));
    }

    #[tokio::test]
    async fn require_auth_with_parts_returns_server_error_when_parts_missing() {
        let e = require_auth_with_parts(None).await.unwrap_err();
        assert!(matches!(
            crate::error::project(e.kind(), e.public_message()),
            crate::error::WebError::Server { .. }
        ));
    }

    #[tokio::test]
    async fn auth_user_extraction_rejects_invalid_authorization_before_cookie_fallback() {
        use axum::body::Body;
        use axum::http::{Request, header};

        let request = Request::builder()
            .header(header::AUTHORIZATION, "Negotiate unsupported")
            .header(header::COOKIE, "session=some-token")
            .body(Body::empty())
            .unwrap();
        let (mut parts, _) = request.into_parts();

        let result = User::from_request_parts(&mut parts, &()).await;
        assert!(matches!(
            result.unwrap_err(),
            Rejection::InvalidAuthorization
        ));
    }

    // Pure extractor unit test: with a session cookie but no SessionStorage in the
    // request extensions, `User` extraction rejects with MissingSessionStorage.
    // Touches no router and no database.
    #[tokio::test]
    async fn auth_user_extraction_fails_without_session_storage_extension() {
        use axum::body::Body;
        use axum::http::{Request, header};

        let request = Request::builder()
            .header(header::COOKIE, "session=some-token")
            .body(Body::empty())
            .unwrap();
        let (mut parts, _) = request.into_parts();

        // Attempt to extract User without the session store in extensions
        let result = User::from_request_parts(&mut parts, &()).await;
        assert!(matches!(
            result.unwrap_err(),
            Rejection::MissingSessionStorage
        ));
    }
}
