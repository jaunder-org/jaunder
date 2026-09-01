use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use common::time::UtcInstant;
use host::{
    error::{self, ErrorClass, ErrorKind, SwallowedSource},
    metrics,
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
    pub(crate) async fn start(self, interval: Duration) -> Result<JobScheduler> {
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
            metrics::retention_run(domain, CleanupResult::Success);
            let domain = domain.label();
            tracing::info!(
                retention.domain = domain,
                pruned,
                "database.maintenance.completed"
            );
        }
        Err(error) => {
            metrics::retention_run(domain, CleanupResult::Failure);
            error::report_swallowed(
                ErrorKind::Storage,
                ErrorClass::Transient,
                "server.maintenance",
                SwallowedSource::Error(&error),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Error as SqlxError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use storage::{
        FeedEventError, MockEmailVerificationStorage, MockFeedEventStorage, MockInviteStorage,
        MockPasswordResetStorage, MockPostStorage,
    };
    use tokio::time;
    #[derive(Clone)]
    struct SharedWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("trace lock").extend_from_slice(bytes);
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
        std::io::Write::flush(&mut SharedWriter(output.clone())).expect("flush trace");
        String::from_utf8(output.lock().expect("trace lock").clone()).expect("utf8 trace")
    }

    fn maintenance(
        posts: MockPostStorage,
        invites: MockInviteStorage,
        email_verifications: MockEmailVerificationStorage,
        password_resets: MockPasswordResetStorage,
        feed_events: MockFeedEventStorage,
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
        MockPostStorage,
        MockInviteStorage,
        MockEmailVerificationStorage,
        MockPasswordResetStorage,
        MockFeedEventStorage,
    ) {
        let mut posts = MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .withf(move |actual| *actual == expected_now)
            .returning(|_| Ok(1));
        let mut invites = MockInviteStorage::new();
        invites
            .expect_prune_invites()
            .withf(move |actual| *actual == expected_now)
            .returning(|_| Ok(2));
        let mut email_verifications = MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .withf(move |actual| *actual == expected_now)
            .returning(|_| Ok(3));
        let mut password_resets = MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .withf(move |actual| *actual == expected_now)
            .returning(|_| Ok(4));
        let mut feed_events = MockFeedEventStorage::new();
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
        let mut posts = MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(SqlxError::PoolClosed));
        let mut invites = MockInviteStorage::new();
        invites
            .expect_prune_invites()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(SqlxError::PoolClosed));
        let mut email_verifications = MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(SqlxError::PoolClosed));
        let mut password_resets = MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(SqlxError::PoolClosed));
        let mut feed_events = MockFeedEventStorage::new();
        feed_events
            .expect_prune_terminal_events()
            .withf(move |actual| *actual == now)
            .times(1)
            .returning(|_| Err(FeedEventError::Db(SqlxError::PoolClosed)));

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
        let mut posts = MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .times(2)
            .returning(move |_| {
                if post_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(SqlxError::PoolClosed)
                } else {
                    Ok(1)
                }
            });
        let mut invites = MockInviteStorage::new();
        invites.expect_prune_invites().times(2).returning(|_| Ok(0));
        let mut email_verifications = MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .times(2)
            .returning(|_| Ok(0));
        let mut password_resets = MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .times(2)
            .returning(|_| Ok(0));
        let mut feed_events = MockFeedEventStorage::new();
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
        let mut posts = MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .times(1)
            .returning(|_| Err(SqlxError::PoolClosed));
        let mut invites = MockInviteStorage::new();
        invites
            .expect_prune_invites()
            .times(1)
            .returning(|_| Err(SqlxError::PoolClosed));
        let mut email_verifications = MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .times(1)
            .returning(|_| Err(SqlxError::PoolClosed));
        let mut password_resets = MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .times(1)
            .returning(|_| Err(SqlxError::PoolClosed));
        let mut feed_events = MockFeedEventStorage::new();
        feed_events
            .expect_prune_terminal_events()
            .times(1)
            .returning(|_| Err(FeedEventError::Db(SqlxError::PoolClosed)));

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
        let mut posts = MockPostStorage::new();
        posts
            .expect_prune_expired_idempotency_keys()
            .returning(move |_| {
                post_calls.fetch_add(1, Ordering::SeqCst);
                Ok(0)
            });
        let mut invites = MockInviteStorage::new();
        invites.expect_prune_invites().returning(|_| Ok(0));
        let mut email_verifications = MockEmailVerificationStorage::new();
        email_verifications
            .expect_prune_email_verifications()
            .returning(|_| Ok(0));
        let mut password_resets = MockPasswordResetStorage::new();
        password_resets
            .expect_prune_password_resets()
            .returning(|_| Ok(0));
        let mut feed_events = MockFeedEventStorage::new();
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

        time::timeout(Duration::from_secs(3), async {
            while calls.load(Ordering::SeqCst) < 2 {
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("scheduled maintenance run");
        scheduler.shutdown().await.expect("shutdown scheduler");
        assert!(calls.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn report_cleanup_logs_a_bounded_successful_domain_result() {
        let (guard, output) = trace_capture();

        report_cleanup(Domain::Invites, Ok::<u64, std::io::Error>(3));

        drop(guard);
        let trace = trace_text(&output);
        assert_eq!(
            trace
                .matches(r#""message":"database.maintenance.completed""#)
                .count(),
            1,
            "trace: {trace}"
        );
        assert!(
            trace.contains(r#""retention.domain":"invites""#),
            "trace: {trace}"
        );
        assert!(trace.contains(r#""pruned":3"#), "trace: {trace}");
        assert!(
            !trace.contains(r#""error":"#),
            "a successful cleanup must not produce an error field: {trace}"
        );
    }

    #[test]
    fn report_cleanup_reports_a_bounded_failure_once() {
        let (guard, output) = trace_capture();

        report_cleanup(
            Domain::FeedEvents,
            Err::<u64, _>(std::io::Error::other("cleanup storage unavailable")),
        );

        drop(guard);
        let trace = trace_text(&output);
        assert_eq!(
            trace
                .matches(r#""message":"error swallowed after reporting""#)
                .count(),
            1,
            "failure must produce exactly one warning: {trace}"
        );
        assert!(
            !trace.contains("database.maintenance.failed"),
            "the swallowed-error reporter owns the warning: {trace}"
        );
        assert!(
            trace.contains(r#""error.source":"cleanup storage unavailable""#),
            "trace: {trace}"
        );
        assert_eq!(
            trace
                .matches(r#""error.context":"server.maintenance""#)
                .count(),
            1,
            "failure must be reported once: {trace}"
        );
        assert!(
            trace.contains(r#""error.kind":"storage""#),
            "trace: {trace}"
        );
        assert!(
            trace.contains(r#""error.class":"transient""#),
            "trace: {trace}"
        );
        assert!(
            trace.contains(r#""error.disposition":"swallowed""#),
            "trace: {trace}"
        );
        assert!(
            !trace.contains("http://") && !trace.contains("user@"),
            "bounded maintenance reporting must not include fixture PII: {trace}"
        );
    }

    #[test]
    fn production_cadence_is_exactly_daily() {
        assert_eq!(DATABASE_MAINTENANCE_INTERVAL.as_secs(), 86_400);
    }
}
