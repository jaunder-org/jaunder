//! Authenticated, bounded intake for swallowed browser-error diagnostics.
//!
//! The route has its own cookie-only guard and receives only the session store
//! plus its process-local limiter. The wire is closed in `common`; this module
//! authenticates and bounds it before converting the event to host observability.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Extension, FromRequestParts, Request},
    http::{StatusCode, header, request::Parts},
    routing::post,
};
use common::{
    client_telemetry::{ClientErrorContext, ClientErrorKind, ClientTelemetryEvent},
    ids::UserId,
};
use host::error::{ErrorClass, ErrorKind};
use storage::{SessionAuthError, SessionStorage};

const MAX_BODY_BYTES: usize = 1_024;
const BURST: u8 = 5;
const REFILL: Duration = Duration::from_mins(1);
const STALE: Duration = Duration::from_mins(15);
const MAX_CLEANUP: usize = 64;

/// Monotonic time source used by [`ClientTelemetryLimiter`].
///
/// Production uses [`Instant::now`]; tests inject manual time so refill and
/// cleanup behavior are deterministic.
pub trait ClientTelemetryClock: Send + Sync {
    /// Returns the current monotonic instant.
    #[must_use]
    fn now(&self) -> Instant;
}

#[derive(Default)]
struct SystemClock;

impl ClientTelemetryClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

struct Bucket {
    tokens: u8,
    last_refill: Instant,
    last_activity: Instant,
}

impl Bucket {
    fn new(now: Instant) -> Self {
        Self {
            tokens: BURST,
            last_refill: now,
            last_activity: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        if self.tokens == BURST {
            // Capacity accumulated while already full is discarded. A token
            // consumed after an idle period therefore waits a complete minute.
            self.last_refill = now;
            return;
        }

        let intervals = now.duration_since(self.last_refill).as_secs() / REFILL.as_secs();
        if intervals == 0 {
            return;
        }
        let available = u64::from(BURST - self.tokens);
        let added = intervals.min(available);
        self.tokens += u8::try_from(added).unwrap_or(BURST);
        if self.tokens == BURST {
            self.last_refill = now;
        } else {
            self.last_refill += REFILL * u32::try_from(intervals).unwrap_or(u32::from(BURST));
        }
    }
}

#[derive(Default)]
struct LimiterState {
    buckets: HashMap<UserId, Bucket>,
    ring: VecDeque<UserId>,
}

/// Per-user token bucket for the untrusted diagnostics intake.
///
/// The map and ring are instance state: constructing a limiter always starts
/// empty. Every bucket has exactly one ring entry, and cleanup rotates through a
/// bounded prefix so no request performs an unbounded identity scan.
pub struct ClientTelemetryLimiter {
    clock: Arc<dyn ClientTelemetryClock>,
    state: Mutex<LimiterState>,
}

impl Default for ClientTelemetryLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientTelemetryLimiter {
    /// Constructs a fresh limiter using monotonic process time.
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    /// Constructs a fresh limiter with an injected monotonic clock.
    #[must_use]
    pub fn with_clock(clock: Arc<dyn ClientTelemetryClock>) -> Self {
        Self {
            clock,
            state: Mutex::new(LimiterState::default()),
        }
    }

    /// Attempts to consume one token for `user_id`.
    #[must_use]
    pub fn accept(&self, user_id: UserId) -> bool {
        let now = self.clock.now();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::cleanup(&mut state, now);

        let (bucket, inserted) = match state.buckets.entry(user_id) {
            std::collections::hash_map::Entry::Occupied(entry) => (entry.into_mut(), false),
            std::collections::hash_map::Entry::Vacant(entry) => {
                (entry.insert(Bucket::new(now)), true)
            }
        };
        bucket.refill(now);
        bucket.last_activity = now;
        let accepted = if bucket.tokens == 0 {
            false
        } else {
            bucket.tokens -= 1;
            true
        };
        if inserted {
            state.ring.push_back(user_id);
        }
        accepted
    }

    fn cleanup(state: &mut LimiterState, now: Instant) -> usize {
        let visits = state.ring.len().min(MAX_CLEANUP);
        for _ in 0..visits {
            let user_id = state.ring[0];
            let should_evict = {
                let Some(bucket) = state.buckets.get_mut(&user_id) else {
                    unreachable!("ring entries always have buckets");
                };
                bucket.refill(now);
                bucket.tokens == BURST && now.duration_since(bucket.last_activity) > STALE
            };
            if should_evict {
                let _removed_user_id = state.ring.pop_front();
                let _removed_bucket = state.buckets.remove(&user_id);
            } else {
                state.ring.rotate_left(1);
            }
        }
        visits
    }
}

struct BrowserSession {
    user_id: UserId,
}

impl<S> FromRequestParts<S> for BrowserSession
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let token =
            host::auth::resolve_session_cookie(&parts.headers).ok_or(StatusCode::UNAUTHORIZED)?;
        let sessions = parts
            .extensions
            .get::<Arc<dyn SessionStorage>>()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        match sessions.authenticate(&token).await {
            Ok(record) => Ok(Self {
                user_id: record.user_id,
            }),
            Err(SessionAuthError::InvalidToken | SessionAuthError::SessionNotFound) => {
                Err(StatusCode::UNAUTHORIZED)
            }
            Err(SessionAuthError::Internal(_)) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}

/// Builds the raw intake route with exactly its two injected dependencies.
pub fn router(sessions: Arc<dyn SessionStorage>, limiter: Arc<ClientTelemetryLimiter>) -> Router {
    Router::new()
        .route("/api/client-telemetry", post(intake))
        .layer(Extension(limiter))
        .layer(Extension(sessions))
}

async fn intake(
    session: BrowserSession,
    Extension(limiter): Extension<Arc<ClientTelemetryLimiter>>,
    request: Request,
) -> StatusCode {
    if !is_json(request.headers()) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE;
    }

    let body = match bounded_body(request.into_body()).await {
        Ok(body) => body,
        Err(status) => return status,
    };
    let Ok(event) = serde_json::from_slice::<ClientTelemetryEvent>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    if !limiter.accept(session.user_id) {
        return StatusCode::TOO_MANY_REQUESTS;
    }

    let (kind, class) = error_classification(event.kind);
    host::error::report_client_swallowed(
        kind,
        class,
        error_context(event.context),
        event.source_kind,
    );
    StatusCode::NO_CONTENT
}

fn is_json(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

async fn bounded_body(body: Body) -> Result<axum::body::Bytes, StatusCode> {
    match to_bytes(body, MAX_BODY_BYTES).await {
        Ok(body) => Ok(body),
        Err(error) => {
            let limit_exceeded = std::error::Error::source(&error).is_some_and(
                <dyn std::error::Error + 'static>::is::<http_body_util::LengthLimitError>,
            );
            if limit_exceeded {
                Err(StatusCode::PAYLOAD_TOO_LARGE)
            } else {
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
}

fn error_classification(kind: ClientErrorKind) -> (ErrorKind, ErrorClass) {
    match kind {
        ClientErrorKind::Network | ClientErrorKind::Dialog => {
            (ErrorKind::External, ErrorClass::External)
        }
        ClientErrorKind::Storage => (ErrorKind::Storage, ErrorClass::Transient),
        ClientErrorKind::Decode => (ErrorKind::Internal, ErrorClass::Bug),
        ClientErrorKind::FormData | ClientErrorKind::Internal => {
            (ErrorKind::Internal, ErrorClass::Bug)
        }
    }
}

fn error_context(context: ClientErrorContext) -> &'static str {
    match context {
        ClientErrorContext::ThemeStorageRead => "client.theme_storage.read",
        ClientErrorContext::ThemeStorageWrite => "client.theme_storage.write",
        ClientErrorContext::SessionMarkerRead => "client.session_marker.read",
        ClientErrorContext::SessionMarkerWrite => "client.session_marker.write",
        ClientErrorContext::SessionMarkerRemove => "client.session_marker.remove",
        ClientErrorContext::ProjectorSeedDecode => "client.projector_seed.decode",
        ClientErrorContext::PublishConfirm => "client.publish.confirm",
        ClientErrorContext::DeleteConfirm => "client.delete.confirm",
        ClientErrorContext::MediaFormData => "client.media.form_data",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ManualClock(Mutex<Instant>);

    impl ManualClock {
        fn new() -> Self {
            Self(Mutex::new(Instant::now()))
        }

        fn advance(&self, duration: Duration) {
            let mut now = self.0.lock().expect("manual clock lock");
            *now += duration;
        }
    }

    impl ClientTelemetryClock for ManualClock {
        fn now(&self) -> Instant {
            *self.0.lock().expect("manual clock lock")
        }
    }

    fn manual_limiter() -> (Arc<ManualClock>, ClientTelemetryLimiter) {
        let clock = Arc::new(ManualClock::new());
        let limiter = ClientTelemetryLimiter::with_clock(clock.clone());
        (clock, limiter)
    }

    fn counts(limiter: &ClientTelemetryLimiter) -> (usize, usize) {
        let state = limiter.state.lock().expect("limiter state");
        (state.buckets.len(), state.ring.len())
    }

    #[test]
    fn five_immediate_attempts_are_accepted_and_sixth_is_rejected() {
        let (_clock, limiter) = manual_limiter();
        let user = UserId::from(1);

        for _ in 0..BURST {
            assert!(limiter.accept(user));
        }
        assert!(!limiter.accept(user));
    }

    #[test]
    fn one_token_refills_after_one_minute() {
        let (clock, limiter) = manual_limiter();
        let user = UserId::from(1);
        for _ in 0..BURST {
            assert!(limiter.accept(user));
        }
        assert!(!limiter.accept(user));

        clock.advance(REFILL);
        assert!(limiter.accept(user));
        assert!(!limiter.accept(user));
    }

    #[test]
    fn users_have_independent_buckets() {
        let (_clock, limiter) = manual_limiter();
        let first = UserId::from(1);
        let second = UserId::from(2);
        for _ in 0..BURST {
            assert!(limiter.accept(first));
        }

        assert!(!limiter.accept(first));
        assert!(limiter.accept(second));
    }

    #[test]
    fn full_idle_bucket_is_retained_before_stale_boundary_and_evicted_after() {
        let (clock, limiter) = manual_limiter();
        let user = UserId::from(1);
        assert!(limiter.accept(user));

        clock.advance(Duration::from_secs(899));
        {
            let now = clock.now();
            let mut state = limiter.state.lock().expect("limiter state");
            ClientTelemetryLimiter::cleanup(&mut state, now);
        }
        assert!(
            limiter
                .state
                .lock()
                .expect("limiter state")
                .buckets
                .contains_key(&user)
        );

        clock.advance(Duration::from_secs(2));
        {
            let now = clock.now();
            let mut state = limiter.state.lock().expect("limiter state");
            ClientTelemetryLimiter::cleanup(&mut state, now);
        }
        assert!(
            !limiter
                .state
                .lock()
                .expect("limiter state")
                .buckets
                .contains_key(&user)
        );
    }

    #[test]
    fn duplicate_attempts_never_duplicate_ring_entries() {
        let (_clock, limiter) = manual_limiter();
        let user = UserId::from(1);
        for _ in 0..10 {
            let _ = limiter.accept(user);
        }

        assert_eq!(counts(&limiter), (1, 1));
    }

    #[test]
    fn bounded_round_robin_cleanup_eventually_visits_every_retained_bucket() {
        let (clock, limiter) = manual_limiter();
        for id in 1..=70 {
            assert!(limiter.accept(UserId::from(id)));
        }
        assert_eq!(counts(&limiter), (70, 70));
        clock.advance(STALE + Duration::from_secs(1));

        let first_visits = {
            let now = clock.now();
            let mut state = limiter.state.lock().expect("limiter state");
            ClientTelemetryLimiter::cleanup(&mut state, now)
        };
        assert_eq!(first_visits, MAX_CLEANUP);
        assert_eq!(counts(&limiter), (6, 6));

        let second_visits = {
            let now = clock.now();
            let mut state = limiter.state.lock().expect("limiter state");
            ClientTelemetryLimiter::cleanup(&mut state, now)
        };
        assert_eq!(second_visits, 6);
        assert_eq!(counts(&limiter), (0, 0));
    }

    #[test]
    fn fresh_limiters_have_empty_independent_state() {
        let (_first_clock, first) = manual_limiter();
        for _ in 0..BURST {
            assert!(first.accept(UserId::from(1)));
        }
        assert!(!first.accept(UserId::from(1)));

        let second = ClientTelemetryLimiter::default();
        assert_eq!(counts(&second), (0, 0));
        assert!(second.accept(UserId::from(1)));
        assert_eq!(counts(&first), (1, 1));
        assert_eq!(counts(&second), (1, 1));
    }

    #[tokio::test]
    async fn non_limit_body_read_failure_returns_internal_server_error() {
        let body = Body::from_stream(futures_util::stream::once(async {
            Err::<axum::body::Bytes, _>(std::io::Error::other("body read failed"))
        }));

        assert_eq!(
            bounded_body(body).await,
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        );
    }

    #[test]
    fn every_closed_client_enum_maps_to_bounded_host_fields() {
        let classifications = [
            (
                ClientErrorKind::Network,
                ErrorKind::External,
                ErrorClass::External,
            ),
            (
                ClientErrorKind::Storage,
                ErrorKind::Storage,
                ErrorClass::Transient,
            ),
            (
                ClientErrorKind::Decode,
                ErrorKind::Internal,
                ErrorClass::Bug,
            ),
            (
                ClientErrorKind::Dialog,
                ErrorKind::External,
                ErrorClass::External,
            ),
            (
                ClientErrorKind::FormData,
                ErrorKind::Internal,
                ErrorClass::Bug,
            ),
            (
                ClientErrorKind::Internal,
                ErrorKind::Internal,
                ErrorClass::Bug,
            ),
        ];
        for (client, kind, class) in classifications {
            assert_eq!(error_classification(client), (kind, class));
        }

        let contexts = [
            (
                ClientErrorContext::ThemeStorageRead,
                "client.theme_storage.read",
            ),
            (
                ClientErrorContext::ThemeStorageWrite,
                "client.theme_storage.write",
            ),
            (
                ClientErrorContext::SessionMarkerRead,
                "client.session_marker.read",
            ),
            (
                ClientErrorContext::SessionMarkerWrite,
                "client.session_marker.write",
            ),
            (
                ClientErrorContext::SessionMarkerRemove,
                "client.session_marker.remove",
            ),
            (
                ClientErrorContext::ProjectorSeedDecode,
                "client.projector_seed.decode",
            ),
            (ClientErrorContext::PublishConfirm, "client.publish.confirm"),
            (ClientErrorContext::DeleteConfirm, "client.delete.confirm"),
            (ClientErrorContext::MediaFormData, "client.media.form_data"),
        ];
        for (client, context) in contexts {
            assert_eq!(error_context(client), context);
        }
    }
}
