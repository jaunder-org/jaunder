use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use common::time::UtcInstant;
use host::{
    error, metrics,
    retention::{CleanupResult, Domain},
};
use storage::{
    EmailVerificationStorage, FeedEventStorage, InviteStorage, PasswordResetStorage, PostStorage,
};
use tokio_cron_scheduler::{Job, JobScheduler};

pub(crate) const DATABASE_MAINTENANCE_INTERVAL: Duration = Duration::new(86_400, 0);

/// Runs the bounded, domain-owned database cleanup operations.
///
/// This composition-root service knows the schedule and ordering only. Cutoffs,
/// batches, and SQL remain owned by each storage domain.
pub(crate) struct DatabaseMaintenance {
    posts: Arc<dyn PostStorage>,
    invites: Arc<dyn InviteStorage>,
    email_verifications: Arc<dyn EmailVerificationStorage>,
    password_resets: Arc<dyn PasswordResetStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
}

impl DatabaseMaintenance {
    pub(crate) fn new(
        posts: Arc<dyn PostStorage>,
        invites: Arc<dyn InviteStorage>,
        email_verifications: Arc<dyn EmailVerificationStorage>,
        password_resets: Arc<dyn PasswordResetStorage>,
        feed_events: Arc<dyn FeedEventStorage>,
    ) -> Self {
        Self {
            posts,
            invites,
            email_verifications,
            password_resets,
            feed_events,
        }
    }

    /// Runs every cleanup domain against one frozen eligibility instant.
    pub(crate) async fn run_at(&self, now: UtcInstant) {
        report_cleanup(
            Domain::IdempotencyKeys,
            self.posts.prune_expired_idempotency_keys(now).await,
        );
        report_cleanup(Domain::Invites, self.invites.prune_invites(now).await);
        report_cleanup(
            Domain::EmailVerifications,
            self.email_verifications
                .prune_email_verifications(now)
                .await,
        );
        report_cleanup(
            Domain::PasswordResets,
            self.password_resets.prune_password_resets(now).await,
        );
        report_cleanup(
            Domain::FeedEvents,
            self.feed_events.prune_terminal_events(now).await,
        );
    }

    /// Runs startup maintenance, then schedules subsequent runs at `interval`.
    ///
    /// # Errors
    ///
    /// Returns an error when `interval` is zero or the scheduler cannot be
    /// constructed, populated, or started. Cleanup failures are reported and
    /// swallowed per domain so they do not fail startup.
    pub(crate) async fn start(self, interval: Duration) -> anyhow::Result<JobScheduler> {
        anyhow::ensure!(
            !interval.is_zero(),
            "database maintenance interval must be non-zero"
        );

        self.run_at(UtcInstant::now()).await;

        let maintenance = Arc::new(self);
        let scheduler = JobScheduler::new().await?;
        let job = Job::new_repeated_async(interval, move |_uuid, _lock| {
            let maintenance = Arc::clone(&maintenance);
            Box::pin(async move {
                maintenance.run_at(UtcInstant::now()).await;
            })
        })?;
        scheduler.add(job).await?;
        scheduler.start().await?;
        Ok(scheduler)
    }
}

fn report_cleanup<E>(domain: Domain, result: Result<u64, E>)
where
    E: Error + 'static,
{
    match result {
        Ok(pruned) => {
            metrics::retention_cleanup(domain, CleanupResult::Success, pruned);
            tracing::info!(
                retention.domain = domain.label(),
                pruned,
                "database.maintenance.completed"
            );
        }
        Err(error) => {
            metrics::retention_cleanup(domain, CleanupResult::Failure, 0);
            tracing::warn!(
                retention.domain = domain.label(),
                error = %error,
                "database.maintenance.failed"
            );
            error::report_swallowed(
                error::ErrorKind::Storage,
                error::ErrorClass::Transient,
                "server.maintenance",
                error::SwallowedSource::Error(&error),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn maintenance(
        posts: storage::MockPostStorage,
        invites: storage::MockInviteStorage,
        email_verifications: storage::MockEmailVerificationStorage,
        password_resets: storage::MockPasswordResetStorage,
        feed_events: storage::MockFeedEventStorage,
    ) -> DatabaseMaintenance {
        DatabaseMaintenance::new(
            Arc::new(posts),
            Arc::new(invites),
            Arc::new(email_verifications),
            Arc::new(password_resets),
            Arc::new(feed_events),
        )
    }

    fn successful_stores(
        expected_now: UtcInstant,
    ) -> (
        storage::MockPostStorage,
        storage::MockInviteStorage,
        storage::MockEmailVerificationStorage,
        storage::MockPasswordResetStorage,
        storage::MockFeedEventStorage,
    ) {
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .withf(move |actual| *actual == expected_now)
            .returning(|_| Ok(1));
        let mut invites = storage::MockInviteStorage::new();
        invites
            .expect_prune_invites()
            .withf(move |actual| *actual == expected_now)
            .returning(|_| Ok(2));
        let mut email_verifications = storage::MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .withf(move |actual| *actual == expected_now)
            .returning(|_| Ok(3));
        let mut password_resets = storage::MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .withf(move |actual| *actual == expected_now)
            .returning(|_| Ok(4));
        let mut feed_events = storage::MockFeedEventStorage::new();
        feed_events
            .expect_prune_terminal_events()
            .withf(move |actual| *actual == expected_now)
            .returning(|_| Ok(5));
        (
            posts,
            invites,
            email_verifications,
            password_resets,
            feed_events,
        )
    }

    // guard:no-backend — storage mocks verify composition-root time and isolation.
    #[tokio::test]
    async fn run_at_freezes_one_instant_for_every_domain() {
        let now: UtcInstant = "2026-08-31T12:00:00Z".parse().expect("fixed instant");
        let stores = successful_stores(now);
        maintenance(stores.0, stores.1, stores.2, stores.3, stores.4)
            .run_at(now)
            .await;
    }

    // guard:no-backend — storage mocks prove later domains survive an earlier failure.
    #[tokio::test]
    async fn run_at_continues_after_each_domain_failure() {
        let now: UtcInstant = "2026-08-31T12:00:00Z".parse().expect("fixed instant");
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(sqlx::Error::PoolClosed));
        let mut invites = storage::MockInviteStorage::new();
        invites
            .expect_prune_invites()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(sqlx::Error::PoolClosed));
        let mut email_verifications = storage::MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(sqlx::Error::PoolClosed));
        let mut password_resets = storage::MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(sqlx::Error::PoolClosed));
        let mut feed_events = storage::MockFeedEventStorage::new();
        feed_events
            .expect_prune_terminal_events()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(storage::FeedEventError::Db(sqlx::Error::PoolClosed)));

        maintenance(
            posts,
            invites,
            email_verifications,
            password_resets,
            feed_events,
        )
        .run_at(now)
        .await;
    }
    // guard:no-backend — mocks prove a swallowed failure remains eligible next run.
    #[tokio::test]
    async fn failed_domain_is_retried_on_the_next_run() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let post_attempts = Arc::clone(&attempts);
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .times(2)
            .returning(move |_| {
                if post_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(sqlx::Error::PoolClosed)
                } else {
                    Ok(1)
                }
            });
        let mut invites = storage::MockInviteStorage::new();
        invites.expect_prune_invites().times(2).returning(|_| Ok(0));
        let mut email_verifications = storage::MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .times(2)
            .returning(|_| Ok(0));
        let mut password_resets = storage::MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .times(2)
            .returning(|_| Ok(0));
        let mut feed_events = storage::MockFeedEventStorage::new();
        feed_events
            .expect_prune_terminal_events()
            .times(2)
            .returning(|_| Ok(0));
        let maintenance = maintenance(
            posts,
            invites,
            email_verifications,
            password_resets,
            feed_events,
        );

        let first: UtcInstant = "2026-08-31T12:00:00Z".parse().expect("first instant");
        let second: UtcInstant = "2026-09-01T12:00:00Z".parse().expect("second instant");
        maintenance.run_at(first).await;
        maintenance.run_at(second).await;

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    // guard:no-backend — failing storage mocks prove maintenance is startup-best-effort.
    #[tokio::test]
    async fn start_survives_database_cleanup_failures() {
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .times(1)
            .returning(|_| Err(sqlx::Error::PoolClosed));
        let mut invites = storage::MockInviteStorage::new();
        invites
            .expect_prune_invites()
            .times(1)
            .returning(|_| Err(sqlx::Error::PoolClosed));
        let mut email_verifications = storage::MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .times(1)
            .returning(|_| Err(sqlx::Error::PoolClosed));
        let mut password_resets = storage::MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .times(1)
            .returning(|_| Err(sqlx::Error::PoolClosed));
        let mut feed_events = storage::MockFeedEventStorage::new();
        feed_events
            .expect_prune_terminal_events()
            .times(1)
            .returning(|_| Err(storage::FeedEventError::Db(sqlx::Error::PoolClosed)));

        let mut scheduler = maintenance(
            posts,
            invites,
            email_verifications,
            password_resets,
            feed_events,
        )
        .start(Duration::from_mins(1))
        .await
        .expect("cleanup failures must not fail scheduler startup");
        scheduler.shutdown().await.expect("shutdown scheduler");
    }

    // guard:no-backend — short injected cadence exercises startup plus scheduled runs.
    #[tokio::test]
    async fn start_runs_immediately_then_repeats() {
        let calls = Arc::new(AtomicUsize::new(0));

        let post_calls = Arc::clone(&calls);
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .returning(move |_| {
                post_calls.fetch_add(1, Ordering::SeqCst);
                Ok(0)
            });
        let mut invites = storage::MockInviteStorage::new();
        invites.expect_prune_invites().returning(|_| Ok(0));
        let mut email_verifications = storage::MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .returning(|_| Ok(0));
        let mut password_resets = storage::MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .returning(|_| Ok(0));
        let mut feed_events = storage::MockFeedEventStorage::new();
        feed_events
            .expect_prune_terminal_events()
            .returning(|_| Ok(0));

        let mut scheduler = maintenance(
            posts,
            invites,
            email_verifications,
            password_resets,
            feed_events,
        )
        .start(Duration::from_secs(1))
        .await
        .expect("start maintenance scheduler");

        tokio::time::timeout(Duration::from_secs(3), async {
            while calls.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("scheduled maintenance run");
        scheduler.shutdown().await.expect("shutdown scheduler");
        assert!(calls.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn production_cadence_is_exactly_daily() {
        assert_eq!(DATABASE_MAINTENANCE_INTERVAL.as_secs(), 86_400);
    }
}
