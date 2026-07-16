//! Shared test-only helpers for driving `#[server]` / auth boundary functions
//! under unit test without a live database.
//!
//! `SessionStorage` is `mockall::automock`-mocked behind storage's `test-utils`
//! feature, so the auth session store is a `storage::MockSessionStorage` with
//! only `authenticate` (the method `require_auth` calls) stubbed.

// Helpers here live in a feature-gated test module, which clippy's
// allow-{unwrap,expect}-in-tests does not treat as a test context; allow the
// panics explicitly (this is test scaffolding).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::http::{header, request::Parts, Request};
use common::ids::UserId;
use common::username::Username;
use std::sync::Arc;
use storage::{MockSessionStorage, SessionRecord, SessionStorage};

/// Builds request `Parts` carrying a Bearer credential whose session store
/// authenticates as `(user_id, username)`. Provide it into the reactive owner
/// via `provide_context` so `require_auth()` resolves to this user.
pub(crate) fn auth_parts(user_id: UserId, username: &str) -> Parts {
    let username: Username = username.parse().unwrap();
    let mut mock = MockSessionStorage::new();
    // `require_auth` only ever calls `authenticate`, which must resolve to the
    // fixed user so `AuthUser` extraction succeeds.
    mock.expect_authenticate().returning(move |_raw_token| {
        Ok(SessionRecord {
            token_hash: common::token::TokenHash::from_digest("hash"),
            user_id,
            username: username.clone(),
            label: "test".to_string(),
            created_at: chrono::Utc::now(),
            last_used_at: chrono::Utc::now(),
        })
    });
    let sessions: Arc<dyn SessionStorage> = Arc::new(mock);
    let request = Request::builder()
        .header(header::AUTHORIZATION, "Bearer test-token")
        .body(())
        .unwrap();
    let (mut parts, ()) = request.into_parts();
    parts.extensions.insert(sessions);
    parts
}
