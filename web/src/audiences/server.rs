//! Server-only support for the audiences vertical: unit tests for the
//! `#[server]` endpoints in [`super::api`].

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::super::api::list_my_subscribers;
    use crate::test_support::auth_parts;
    use common::ids::{SubscriptionId, UserId};
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::Arc;
    use storage::{MockSubscriptionStorage, SubscriberSummaryRecord, SubscriptionStorage};

    // guard:no-backend — mock store isolates the server-function boundary.
    #[tokio::test]
    async fn list_my_subscribers_delegates_projection_after_auth() {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        let mut subs = MockSubscriptionStorage::new();
        subs.expect_list_subscriber_summaries()
            .withf(|author| *author == UserId::from(1))
            .returning(|_| {
                Ok(vec![SubscriberSummaryRecord {
                    subscription_id: SubscriptionId::from(7),
                    label: "bob".to_string(),
                }])
            });

        provide_context(Arc::new(subs) as Arc<dyn SubscriptionStorage>);

        let result = list_my_subscribers().await.unwrap();
        drop(owner);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].subscription_id, SubscriptionId::from(7));
        assert_eq!(result[0].label, "bob");
    }
}
