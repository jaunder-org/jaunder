use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use base64::Engine as _;
use common::client_telemetry::{
    CLIENT_TELEMETRY_VERSION, ClientErrorContext, ClientErrorKind, ClientSourceKind,
    ClientTelemetryEvent,
};
use opentelemetry_sdk::metrics::{
    InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
    data::{AggregatedMetrics, MetricData},
};
use rstest::*;
use rstest_reuse::*;
use storage::{
    SessionStorage,
    test_support::{Backend, TestEnv, backends},
};
use tower::ServiceExt;

use crate::helpers::create_user_and_session;

const PATH: &str = "/api/client-telemetry";
const INTAKE_WARNING: &str = "client error swallowed after reporting";

#[derive(Clone)]
struct CapturedWriter(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for CapturedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("event capture lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

struct MetricPoint {
    name: String,
    value: u64,
    attributes: BTreeMap<String, String>,
}

struct Observation {
    response: Response,
    events: String,
    metrics: Vec<MetricPoint>,
}

impl Observation {
    fn metric_points(&self, name: &str) -> impl Iterator<Item = &MetricPoint> {
        self.metrics.iter().filter(move |point| point.name == name)
    }
}

async fn observe(app: Router, request: Request<Body>) -> Observation {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    opentelemetry::global::set_meter_provider(provider.clone());

    let output = Arc::new(std::sync::Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(CapturedWriter(output.clone()))
        .finish();
    let response = {
        let _guard = tracing::subscriber::set_default(subscriber);
        app.oneshot(request).await.expect("telemetry request")
    };
    provider.force_flush().expect("flush telemetry metrics");

    let events = String::from_utf8(output.lock().expect("event capture lock").clone())
        .expect("captured events are UTF-8");
    let metrics = exporter
        .get_finished_metrics()
        .expect("finished metrics")
        .iter()
        .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
        .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
        .filter_map(|metric| match metric.data() {
            AggregatedMetrics::U64(MetricData::Sum(sum)) => Some((metric.name().to_string(), sum)),
            _ => None,
        })
        .flat_map(|(name, sum)| {
            sum.data_points().map(move |point| MetricPoint {
                name: name.clone(),
                value: point.value(),
                attributes: point
                    .attributes()
                    .map(|kv| (kv.key.as_str().to_owned(), kv.value.to_string()))
                    .collect(),
            })
        })
        .collect();

    Observation {
        response,
        events,
        metrics,
    }
}

fn limiter() -> Arc<jaunder::client_telemetry::ClientTelemetryLimiter> {
    Arc::new(jaunder::client_telemetry::ClientTelemetryLimiter::new())
}

fn app(
    sessions: Arc<dyn SessionStorage>,
    limiter: Arc<jaunder::client_telemetry::ClientTelemetryLimiter>,
) -> Router {
    // This constructor is deliberately narrower than the production composition
    // root: the route is proven to need only its session store and limiter.
    jaunder::client_telemetry::router(sessions, limiter)
}

fn request(
    body: impl Into<Body>,
    content_type: Option<&str>,
    cookie: Option<&str>,
    authorization: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method("POST").uri(PATH);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(authorization) = authorization {
        builder = builder.header(header::AUTHORIZATION, authorization);
    }
    builder.body(body.into()).expect("client telemetry request")
}

fn event() -> ClientTelemetryEvent {
    ClientTelemetryEvent {
        version: CLIENT_TELEMETRY_VERSION,
        kind: ClientErrorKind::Storage,
        context: ClientErrorContext::ThemeStorageRead,
        source_kind: ClientSourceKind::StorageUnavailable,
    }
}

fn event_json() -> String {
    serde_json::to_string(&event()).expect("serialize telemetry event")
}

fn assert_silent_rejection(observation: &Observation, expected: StatusCode) {
    assert_eq!(observation.response.status(), expected);
    assert!(
        !observation.events.contains(INTAKE_WARNING),
        "rejection emitted intake warning: {}",
        observation.events
    );
    assert_eq!(
        observation.metric_points("jaunder.errors").count(),
        0,
        "rejection emitted jaunder.errors"
    );
    assert_eq!(
        observation
            .metric_points("jaunder.auth.session_validations")
            .count(),
        0,
        "intake used the general session-validation metric"
    );
}

#[apply(backends)]
#[tokio::test]
async fn missing_malformed_unknown_and_revoked_cookies_return_401(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let sessions: Arc<dyn SessionStorage> = state.sessions.clone();
    let body = event_json();

    let cases = [
        request(body.clone(), Some("application/json"), None, None),
        request(
            body.clone(),
            Some("application/json"),
            Some("session=not-a-token!"),
            None,
        ),
        request(
            body.clone(),
            Some("application/json"),
            Some(&format!("session={}", host::token::generate())),
            None,
        ),
    ];
    for request in cases {
        let observation = observe(app(sessions.clone(), limiter()), request).await;
        assert_silent_rejection(&observation, StatusCode::UNAUTHORIZED);
    }

    let token_hash = host::token::hash(&session.token).expect("hash session token");
    state
        .sessions
        .revoke_session(&token_hash)
        .await
        .expect("revoke session");
    let observation = observe(
        app(sessions, limiter()),
        request(
            body,
            Some("application/json"),
            Some(&session.cookie()),
            None,
        ),
    )
    .await;
    assert_silent_rejection(&observation, StatusCode::UNAUTHORIZED);
}

#[apply(backends)]
#[tokio::test]
async fn bearer_and_basic_without_cookie_return_401(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let sessions: Arc<dyn SessionStorage> = state.sessions.clone();
    let bearer = format!("Bearer {}", session.token);
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", session.username, session.token))
    );

    for authorization in [&bearer, &basic] {
        let observation = observe(
            app(sessions.clone(), limiter()),
            request(
                event_json(),
                Some("application/json"),
                None,
                Some(authorization),
            ),
        )
        .await;
        assert_silent_rejection(&observation, StatusCode::UNAUTHORIZED);
    }
}

#[apply(backends)]
#[tokio::test]
async fn malformed_json_unsupported_version_unknown_enum_and_unknown_field_return_400(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let sessions: Arc<dyn SessionStorage> = state.sessions.clone();
    let cookie = session.cookie();
    let bodies = [
        "{".to_owned(),
        r#"{"version":2,"kind":"storage","context":"theme_storage_read","source_kind":"storage_unavailable"}"#.to_owned(),
        r#"{"version":1,"kind":"unknown","context":"theme_storage_read","source_kind":"storage_unavailable"}"#.to_owned(),
        r#"{"version":1,"kind":"storage","context":"theme_storage_read","source_kind":"storage_unavailable","detail":"unbounded"}"#.to_owned(),
    ];

    for body in bodies {
        let observation = observe(
            app(sessions.clone(), limiter()),
            request(body, Some("application/json"), Some(&cookie), None),
        )
        .await;
        assert_silent_rejection(&observation, StatusCode::BAD_REQUEST);
    }
}

#[apply(backends)]
#[tokio::test]
async fn missing_and_text_content_types_return_415(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let sessions: Arc<dyn SessionStorage> = state.sessions.clone();
    let cookie = session.cookie();

    for content_type in [None, Some("text/plain")] {
        let observation = observe(
            app(sessions.clone(), limiter()),
            request(event_json(), content_type, Some(&cookie), None),
        )
        .await;
        assert_silent_rejection(&observation, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
}

#[apply(backends)]
#[tokio::test]
async fn body_limit_accepts_1024_for_decode_and_rejects_1025(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let sessions: Arc<dyn SessionStorage> = state.sessions.clone();
    let cookie = session.cookie();

    for (size, status) in [
        (1_024, StatusCode::BAD_REQUEST),
        (1_025, StatusCode::PAYLOAD_TOO_LARGE),
    ] {
        let mut body = String::from("{");
        body.extend(std::iter::repeat_n(' ', size - 1));
        assert_eq!(body.len(), size);
        let observation = observe(
            app(sessions.clone(), limiter()),
            request(body, Some("application/json"), Some(&cookie), None),
        )
        .await;
        assert_silent_rejection(&observation, status);
    }
}

#[apply(backends)]
#[tokio::test]
async fn closed_session_storage_returns_silent_500(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let sessions: Arc<dyn SessionStorage> = state.sessions.clone();
    let app = app(sessions, limiter());
    base.close_pool().await;

    let observation = observe(
        app,
        request(
            event_json(),
            Some("application/json"),
            Some(&session.cookie()),
            None,
        ),
    )
    .await;
    assert_silent_rejection(&observation, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn sixth_event_for_one_user_returns_silent_429(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let sessions: Arc<dyn SessionStorage> = state.sessions.clone();
    let limiter = limiter();
    let app = app(sessions, limiter);
    let cookie = session.cookie();

    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(request(
                event_json(),
                Some("application/json"),
                Some(&cookie),
                None,
            ))
            .await
            .expect("accepted telemetry request");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    let observation = observe(
        app,
        request(event_json(), Some("application/json"), Some(&cookie), None),
    )
    .await;
    assert_silent_rejection(&observation, StatusCode::TOO_MANY_REQUESTS);
}

#[apply(backends)]
#[tokio::test]
async fn valid_cookie_returns_204_and_reports_one_client_swallow(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let sessions: Arc<dyn SessionStorage> = state.sessions.clone();

    let observation = observe(
        app(sessions, limiter()),
        request(
            event_json(),
            Some("application/json"),
            Some(&session.cookie()),
            None,
        ),
    )
    .await;

    assert_eq!(observation.response.status(), StatusCode::NO_CONTENT);
    let warnings: Vec<_> = observation
        .events
        .lines()
        .filter(|line| line.contains(INTAKE_WARNING))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "one fixed intake warning: {}",
        observation.events
    );
    let warning = warnings[0];
    for field in [
        r#""error.kind":"storage""#,
        r#""error.class":"transient""#,
        r#""error.disposition":"swallowed""#,
        r#""telemetry.origin":"client""#,
        r#""error.context":"client.theme_storage.read""#,
        r#""error.source_kind":"storage_unavailable""#,
    ] {
        assert!(warning.contains(field), "missing {field}: {warning}");
    }

    let points: Vec<_> = observation
        .metric_points("jaunder.errors")
        .filter(|point| {
            point
                .attributes
                .get("error.disposition")
                .map(String::as_str)
                == Some("swallowed")
                && point.attributes.get("telemetry.origin").map(String::as_str) == Some("client")
        })
        .collect();
    assert_eq!(points.len(), 1, "one swallowed/client metric point");
    assert_eq!(points[0].value, 1, "metric increments once");
    assert_eq!(
        observation
            .metric_points("jaunder.auth.session_validations")
            .count(),
        0,
        "dedicated guard suppresses general session metric"
    );
}

#[apply(backends)]
#[tokio::test]
async fn valid_cookie_wins_when_valid_bearer_or_basic_is_also_present(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie_session = create_user_and_session(&state).await;
    let authorization_session = create_user_and_session(&state).await;
    let sessions: Arc<dyn SessionStorage> = state.sessions.clone();
    let limiter = limiter();
    let app = app(sessions, limiter);

    // Exhaust the Authorization user's bucket through the only accepted credential
    // source, its cookie. If Authorization were consulted below, both requests would
    // be rejected; the other user's cookie must select its independent bucket.
    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(request(
                event_json(),
                Some("application/json"),
                Some(&authorization_session.cookie()),
                None,
            ))
            .await
            .expect("warm authorization-user bucket");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    let bearer = format!("Bearer {}", authorization_session.token);
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            authorization_session.username, authorization_session.token
        ))
    );
    for authorization in [&bearer, &basic] {
        let response = app
            .clone()
            .oneshot(request(
                event_json(),
                Some("application/json"),
                Some(&cookie_session.cookie()),
                Some(authorization),
            ))
            .await
            .expect("cookie-precedence telemetry request");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}
