//! Factory-minted transaction scopes for composable application writes.

use std::fmt;

use common::mutation::MutationOutcome;
use futures_util::future::BoxFuture;
use sqlx::{PgPool, SqlitePool, Transaction};
use tracing::Instrument;

/// The concrete transaction held by [`WriteTransaction`].
enum HeldTransaction {
    Sqlite(Transaction<'static, sqlx::Sqlite>),
    Postgres(Transaction<'static, sqlx::Postgres>),
    #[cfg(any(test, feature = "test-utils"))]
    Mock,
}

/// A backend-erased mutable capability for one application write transaction.
///
/// Application code cannot construct this capability or execute SQL through it.
/// Storage traits consume it to join the caller-owned [`WriteScope`].
pub struct WriteTransaction {
    transaction: HeldTransaction,
}

/// A failure before a callback can begin, or a callback failure whose transaction
/// has not been committed.
#[derive(Debug)]
pub enum WriteScopeError<E> {
    /// The callback failed; dropping the tracked transaction rolls it back.
    Operation(E),
    /// Beginning a write scope failed before an application operation ran.
    Begin(sqlx::Error),
}

impl<E: fmt::Display> fmt::Display for WriteScopeError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation(error) => write!(formatter, "write operation failed: {error}"),
            Self::Begin(error) => write!(formatter, "write scope could not begin: {error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for WriteScopeError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Operation(error) => Some(error),
            Self::Begin(error) => Some(error),
        }
    }
}

/// A concrete transaction factory selected at the composition root.
///
/// This type only enters a scope. It neither locates storage nor exposes SQL.
#[derive(Clone)]
pub struct WriteScope {
    backend: ScopeBackend,
    #[cfg(test)]
    lose_commit_acknowledgement_after_commit: bool,
}

#[derive(Clone)]
enum ScopeBackend {
    Sqlite(SqlitePool),
    Postgres(PgPool),
    #[cfg(any(test, feature = "test-utils"))]
    Mock,
}

impl WriteTransaction {
    fn sqlite(&mut self) -> Option<&mut sqlx::SqliteConnection> {
        match &mut self.transaction {
            HeldTransaction::Sqlite(transaction) => Some(&mut **transaction),
            HeldTransaction::Postgres(_) => None,
            #[cfg(any(test, feature = "test-utils"))]
            HeldTransaction::Mock => None,
        }
    }

    fn postgres(&mut self) -> Option<&mut sqlx::PgConnection> {
        match &mut self.transaction {
            HeldTransaction::Sqlite(_) => None,
            HeldTransaction::Postgres(transaction) => Some(&mut **transaction),
            #[cfg(any(test, feature = "test-utils"))]
            HeldTransaction::Mock => None,
        }
    }
}

/// Borrows the `SQLite` connection held by `capability`.
pub(crate) fn sqlite_connection(
    capability: &mut WriteTransaction,
) -> Result<&mut sqlx::SqliteConnection, sqlx::Error> {
    capability
        .sqlite()
        .ok_or_else(|| sqlx::Error::Protocol("SQLite write capability required".into()))
}

/// Borrows the `PostgreSQL` connection held by `capability`.
pub(crate) fn postgres_connection(
    capability: &mut WriteTransaction,
) -> Result<&mut sqlx::PgConnection, sqlx::Error> {
    capability
        .postgres()
        .ok_or_else(|| sqlx::Error::Protocol("PostgreSQL write capability required".into()))
}

/// Classifies a commit acknowledgement while preserving the completed mutation value.
fn classify_commit_result<T>(commit: Result<(), sqlx::Error>, value: T) -> MutationOutcome<T> {
    match commit {
        Ok(()) => MutationOutcome::Confirmed(value),
        Err(error) => {
            // The server must return the mutation value as indeterminate because the
            // database may have committed before acknowledgement failed. Report the
            // suppressed commit error through the owning runtime's structured seam.
            host::error::report_swallowed(
                host::error::ErrorKind::Storage,
                host::error::ErrorClass::Transient,
                "storage.write_scope.commit_acknowledgement",
                host::error::SwallowedSource::Error(&error),
            );
            MutationOutcome::CommitIndeterminate(value)
        }
    }
}

impl WriteScope {
    pub(crate) fn sqlite(pool: SqlitePool) -> Self {
        Self {
            backend: ScopeBackend::Sqlite(pool),
            #[cfg(test)]
            lose_commit_acknowledgement_after_commit: false,
        }
    }

    pub(crate) fn postgres(pool: PgPool) -> Self {
        Self {
            backend: ScopeBackend::Postgres(pool),
            #[cfg(test)]
            lose_commit_acknowledgement_after_commit: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_commit_acknowledgement_loss_after_commit_for_test(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            lose_commit_acknowledgement_after_commit: true,
        }
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub(crate) fn mock() -> Self {
        Self {
            backend: ScopeBackend::Mock,
            #[cfg(test)]
            lose_commit_acknowledgement_after_commit: false,
        }
    }

    /// Executes one coherent mutation and makes its commit boundary explicit.
    ///
    /// A callback error drops the tracked transaction without committing it. A
    /// callback success makes exactly one commit attempt; a failed acknowledgement
    /// is returned as [`MutationOutcome::CommitIndeterminate`].
    ///
    /// # Errors
    ///
    /// Returns [`WriteScopeError::Begin`] if transaction acquisition fails, or
    /// [`WriteScopeError::Operation`] if the callback rejects the mutation.
    pub async fn run<T, E>(
        &self,
        callback: impl for<'scope> FnOnce(
            &'scope mut WriteTransaction,
        ) -> BoxFuture<'scope, Result<T, E>>,
    ) -> Result<MutationOutcome<T>, WriteScopeError<E>> {
        let db_system = match self.backend {
            ScopeBackend::Sqlite(_) => "sqlite",
            ScopeBackend::Postgres(_) => "postgres",
            #[cfg(any(test, feature = "test-utils"))]
            ScopeBackend::Mock => "mock",
        };
        let span = tracing::info_span!(
            "storage.write_scope",
            db.system = db_system,
            write_scope.outcome = tracing::field::Empty,
        );
        let span_for_future = span.clone();
        async move {
            let transaction = match &self.backend {
                ScopeBackend::Sqlite(pool) => HeldTransaction::Sqlite(
                    pool.begin_with("BEGIN IMMEDIATE")
                        .await
                        .map_err(WriteScopeError::Begin)?,
                ),
                ScopeBackend::Postgres(pool) => {
                    HeldTransaction::Postgres(pool.begin().await.map_err(WriteScopeError::Begin)?)
                }
                #[cfg(any(test, feature = "test-utils"))]
                ScopeBackend::Mock => HeldTransaction::Mock,
            };
            let mut capability = WriteTransaction { transaction };
            let value = match callback(&mut capability).await {
                Ok(value) => value,
                Err(error) => {
                    span.record("write_scope.outcome", "rollback_confirmed");
                    return Err(WriteScopeError::Operation(error));
                }
            };
            let commit = match capability.transaction {
                HeldTransaction::Sqlite(transaction) => transaction.commit().await,
                HeldTransaction::Postgres(transaction) => transaction.commit().await,
                #[cfg(any(test, feature = "test-utils"))]
                HeldTransaction::Mock => Ok(()),
            };
            let outcome = classify_commit_result(commit, value);
            #[cfg(test)]
            let outcome = match (self.lose_commit_acknowledgement_after_commit, outcome) {
                (true, MutationOutcome::Confirmed(value)) => {
                    MutationOutcome::CommitIndeterminate(value)
                }
                (_, outcome) => outcome,
            };
            let outcome_label = match outcome {
                MutationOutcome::Confirmed(_) => "confirmed_commit",
                MutationOutcome::CommitIndeterminate(_) => "commit_indeterminate",
            };
            span.record("write_scope.outcome", outcome_label);
            Ok(outcome)
        }
        .instrument(span_for_future)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, SeedRawPost, SeedUser, backends, set_post_tags_confirmed};
    use crate::{PostStorage, TaggingError};

    use common::tag::TagLabel;
    use common::test_support::parse_tag_label;
    use common::visibility::ViewerIdentity;
    use futures_util::FutureExt;
    use rstest::*;
    use rstest_reuse::apply;
    use std::panic::AssertUnwindSafe;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    #[derive(Default)]
    struct RecordedOutcomes(Mutex<Vec<String>>);

    struct WriteScopeRecorder(Arc<RecordedOutcomes>);

    struct FieldRecorder(Vec<(String, String)>);

    impl Visit for FieldRecorder {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push((field.name().to_owned(), value.to_owned()));
        }

        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.0.push((field.name().to_owned(), format!("{value:?}")));
        }
    }

    impl<S> Layer<S> for WriteScopeRecorder
    where
        S: tracing::Subscriber + for<'span> LookupSpan<'span>,
    {
        fn on_record(
            &self,
            id: &tracing::span::Id,
            values: &tracing::span::Record<'_>,
            context: Context<'_, S>,
        ) {
            if let Some(span) = context.span(id)
                && span.metadata().name() == "storage.write_scope"
            {
                let mut fields = FieldRecorder(Vec::new());
                values.record(&mut fields);
                self.0.0.lock().expect("outcome recorder mutex").extend(
                    fields.0.into_iter().filter_map(|(field, value)| {
                        (field == "write_scope.outcome").then_some(value)
                    }),
                );
            }
        }
    }

    async fn tag_labels(posts: &dyn PostStorage, post_id: common::ids::PostId) -> Vec<TagLabel> {
        posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .expect("read post")
            .expect("seeded post")
            .tags
            .into_iter()
            .map(|tag| tag.tag_display)
            .collect()
    }

    #[apply(backends)]
    #[tokio::test]
    async fn callback_failure_rolls_back_and_later_writer_commits(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let post_id = SeedRawPost::new(user_id).seed(&env.state).await.post_id;
        let posts = Arc::clone(&env.state.posts);
        let desired = vec![parse_tag_label("rolled-back")];

        let result = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    posts
                        .set_post_tags(transaction, post_id, user_id, &desired)
                        .await
                        .expect("first mutation succeeds before later failure");
                    Err::<(), _>("later mutation failed")
                })
            })
            .await;

        assert!(matches!(
            result,
            Err(WriteScopeError::Operation("later mutation failed"))
        ));
        assert!(tag_labels(&*env.state.posts, post_id).await.is_empty());

        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post_id,
            user_id,
            &[parse_tag_label("later-writer")],
        )
        .await
        .expect("later writer commits after rollback");
        assert_eq!(
            tag_labels(&*env.state.posts, post_id).await,
            vec![parse_tag_label("later-writer")]
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn unwinding_scope_drops_transaction_and_releases_writer(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let post_id = SeedRawPost::new(user_id).seed(&env.state).await.post_id;
        let posts = Arc::clone(&env.state.posts);
        let desired = vec![parse_tag_label("unwound")];

        let unwind = AssertUnwindSafe(env.state.write_scope.run::<(), TaggingError>(
            |transaction| {
                Box::pin(async move {
                    posts
                        .set_post_tags(transaction, post_id, user_id, &desired)
                        .await
                        .expect("mutation succeeds before unwind");
                    panic!("force scope unwind");
                })
            },
        ))
        .catch_unwind()
        .await;

        assert!(unwind.is_err());
        assert!(tag_labels(&*env.state.posts, post_id).await.is_empty());
        set_post_tags_confirmed(
            &env.state.write_scope,
            Arc::clone(&env.state.posts),
            post_id,
            user_id,
            &[parse_tag_label("after-unwind")],
        )
        .await
        .expect("writer remains usable after scope unwind");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn confirmed_commit_and_post_commit_acknowledgement_loss_have_distinct_outcomes(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let post_id = SeedRawPost::new(user_id).seed(&env.state).await.post_id;
        let confirmed_tags = vec![parse_tag_label("confirmed")];
        let confirmed_posts = Arc::clone(&env.state.posts);

        let confirmed = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    confirmed_posts
                        .set_post_tags(transaction, post_id, user_id, &confirmed_tags)
                        .await?;
                    Ok::<_, TaggingError>("confirmed value")
                })
            })
            .await
            .expect("confirmed write scope");
        assert_eq!(confirmed, MutationOutcome::Confirmed("confirmed value"));
        assert_eq!(
            tag_labels(&*env.state.posts, post_id).await,
            vec![parse_tag_label("confirmed")]
        );

        let indeterminate_scope = env
            .state
            .write_scope
            .with_commit_acknowledgement_loss_after_commit_for_test();
        let indeterminate_tags = vec![parse_tag_label("indeterminate")];
        let indeterminate_posts = Arc::clone(&env.state.posts);
        let indeterminate = indeterminate_scope
            .run(|transaction| {
                Box::pin(async move {
                    indeterminate_posts
                        .set_post_tags(transaction, post_id, user_id, &indeterminate_tags)
                        .await?;
                    Ok::<_, TaggingError>("indeterminate value")
                })
            })
            .await
            .expect("post-commit acknowledgement loss is an outcome, not operation error");
        assert_eq!(
            indeterminate,
            MutationOutcome::CommitIndeterminate("indeterminate value")
        );
        assert_eq!(
            tag_labels(&*env.state.posts, post_id).await,
            vec![parse_tag_label("indeterminate")]
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn scope_records_only_bounded_outcome_determinants(#[case] backend: Backend) {
        let env = backend.setup().await;
        let outcomes = Arc::new(RecordedOutcomes::default());
        let subscriber =
            tracing_subscriber::registry().with(WriteScopeRecorder(Arc::clone(&outcomes)));
        let guard = tracing::subscriber::set_default(subscriber);

        let rollback = env
            .state
            .write_scope
            .run(|_| Box::pin(async { Err::<(), _>("operation failure") }))
            .await;
        assert!(matches!(
            rollback,
            Err(WriteScopeError::Operation("operation failure"))
        ));
        assert!(matches!(
            env.state
                .write_scope
                .run(|_| Box::pin(async { Ok::<_, &'static str>(()) }))
                .await,
            Ok(MutationOutcome::Confirmed(()))
        ));
        assert!(matches!(
            env.state
                .write_scope
                .with_commit_acknowledgement_loss_after_commit_for_test()
                .run(|_| Box::pin(async { Ok::<_, &'static str>(()) }))
                .await,
            Ok(MutationOutcome::CommitIndeterminate(()))
        ));
        drop(guard);

        assert_eq!(
            outcomes
                .0
                .lock()
                .expect("outcome recorder mutex")
                .as_slice(),
            [
                "rollback_confirmed",
                "confirmed_commit",
                "commit_indeterminate"
            ]
        );
    }

    #[test]
    fn outcome_recorder_records_debug_values() {
        let outcomes = Arc::new(RecordedOutcomes::default());
        let subscriber =
            tracing_subscriber::registry().with(WriteScopeRecorder(Arc::clone(&outcomes)));
        let guard = tracing::subscriber::set_default(subscriber);
        let span = tracing::info_span!(
            "storage.write_scope",
            db.system = "mock",
            write_scope.outcome = tracing::field::Empty,
        );

        span.record(
            "write_scope.outcome",
            tracing::field::debug("debug outcome"),
        );
        drop(guard);

        assert_eq!(
            outcomes
                .0
                .lock()
                .expect("outcome recorder mutex")
                .as_slice(),
            ["\"debug outcome\""]
        );
    }

    #[test]
    fn begin_error_formats_and_exposes_its_source() {
        let begin_error = sqlx::Error::Io(std::io::Error::other("begin failure"));
        let expected = format!("write scope could not begin: {begin_error}");
        let error = WriteScopeError::<std::io::Error>::Begin(begin_error);

        assert_eq!(error.to_string(), expected);
        assert!(
            std::error::Error::source(&error)
                .is_some_and(|source| source.downcast_ref::<sqlx::Error>().is_some())
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn real_scope_rejects_other_backend_capability(#[case] backend: Backend) {
        let env = backend.setup().await;

        let result = match backend {
            Backend::Sqlite => {
                env.state
                    .write_scope
                    .run(|transaction| {
                        Box::pin(async move { postgres_connection(transaction).map(|_| ()) })
                    })
                    .await
            }
            Backend::Postgres => {
                env.state
                    .write_scope
                    .run(|transaction| {
                        Box::pin(async move { sqlite_connection(transaction).map(|_| ()) })
                    })
                    .await
            }
        };

        assert!(matches!(
            result,
            Err(WriteScopeError::Operation(sqlx::Error::Protocol(_)))
        ));
    }

    // guard:no-backend — mock write scope holds no database connection
    #[tokio::test]
    async fn mock_scope_rejects_database_capabilities() {
        let scope = WriteScope::mock();

        let sqlite = scope
            .run(|transaction| Box::pin(async move { sqlite_connection(transaction).map(|_| ()) }))
            .await;
        let postgres = scope
            .run(|transaction| {
                Box::pin(async move { postgres_connection(transaction).map(|_| ()) })
            })
            .await;

        assert!(matches!(
            sqlite,
            Err(WriteScopeError::Operation(sqlx::Error::Protocol(_)))
        ));
        assert!(matches!(
            postgres,
            Err(WriteScopeError::Operation(sqlx::Error::Protocol(_)))
        ));
    }

    #[test]
    fn failed_commit_acknowledgement_is_indeterminate_with_value_preserved() {
        let outcome = classify_commit_result(
            Err(sqlx::Error::Io(std::io::Error::other(
                "acknowledgement lost",
            ))),
            "completed mutation",
        );

        assert_eq!(
            outcome,
            MutationOutcome::CommitIndeterminate("completed mutation")
        );
    }
}
