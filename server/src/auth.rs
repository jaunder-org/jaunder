use std::sync::Arc;

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use leptos::prelude::ServerFnError;
use rand::RngCore;

use crate::storage::AppState;
use crate::username::Username;

/// Generates an opaque session token: 32 cryptographically random bytes encoded
/// as base64url without padding (43 characters).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
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
