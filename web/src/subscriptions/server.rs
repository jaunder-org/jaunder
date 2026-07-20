//! Host-only support for the subscription endpoints: author resolution + tests.

use crate::error::InternalError;
use common::ids::UserId;
use common::username::Username;
use storage::UserStorage;

/// Resolves an author `user_id` from a validated username, rejecting the
/// caller's own username (self-subscribe) and an unknown username.
///
/// Takes the `users` handle as a parameter rather than reading it from context,
/// keeping it a pure helper its callers wire up.
pub(crate) async fn resolve_author(
    users: &dyn UserStorage,
    author_username: &Username,
    viewer_user_id: UserId,
) -> Result<UserId, InternalError> {
    let author = users
        .get_user_by_username(author_username)
        .await?
        .ok_or_else(|| InternalError::not_found("user"))?;
    if author.user_id == viewer_user_id {
        return Err(InternalError::validation("cannot subscribe to yourself"));
    }
    Ok(author.user_id)
}

#[cfg(test)]
mod tests {
    // Helper fns in this feature-gated test module aren't covered by clippy's
    // allow-{unwrap,expect}-in-tests, so allow the test-scaffolding panics.
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::is_subscribed_to;
    use crate::test_support::auth_parts;
    use common::ids::UserId;
    use common::test_support::parse_username;
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::Arc;
    use storage::{
        MockSubscriptionStorage, MockUserStorage, SubscriptionStorage, UserRecord, UserStorage,
    };

    fn user(user_id: UserId, username: &str) -> UserRecord {
        UserRecord {
            user_id,
            username: parse_username(username),
            display_name: None,
            bio: None,
            created_at: chrono::Utc::now(),
            last_authenticated_at: None,
            email: None,
            email_verified: false,
            is_operator: false,
        }
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn is_subscribed_to_returns_false_when_viewing_own_profile() {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        let mut users = MockUserStorage::new();
        users
            .expect_get_user_by_username()
            .returning(|_username| Ok(Some(user(UserId::from(1), "alice"))));
        provide_context(Arc::new(users) as Arc<dyn UserStorage>);
        provide_context(Arc::new(MockSubscriptionStorage::new()) as Arc<dyn SubscriptionStorage>);

        // `resolve_author` rejects the self-target, so the fn short-circuits to
        // `Ok(false)` without ever consulting the subscription store.
        let result = is_subscribed_to(parse_username("alice")).await;
        drop(owner);
        assert!(!result.unwrap());
    }
}
