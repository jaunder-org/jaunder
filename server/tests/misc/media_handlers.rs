use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use common::ids::UserId;
use common::test_support::parse_content_hash;
use host::etag::from_content_hash;
use server_fn::ServerFn;
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};

use crate::helpers::{
    MultipartFile, body_string, confirmed_mutation, create_user_and_session, make_app,
    post_multipart,
};

/// Captures one request-boundary error event and its `jaunder.errors` point.
///
/// Task 3 established these event/metric fields; request-boundary tests reuse the
/// same real tracing subscriber and in-memory `OTel` exporter rather than mocking
/// either reporting path.
#[macro_export]
macro_rules! assert_error_signal {
    (
        $future:expr,
        event = $event_marker:literal,
        event_kind = $event_kind:literal,
        event_class = $event_class:literal,
        metric_kind = $metric_kind:literal,
        metric_class = $metric_class:literal,
        disposition = $disposition:literal,
        context = $context:literal
    ) => {{
        #[derive(Clone)]
        struct CapturedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

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

        use opentelemetry_sdk::metrics::{
            InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
            data::{AggregatedMetrics, MetricData},
        };

        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        opentelemetry::global::set_meter_provider(provider.clone());

        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(CapturedWriter(output.clone()))
            .finish();
        let value = {
            let _guard = tracing::subscriber::set_default(subscriber);
            $future.await
        };
        provider.force_flush().expect("flush error metrics");

        let text = String::from_utf8(output.lock().expect("event capture lock").clone())
            .expect("captured events are UTF-8");
        let events: Vec<_> = text
            .lines()
            .filter(|line| line.contains($event_marker))
            .collect();
        assert_eq!(events.len(), 1, "exactly one error event: {text}");
        let event = events[0].to_owned();
        assert!(
            event.contains(&format!(r#""error.kind":"{}""#, $event_kind)),
            "event kind: {event}"
        );
        assert!(
            event.contains(&format!(r#""error.class":"{}""#, $event_class)),
            "event class: {event}"
        );
        if !$context.is_empty() {
            assert!(event.contains($context), "event context: {event}");
        }

        let metrics = exporter.get_finished_metrics().expect("finished metrics");
        let points: Vec<_> = metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .filter(|metric| metric.name() == "jaunder.errors")
            .filter_map(|metric| match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => Some(sum),
                _ => None,
            })
            .flat_map(opentelemetry_sdk::metrics::data::Sum::data_points)
            .map(|point| {
                (
                    point.value(),
                    point
                        .attributes()
                        .map(|kv| (kv.key.as_str().to_owned(), kv.value.to_string()))
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
            })
            .filter(|(_, attributes)| {
                attributes.get("error.kind").map(String::as_str) == Some($metric_kind)
                    && attributes.get("error.class").map(String::as_str) == Some($metric_class)
                    && attributes.get("error.disposition").map(String::as_str) == Some($disposition)
                    && attributes.get("telemetry.origin").map(String::as_str) == Some("server")
            })
            .collect();
        assert_eq!(points.len(), 1, "one matching jaunder.errors point");
        assert_eq!(points[0].0, 1, "error metric increments exactly once");

        (value, event)
    }};
}

// ---------------------------------------------------------------------------
// Serve tests
// ---------------------------------------------------------------------------

#[apply(backends)]
#[tokio::test]
async fn serve_returns_200_with_cache_headers(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let cookie = create_user_and_session(&state).await.cookie();

    let storage = TempDir::new().unwrap();

    // Upload via the `upload_media` server fn so a file lands on `storage`'s disk;
    // the fn returns 200 with a confirmed `UploadedMedia` mutation payload.
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "serve_test.png",
            content_type: "image/png",
            bytes: b"PNG_CONTENT_HERE",
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload must succeed");

    let upload_json: serde_json::Value = confirmed_mutation(&body);
    let url = upload_json["url"].as_str().unwrap().to_owned();

    // A fresh app over the SAME storage serves the persisted file.
    let app = make_app(&state, &storage);

    let serve_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(serve_response.status(), StatusCode::OK);
    let cache_control = serve_response
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        cache_control.contains("max-age=31536000"),
        "expected immutable cache-control, got: {cache_control}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn serve_without_database_record_preserves_file_response(#[case] backend: Backend) {
    const HASH: &str = "13015a3cf02c05dafbefab3b331350db348e70e86f4e43e73f325473957f0a5c";
    let TestEnv { state, base: _base } = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let file = storage
        .path()
        .join("media/upload/13/01")
        .join(HASH)
        .join("my%20photo.jpg");
    tokio::fs::create_dir_all(file.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&file, b"file-bytes").await.unwrap();

    let response = make_app(&state, &storage)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/media/upload/13/01/{HASH}/my%20photo.jpg"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/jpeg"
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        response.headers().get(header::ETAG).unwrap(),
        &from_content_hash(&parse_content_hash(HASH)).to_string()
    );
    assert_eq!(
        response.headers().get(header::CONTENT_DISPOSITION).unwrap(),
        r#"inline; filename="my photo.jpg"; filename*=UTF-8''my%20photo%2Ejpg"#
    );
    assert_eq!(body_string(response).await, "file-bytes");
}

#[apply(backends)]
#[tokio::test]
async fn serve_returns_404_when_recorded_file_disappears_after_router_setup(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();
    let storage = TempDir::new().unwrap();
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "disappearing.png",
            content_type: "image/png",
            bytes: b"DISAPPEARING_MEDIA",
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "seed upload");
    let upload: serde_json::Value = confirmed_mutation(&body);
    let url = upload["url"].as_str().expect("uploaded media URL");

    // Build the router while both metadata and bytes exist, then remove only the
    // bytes. This deterministically exercises the post-lookup disappearance path
    // on every platform and does not depend on chmod semantics under root.
    let app = make_app(&state, &storage);
    let file_path = storage.path().join(url.trim_start_matches('/'));
    tokio::fs::remove_file(&file_path)
        .await
        .expect("remove seeded media file");

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(url)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends_matrix)]
#[case::missing_file(
    "/media/upload/ab/cd/abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/missing.jpg"
)]
#[tokio::test]
async fn serve_returns_404_for_valid_absent_address(backend: Backend, #[case] uri: &str) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends_matrix)]
#[case::invalid_source(
    "/media/not-a-source/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/file.txt"
)]
#[case::short_hash("/media/upload/e3/b0/a/file.txt")]
#[case::non_hex_hash(
    "/media/upload/zz/zz/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz/file.txt"
)]
#[case::encoded_separator(
    "/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/a%2Fb.txt"
)]
#[case::p1_mismatch(
    "/media/upload/ff/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/file.txt"
)]
#[case::short_p1(
    "/media/upload/e/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/file.txt"
)]
#[case::p2_mismatch(
    "/media/upload/e3/ff/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/file.txt"
)]
#[tokio::test]
async fn serve_rejects_malformed_address_before_handler(backend: Backend, #[case] uri: &str) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let response = make_app(&state, &storage)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// guard:no-backend — pure constructed I/O classification
#[tokio::test]
async fn media_open_classifies_only_not_found_as_404_and_reports_other_io_once() {
    let missing = jaunder::media::classify_media_open_error(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "missing sentinel",
    ));
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(
        std::error::Error::source(&missing).is_none(),
        "expected absence is not an internal source"
    );

    let denied = jaunder::media::classify_media_open_error(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "permission sentinel",
    ));
    assert_eq!(denied.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let source = std::error::Error::source(&denied)
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .expect("PermissionDenied remains a typed I/O source");
    assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(source.to_string(), "permission sentinel");

    let ((), event) = crate::assert_error_signal!(
        async { denied.emit_boundary_failure() },
        event = "server function failed",
        event_kind = "Internal",
        event_class = "Bug",
        metric_kind = "internal",
        metric_class = "bug",
        disposition = "boundary",
        context = ""
    );
    assert!(
        event.contains("permission sentinel"),
        "typed source: {event}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn serve_returns_304_on_if_none_match(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let cookie = create_user_and_session(&state).await.cookie();

    let storage = TempDir::new().unwrap();

    // Upload via the `upload_media` server fn so a file lands on `storage`'s disk.
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "etag_test.png",
            content_type: "image/png",
            bytes: b"PNG_DATA",
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let upload_json: serde_json::Value = confirmed_mutation(&body);
    let url = upload_json["url"].as_str().unwrap().to_owned();
    let sha256 = upload_json["sha256"].as_str().unwrap().to_owned();
    // The ETag the serve handler now emits (sha256-prefixed) — built via the door so the
    // expectation tracks the producer.
    let etag = from_content_hash(&parse_content_hash(&sha256));

    let app = make_app(&state, &storage);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .header(header::IF_NONE_MATCH, etag.as_ref())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
}

// ---------------------------------------------------------------------------
// Proxy tests
// ---------------------------------------------------------------------------

#[apply(backends)]
#[tokio::test]
async fn proxy_rejects_unauthenticated_malformed_url_before_query_extraction(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/media/proxy?url=not-a-url&user_id=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[apply(backends)]
#[tokio::test]
async fn proxy_redirects_authenticated_to_canonical_location(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

    let url = format!(
        "/media/proxy?url=HTTP%3A%2F%2FEXAMPLE.COM%3A80&user_id={}",
        session.user_id
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "http://example.com/"
    );
}

#[apply(backends_matrix)]
#[case::malformed("not-a-url")]
#[case::relative("%2Fimage.jpg")]
#[case::empty_host("http%3A%2F%2F")]
#[case::non_http("%66tp%3A%2F%2Fexample.com%2Fimage.jpg")]
#[tokio::test]
async fn proxy_rejects_authenticated_invalid_url(backend: Backend, #[case] encoded_url: &str) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);
    let url = format!("/media/proxy?url={encoded_url}&user_id={}", session.user_id);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[apply(backends)]
#[tokio::test]
async fn proxy_rejects_mismatched_user_id(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state).await;
    let user_id = session.user_id;
    let cookie = session.cookie();

    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

    // Pass a different user_id in query params.
    let wrong_user_id = UserId::from(i64::from(user_id) + 999);
    let url =
        format!("/media/proxy?url=http%3A%2F%2Fexample.com%2Fimage.jpg&user_id={wrong_user_id}");

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
