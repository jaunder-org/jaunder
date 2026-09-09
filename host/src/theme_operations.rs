//! Principal-scoped admission for expensive Theme Package operations.
//!
//! A permit is acquired immediately after authentication and before package
//! parsing or compilation. Ownership scope is deliberately absent from its key,
//! so switching catalogs cannot evade admission.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};

use common::ids::UserId;

const REFILL_AFTER: Duration = Duration::from_secs(1);
const BURST: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeOperationRejected {
    RateLimited,
    InFlight,
}

#[derive(Default)]
struct State {
    buckets: HashMap<UserId, Bucket>,
    in_flight: HashSet<UserId>,
}
struct Bucket {
    tokens: u8,
    last: Instant,
}
impl Bucket {
    fn new(now: Instant) -> Self {
        Self {
            tokens: BURST,
            last: now,
        }
    }
    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last);
        let tokens = u8::try_from(elapsed.as_secs() / REFILL_AFTER.as_secs()).unwrap_or(u8::MAX);
        if tokens != 0 {
            self.tokens = self.tokens.saturating_add(tokens).min(BURST);
            self.last = now;
        }
    }
}

/// Shared rate and one-in-flight gate for one authenticated principal.
pub struct ThemeOperationCoordinator {
    state: Mutex<State>,
}
impl Default for ThemeOperationCoordinator {
    fn default() -> Self {
        Self::new()
    }
}
impl ThemeOperationCoordinator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }
    /// Atomically consumes a rate token and acquires an exclusive RAII permit.
    ///
    /// # Errors
    ///
    /// Returns [`ThemeOperationRejected`] when the principal exhausted their
    /// rate bucket or already holds the exclusive operation slot.
    pub fn acquire(
        &self,
        principal: UserId,
    ) -> Result<ThemeOperationPermit<'_>, ThemeOperationRejected> {
        let now = Instant::now();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.in_flight.contains(&principal) {
            return Err(ThemeOperationRejected::InFlight);
        }
        let bucket = state
            .buckets
            .entry(principal)
            .or_insert_with(|| Bucket::new(now));
        bucket.refill(now);
        if bucket.tokens == 0 {
            return Err(ThemeOperationRejected::RateLimited);
        }
        bucket.tokens -= 1;
        state.in_flight.insert(principal);
        Ok(ThemeOperationPermit {
            coordinator: self,
            principal,
        })
    }
}

/// Releases its principal's in-flight slot on every exit path, including cancellation.
pub struct ThemeOperationPermit<'a> {
    coordinator: &'a ThemeOperationCoordinator,
    principal: UserId,
}
impl Drop for ThemeOperationPermit<'_> {
    fn drop(&mut self) {
        self.coordinator
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .in_flight
            .remove(&self.principal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refills_at_the_elapsed_second_boundary_and_caps_at_the_burst() {
        let now = Instant::now();
        let mut bucket = Bucket {
            tokens: 0,
            last: now.checked_sub(REFILL_AFTER).expect("monotonic instant"),
        };

        bucket.refill(now);
        assert_eq!(bucket.tokens, 1);
        assert_eq!(bucket.last, now);

        bucket.tokens = BURST - 1;
        bucket.last = now
            .checked_sub(REFILL_AFTER * u32::from(BURST))
            .expect("monotonic instant");
        bucket.refill(now);
        assert_eq!(bucket.tokens, BURST);
    }

    #[test]
    fn default_coordinator_is_ready_to_admit_operations() {
        let coordinator = ThemeOperationCoordinator::default();

        assert!(coordinator.acquire(UserId::from(1)).is_ok());
    }

    #[test]
    fn rejects_a_second_operation_while_its_principal_is_in_flight() {
        let coordinator = ThemeOperationCoordinator::new();
        let permit = coordinator.acquire(UserId::from(1)).expect("first permit");

        assert!(matches!(
            coordinator.acquire(UserId::from(1)),
            Err(ThemeOperationRejected::InFlight)
        ));

        drop(permit);
    }

    #[test]
    fn rate_limits_a_principal_after_its_burst_is_consumed() {
        let coordinator = ThemeOperationCoordinator::new();

        for _ in 0..BURST {
            drop(coordinator.acquire(UserId::from(1)).expect("burst permit"));
        }

        assert!(matches!(
            coordinator.acquire(UserId::from(1)),
            Err(ThemeOperationRejected::RateLimited)
        ));
    }

    #[test]
    fn keeps_rate_buckets_independent_between_principals() {
        let coordinator = ThemeOperationCoordinator::new();

        for _ in 0..BURST {
            drop(
                coordinator
                    .acquire(UserId::from(1))
                    .expect("first principal permit"),
            );
        }

        assert!(coordinator.acquire(UserId::from(2)).is_ok());
    }

    #[test]
    fn releases_a_permit_after_early_exit_and_explicit_drop() {
        let coordinator = ThemeOperationCoordinator::new();
        let early_exit = || -> Result<(), ()> {
            let _permit = coordinator.acquire(UserId::from(1)).expect("permit");
            Err(())
        };

        assert_eq!(early_exit(), Err(()));
        let permit = coordinator
            .acquire(UserId::from(1))
            .expect("released after early exit");
        drop(permit);
        assert!(coordinator.acquire(UserId::from(1)).is_ok());
    }
    #[tokio::test]
    async fn releases_a_permit_when_an_in_flight_operation_is_cancelled() {
        let coordinator = std::sync::Arc::new(ThemeOperationCoordinator::new());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let operation_coordinator = std::sync::Arc::clone(&coordinator);
        let operation = tokio::spawn(async move {
            let _permit = operation_coordinator
                .acquire(UserId::from(1))
                .expect("operation permit");
            started_tx.send(()).expect("test observes acquired permit");
            // cov:ignore-start: the cancellation fixture must remain pending until operation.abort(), so it cannot complete under host coverage
            // The cancellation test intentionally holds this future pending.
            std::future::pending::<()>().await;
            // cov:ignore-stop
        }); // cov:ignore: cancellation fault injection aborts the pending fixture before its closure-ending span can execute

        started_rx.await.expect("operation acquires its permit");
        assert!(matches!(
            coordinator.acquire(UserId::from(1)),
            Err(ThemeOperationRejected::InFlight)
        ));

        operation.abort();
        assert!(
            operation
                .await
                .expect_err("operation is cancelled")
                .is_cancelled()
        );
        assert!(
            coordinator.acquire(UserId::from(1)).is_ok(),
            "cancellation drops the permit"
        );
    }
}
