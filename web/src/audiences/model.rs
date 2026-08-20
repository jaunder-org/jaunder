//! Presentation models and assembly for named-audience screens.

use common::audience::AudienceName;
use common::ids::{AudienceId, SubscriptionId};
use reactive_stores::{Patch, Store};
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use crate::error::InternalResult;
#[cfg(feature = "server")]
use common::ids::UserId;
#[cfg(feature = "server")]
use storage::{AudienceRecord, AudienceStorage, SubscriberSummaryRecord, SubscriptionStorage};

/// A named audience as shown in the management screen.
///
/// A `reactive_stores` keyed-store row (`Store`/`Patch`), so each field carries
/// `#[patch(|this, new| *this = new)]` — the derive's escape hatch, which lets the
/// fields keep their domain types instead of being flattened to `i64`/`String`.
/// Rationale and the rejected alternatives:
/// `docs/adr/0078-reactive-store-domain-newtype-fields.md`.
///
/// `audience_id`'s attribute is required to compile but is behaviorally inert: it is
/// the store key, so `patch_field_keyed` has already matched the two rows *by* it
/// before the closure is reached, and the value can never differ. Only `name`'s
/// attribute does real work — and it is the one the audiences e2e guards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Store, Patch)]
pub struct Summary {
    #[patch(|this, new| *this = new)]
    pub audience_id: AudienceId,
    #[patch(|this, new| *this = new)]
    pub name: AudienceName,
}

/// One of the author's active subscribers, for the assignment checklist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriberSummary {
    pub subscription_id: SubscriptionId,
    /// The local subscriber's username, or the raw channel-scoped reference when
    /// it could not be resolved to a local user.
    pub label: String,
}

#[cfg(feature = "server")]
pub(super) async fn list_audiences(
    author_user_id: UserId,
    audiences: &dyn AudienceStorage,
) -> InternalResult<Vec<Summary>> {
    let rows = audiences.list_audiences(author_user_id).await?;
    Ok(rows.into_iter().map(summary_from_record).collect())
}

#[cfg(feature = "server")]
pub(super) async fn list_subscribers(
    author_user_id: UserId,
    subscriptions: &dyn SubscriptionStorage,
) -> InternalResult<Vec<SubscriberSummary>> {
    let rows = subscriptions
        .list_subscriber_summaries(author_user_id)
        .await?;
    Ok(rows.into_iter().map(subscriber_from_record).collect())
}

#[cfg(feature = "server")]
fn summary_from_record(row: AudienceRecord) -> Summary {
    Summary {
        audience_id: row.audience_id,
        name: row.name,
    }
}

#[cfg(feature = "server")]
fn subscriber_from_record(row: SubscriberSummaryRecord) -> SubscriberSummary {
    SubscriberSummary {
        subscription_id: row.subscription_id,
        label: row.label,
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use common::test_support::parse_audience_name;
    use storage::{MockAudienceStorage, MockSubscriptionStorage};

    #[tokio::test]
    async fn list_audiences_maps_rows_and_preserves_order() {
        let mut audiences = MockAudienceStorage::new();
        audiences.expect_list_audiences().returning(|author| {
            assert_eq!(author, UserId::from(9));
            Ok(vec![
                AudienceRecord {
                    audience_id: AudienceId::from(3),
                    name: parse_audience_name("Friends"),
                    created_at: chrono::Utc::now(),
                },
                AudienceRecord {
                    audience_id: AudienceId::from(5),
                    name: parse_audience_name("Family"),
                    created_at: chrono::Utc::now(),
                },
            ])
        });

        let result = list_audiences(UserId::from(9), &audiences).await.unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].audience_id, AudienceId::from(3));
        assert_eq!(result[0].name, parse_audience_name("Friends"));
        assert_eq!(result[1].audience_id, AudienceId::from(5));
        assert_eq!(result[1].name, parse_audience_name("Family"));
    }

    #[tokio::test]
    async fn list_subscribers_maps_projection_and_preserves_order() {
        let mut subscriptions = MockSubscriptionStorage::new();
        subscriptions
            .expect_list_subscriber_summaries()
            .returning(|author| {
                assert_eq!(author, UserId::from(9));
                Ok(vec![
                    SubscriberSummaryRecord {
                        subscription_id: SubscriptionId::from(11),
                        label: "alice".to_string(),
                    },
                    SubscriberSummaryRecord {
                        subscription_id: SubscriptionId::from(12),
                        label: "remote:42".to_string(),
                    },
                ])
            });

        let result = list_subscribers(UserId::from(9), &subscriptions)
            .await
            .unwrap();

        assert_eq!(
            result,
            vec![
                SubscriberSummary {
                    subscription_id: SubscriptionId::from(11),
                    label: "alice".to_string(),
                },
                SubscriberSummary {
                    subscription_id: SubscriptionId::from(12),
                    label: "remote:42".to_string(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn list_subscribers_propagates_projection_errors() {
        let mut subscriptions = MockSubscriptionStorage::new();
        subscriptions
            .expect_list_subscriber_summaries()
            .returning(|_| Err(sqlx::Error::PoolClosed));

        assert!(
            list_subscribers(UserId::from(9), &subscriptions)
                .await
                .is_err()
        );
    }
}
