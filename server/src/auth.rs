use std::{fmt, str::FromStr, sync::Arc};

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use leptos::prelude::ServerFnError;
use rand::RngCore;
use thiserror::Error;

use crate::storage::{AppState, SiteConfigStorage};
use crate::username::Username;

/// Generates an opaque session token: 32 cryptographically random bytes encoded
/// as base64url without padding (43 characters).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The site's user-registration access policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationPolicy {
    /// Anyone may register without a code.
    Open,
    /// New accounts require a valid, unused invite code.
    InviteOnly,
    /// Registration is disabled; no new accounts can be created.
    Closed,
}

impl fmt::Display for RegistrationPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistrationPolicy::Open => write!(f, "open"),
            RegistrationPolicy::InviteOnly => write!(f, "invite_only"),
            RegistrationPolicy::Closed => write!(f, "closed"),
        }
    }
}

/// Error returned when a string does not name a valid [`RegistrationPolicy`].
#[derive(Debug, Error)]
#[error("invalid registration policy: {0:?}")]
pub struct InvalidRegistrationPolicy(String);

impl FromStr for RegistrationPolicy {
    type Err = InvalidRegistrationPolicy;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(RegistrationPolicy::Open),
            "invite_only" => Ok(RegistrationPolicy::InviteOnly),
            "closed" => Ok(RegistrationPolicy::Closed),
            other => Err(InvalidRegistrationPolicy(other.to_owned())),
        }
    }
}

/// Reads `site.registration_policy` from the config store and parses it.
///
/// Returns [`RegistrationPolicy::Closed`] when the key is absent or its
/// value cannot be parsed — a safe default that prevents unintended open
/// registration on a freshly initialised instance.
pub async fn load_registration_policy(store: &dyn SiteConfigStorage) -> RegistrationPolicy {
    store
        .get("site.registration_policy")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RegistrationPolicy::Closed)
}

/// The authenticated user extracted from a valid session cookie or Bearer token.
pub struct AuthUser {
    pub user_id: i64,
    pub username: Username,
    pub token_hash: String,
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Cookie takes precedence over Authorization header.
        let raw_token = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                s.split(';')
                    .map(str::trim)
                    .find(|c| c.starts_with("session="))
                    .and_then(|c| c.strip_prefix("session="))
                    .map(str::to_string)
            })
            .or_else(|| {
                parts
                    .headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
                    .map(str::to_string)
            });

        let raw_token = raw_token.ok_or(StatusCode::UNAUTHORIZED)?;

        let state = parts
            .extensions
            .get::<Arc<AppState>>()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        match state.sessions.authenticate(&raw_token).await {
            Ok(record) => Ok(AuthUser {
                user_id: record.user_id,
                username: record.username,
                token_hash: record.token_hash,
            }),
            Err(_) => Err(StatusCode::UNAUTHORIZED),
        }
    }
}

/// Extracts the authenticated user inside a Leptos server function.
/// Returns `ServerFnError` when no valid session is present.
pub async fn require_auth() -> Result<AuthUser, ServerFnError> {
    leptos_axum::extract::<AuthUser>()
        .await
        .map_err(|_| ServerFnError::new("unauthorized"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteSiteConfigStorage;

    // --- FromStr / Display ---

    #[test]
    fn open_parses() {
        assert_eq!(
            "open".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::Open
        );
    }

    #[test]
    fn invite_only_parses() {
        assert_eq!(
            "invite_only".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::InviteOnly
        );
    }

    #[test]
    fn closed_parses() {
        assert_eq!(
            "closed".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::Closed
        );
    }

    #[test]
    fn unknown_string_returns_error() {
        assert!("unknown".parse::<RegistrationPolicy>().is_err());
    }

    #[test]
    fn display_round_trips() {
        for policy in [
            RegistrationPolicy::Open,
            RegistrationPolicy::InviteOnly,
            RegistrationPolicy::Closed,
        ] {
            assert_eq!(
                policy.to_string().parse::<RegistrationPolicy>().unwrap(),
                policy
            );
        }
    }

    // --- load_registration_policy ---

    async fn in_memory_store() -> SqliteSiteConfigStorage {
        let pool = sqlx::SqlitePool::connect(":memory:")
            .await
            .expect("in-memory SQLite pool should open");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations should run on in-memory pool");
        SqliteSiteConfigStorage::new(pool)
    }

    #[tokio::test]
    async fn absent_key_returns_closed() {
        let store = in_memory_store().await;
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::Closed
        );
    }

    #[tokio::test]
    async fn key_set_to_open_returns_open() {
        let store = in_memory_store().await;
        store.set("site.registration_policy", "open").await.unwrap();
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::Open
        );
    }

    #[tokio::test]
    async fn key_set_to_invite_only_returns_invite_only() {
        let store = in_memory_store().await;
        store
            .set("site.registration_policy", "invite_only")
            .await
            .unwrap();
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::InviteOnly
        );
    }

    #[tokio::test]
    async fn invalid_value_in_db_returns_closed() {
        let store = in_memory_store().await;
        store
            .set("site.registration_policy", "garbage")
            .await
            .unwrap();
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::Closed
        );
    }
}
