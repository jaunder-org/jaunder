//! Server-only support for the audiences vertical: unit tests for the
//! `#[server]` endpoints in [`super::api`].

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::super::api::list_my_subscribers;
    use crate::test_support::auth_parts;
    use common::ids::{ChannelId, SubscriptionId, UserId};
    use common::visibility::SubscriptionStatus;
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::Arc;
    use storage::{
        MockSubscriptionStorage, MockUserStorage, SubscriptionRecord, SubscriptionStorage,
        UserStorage,
    };

    // guard:no-backend — mock store
    #[tokio::test]
    async fn list_my_subscribers_falls_back_to_raw_ref_when_non_numeric() {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        let mut subs = MockSubscriptionStorage::new();
        subs.expect_list_subscribers().returning(|_author| {
            Ok(vec![SubscriptionRecord {
                subscription_id: SubscriptionId::from(7),
                channel_id: ChannelId::from(1),
                subscriber_ref: "not-a-number".to_string(),
                status: SubscriptionStatus::Active,
                created_at: chrono::Utc::now(),
            }])
        });
        provide_context(Arc::new(subs) as Arc<dyn SubscriptionStorage>);
        // A non-numeric `subscriber_ref` never parses to a user id, so `get_user`
        // is never called; the raw reference is used as the display label.
        provide_context(Arc::new(MockUserStorage::new()) as Arc<dyn UserStorage>);

        let result = list_my_subscribers().await.unwrap();
        drop(owner);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "not-a-number");
    }
}
