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

    #[derive(Clone)]
    struct SharedWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedWriter {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    fn trace_capture() -> (
        tracing::subscriber::DefaultGuard,
        Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        let output = Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()))
            .finish();
        (tracing::subscriber::set_default(subscriber), output)
    }

    fn trace_text(output: &Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        assert!(
            std::io::Write::flush(&mut SharedWriter(output.clone())).is_ok(),
            "trace flush"
        );
        let bytes = output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }

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
    // guard:no-backend — mock stores isolate the absent local-user fallback.
    #[tokio::test]
    async fn continuation_reporting_list_my_subscribers_uses_raw_ref_for_absent_user_without_report()
     {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        let mut subs = MockSubscriptionStorage::new();
        subs.expect_list_subscribers().returning(|_author| {
            Ok(vec![SubscriptionRecord {
                subscription_id: SubscriptionId::from(7),
                channel_id: ChannelId::from(1),
                subscriber_ref: "42".to_string(),
                status: SubscriptionStatus::Active,
                created_at: chrono::Utc::now(),
            }])
        });
        provide_context(Arc::new(subs) as Arc<dyn SubscriptionStorage>);
        let mut users = MockUserStorage::new();
        users
            .expect_get_user()
            .withf(|user_id| *user_id == UserId::from(42))
            .times(1)
            .returning(|_| Ok(None));
        provide_context(Arc::new(users) as Arc<dyn UserStorage>);
        let (guard, output) = trace_capture();

        let result = list_my_subscribers().await.unwrap();
        drop(guard);
        drop(owner);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "42");
        assert!(trace_text(&output).is_empty(), "absence must not report");
    }

    // guard:no-backend — mock stores isolate the degraded label lookup.
    #[tokio::test]
    async fn continuation_reporting_list_my_subscribers_reports_storage_label_failure_once_and_returns_raw_ref()
     {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        let mut subs = MockSubscriptionStorage::new();
        subs.expect_list_subscribers().returning(|_author| {
            Ok(vec![SubscriptionRecord {
                subscription_id: SubscriptionId::from(7),
                channel_id: ChannelId::from(1),
                subscriber_ref: "42".to_string(),
                status: SubscriptionStatus::Active,
                created_at: chrono::Utc::now(),
            }])
        });
        provide_context(Arc::new(subs) as Arc<dyn SubscriptionStorage>);
        let mut users = MockUserStorage::new();
        users
            .expect_get_user()
            .withf(|user_id| *user_id == UserId::from(42))
            .returning(|_| Err(sqlx::Error::PoolClosed));
        provide_context(Arc::new(users) as Arc<dyn UserStorage>);
        let (guard, output) = trace_capture();

        let result = list_my_subscribers().await.unwrap();
        drop(guard);
        drop(owner);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "42");
        let trace = trace_text(&output);
        assert_eq!(
            trace
                .matches(r#""error.context":"web.audiences.subscriber_label_lookup""#)
                .count(),
            1,
            "trace: {trace}"
        );
    }
}
