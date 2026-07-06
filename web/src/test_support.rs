//! Shared test-only helpers for driving `#[server]` / auth boundary functions
//! under unit test without a live database.
//!
//! `SessionStorage` is not `mockall::automock`-mocked and `web` does not depend
//! on `async-trait`, so the stub below hand-writes the `#[async_trait]`-desugared
//! method signatures (each returns a boxed future) to satisfy the trait.

// Helpers here live in a feature-gated test module, which clippy's
// allow-{unwrap,expect}-in-tests does not treat as a test context; allow the
// panics explicitly (this is test scaffolding).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::http::{header, request::Parts, Request};
use common::username::Username;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use storage::{SessionAuthError, SessionRecord, SessionStorage};

/// A `SessionStorage` whose `authenticate` always succeeds as a fixed user, so
/// `require_auth` / `AuthUser` extraction resolves to that user.
struct StubSessions {
    user_id: i64,
    username: Username,
}

impl SessionStorage for StubSessions {
    fn create_session<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _user_id: i64,
        _label: &'life1 str,
    ) -> Pin<Box<dyn Future<Output = sqlx::Result<String>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Ok("token".to_string()) })
    }

    fn authenticate<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _raw_token: &'life1 str,
    ) -> Pin<Box<dyn Future<Output = Result<SessionRecord, SessionAuthError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        let record = SessionRecord {
            token_hash: "hash".to_string(),
            user_id: self.user_id,
            username: self.username.clone(),
            label: "test".to_string(),
            created_at: chrono::Utc::now(),
            last_used_at: chrono::Utc::now(),
        };
        Box::pin(async move { Ok(record) })
    }

    fn revoke_session<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _token_hash: &'life1 str,
    ) -> Pin<Box<dyn Future<Output = sqlx::Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Ok(()) })
    }

    fn list_sessions<'life0, 'async_trait>(
        &'life0 self,
        _user_id: i64,
    ) -> Pin<Box<dyn Future<Output = sqlx::Result<Vec<SessionRecord>>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Builds request `Parts` carrying a Bearer credential whose session store
/// authenticates as `(user_id, username)`. Provide it into the reactive owner
/// via `provide_context` so `require_auth()` resolves to this user.
pub(crate) fn auth_parts(user_id: i64, username: &str) -> Parts {
    let sessions: Arc<dyn SessionStorage> = Arc::new(StubSessions {
        user_id,
        username: username.parse::<Username>().unwrap(),
    });
    let request = Request::builder()
        .header(header::AUTHORIZATION, "Bearer test-token")
        .body(())
        .unwrap();
    let (mut parts, ()) = request.into_parts();
    parts.extensions.insert(sessions);
    parts
}
