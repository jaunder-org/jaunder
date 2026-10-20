use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use common::ids::UserId;
use common::test_support::parse_content_hash;
use host::etag::from_content_hash;
use server_fn::ServerFn;
use storage::test_support::{Backend, backends, backends_matrix};

use crate::helpers::{
    MultipartFile, body_string, confirmed_mutation, create_user_and_session, make_app,
    post_multipart,
};

const CLEAN_JPEG: &[u8] = include_bytes!("../fixtures/image-privacy/jpeg-clean.jpg");

async fn running_image_worker(
    control: &host::image_privacy::WorkerTestControl,
    spool: &std::path::Path,
) -> u32 {
    let pid = control.started().await.expect("real child PID");
    let started = std::time::Instant::now();
    while !spool.join(host::image_privacy::READY_FILE).exists() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "real worker did not enter its controlled job"
        );
        tokio::task::yield_now().await;
    }
    assert!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid.try_into().unwrap()), None).is_ok(),
        "the witness needs a running child"
    );
    pid
}

fn assert_image_worker_reaped(pid: u32) {
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid.try_into().unwrap()), None),
        Err(nix::errno::Errno::ESRCH),
        "owned child still exists after resource release"
    );
}

#[rstest]
#[case::cancelled_request(true)]
#[case::shutdown(false)]
// guard:no-backend — real same-binary child, no application storage.
#[tokio::test(flavor = "current_thread")]
async fn real_worker_cancellation_and_shutdown_reap_before_cleanup(#[case] cancel: bool) {
    use host::image_privacy::{
        PreparationError, UploadPreparer, WorkerTestControl, WorkerTestMode,
    };
    let root = TempDir::new().unwrap();
    let control = WorkerTestControl::new(WorkerTestMode::Paused);
    let preparer =
        UploadPreparer::new_for_test(env!("CARGO_BIN_EXE_jaunder").into(), Arc::clone(&control));
    let mut first = preparer.admit(root.path()).unwrap();
    first.write(CLEAN_JPEG).unwrap();
    let spool = first.output().parent().unwrap().to_path_buf();
    let second = preparer.admit(root.path()).unwrap();
    let mut caller = Some(tokio::spawn(async move {
        let result = first.prepare("image/jpeg").await;
        let cleanup = first.finish().await;
        (result, cleanup)
    }));
    let pid = running_image_worker(&control, &spool).await;
    if cancel {
        let caller = caller.take().unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
    } else {
        preparer.stop();
    }
    control.waiting_for_reap().await;
    if cancel {
        assert!(matches!(
            preparer.admit(root.path()),
            Err(PreparationError::Busy)
        ));
    } else {
        assert!(matches!(
            preparer.admit(root.path()),
            Err(PreparationError::Stopping)
        ));
    }
    assert!(
        spool.join("input").exists(),
        "private input released before child wait"
    );
    control.release_reap();
    if let Some(caller) = caller {
        let (result, cleanup) = caller.await.unwrap();
        assert!(matches!(result, Err(PreparationError::Stopping)));
        cleanup.unwrap();
    }
    second.finish().await.unwrap();
    preparer.shutdown().await.unwrap();
    assert_image_worker_reaped(pid);
    assert!(
        std::fs::read_dir(root.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
}

// guard:no-backend — advances only the application's real thirty-second timer around a real child.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn real_worker_deadline_uses_thirty_seconds_and_retains_owned_resources() {
    use host::image_privacy::{
        PreparationError, UploadPreparer, WorkerTestControl, WorkerTestMode,
    };
    let root = TempDir::new().unwrap();
    let control = WorkerTestControl::new(WorkerTestMode::Paused);
    let preparer =
        UploadPreparer::new_for_test(env!("CARGO_BIN_EXE_jaunder").into(), Arc::clone(&control));
    let mut first = preparer.admit(root.path()).unwrap();
    first.write(CLEAN_JPEG).unwrap();
    let spool = first.output().parent().unwrap().to_path_buf();
    let second = preparer.admit(root.path()).unwrap();
    let started = tokio::time::Instant::now();
    let caller = tokio::spawn(async move {
        let result = first.prepare("image/jpeg").await;
        let cleanup = first.finish().await;
        (result, cleanup)
    });
    let pid = running_image_worker(&control, &spool).await;
    tokio::time::advance(std::time::Duration::from_secs(29)).await;
    let mut reaping = Box::pin(control.waiting_for_reap());
    assert!(
        futures_util::poll!(&mut reaping).is_pending(),
        "processing deadline fired before thirty seconds"
    );
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    reaping.await;
    assert_eq!(
        tokio::time::Instant::now() - started,
        std::time::Duration::from_secs(30),
        "the production deadline must fire at thirty seconds, not later"
    );
    tokio::time::resume();
    assert!(matches!(
        preparer.admit(root.path()),
        Err(PreparationError::Busy)
    ));
    assert!(
        spool.join("input").exists(),
        "deadline released private input before wait"
    );
    control.release_reap();
    let (result, cleanup) = caller.await.unwrap();
    assert!(matches!(result, Err(PreparationError::Deadline)));
    cleanup.unwrap();
    second.finish().await.unwrap();
    preparer.shutdown().await.unwrap();
    assert_image_worker_reaped(pid);
    assert!(
        std::fs::read_dir(root.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
}

// guard:no-backend — a real owned child is crashed; only its execution owner performs wait/reap.
#[tokio::test(flavor = "current_thread")]
async fn real_worker_crash_retains_exit_status_and_cleans_owned_files() {
    use host::image_privacy::{
        PreparationError, UploadPreparer, WorkerTestControl, WorkerTestMode,
    };
    use std::os::unix::process::ExitStatusExt;
    let root = TempDir::new().unwrap();
    let control = WorkerTestControl::new(WorkerTestMode::Paused);
    let preparer =
        UploadPreparer::new_for_test(env!("CARGO_BIN_EXE_jaunder").into(), Arc::clone(&control));
    let mut attempt = preparer.admit(root.path()).unwrap();
    attempt.write(CLEAN_JPEG).unwrap();
    let spool = attempt.output().parent().unwrap().to_path_buf();
    let caller = tokio::spawn(async move {
        let result = attempt.prepare("image/jpeg").await;
        let cleanup = attempt.finish().await;
        (result, cleanup)
    });
    let pid = running_image_worker(&control, &spool).await;
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid.try_into().unwrap()),
        nix::sys::signal::Signal::SIGKILL,
    )
    .unwrap();
    let (result, cleanup) = caller.await.unwrap();
    assert!(
        matches!(result, Err(PreparationError::WorkerExit(status)) if status.signal() == Some(nix::libc::SIGKILL))
    );
    cleanup.unwrap();
    preparer.shutdown().await.unwrap();
    assert_image_worker_reaped(pid);
    assert!(!spool.exists());
}

// guard:no-backend — the actual child emits a response exceeding the production bound.
#[tokio::test]
async fn real_worker_response_bound_rejects_and_reaps_before_cleanup() {
    use host::image_privacy::{
        PreparationError, UploadPreparer, WorkerTestControl, WorkerTestMode,
    };
    let root = TempDir::new().unwrap();
    let control = WorkerTestControl::new(WorkerTestMode::OversizedResponse);
    let preparer =
        UploadPreparer::new_for_test(env!("CARGO_BIN_EXE_jaunder").into(), Arc::clone(&control));
    let mut attempt = preparer.admit(root.path()).unwrap();
    attempt.write(CLEAN_JPEG).unwrap();
    let result = attempt.prepare("image/jpeg").await;
    let pid = control.started().await.unwrap();
    assert!(matches!(result, Err(PreparationError::Protocol)));
    attempt.finish().await.unwrap();
    preparer.shutdown().await.unwrap();
    assert_image_worker_reaped(pid);
    assert!(
        std::fs::read_dir(root.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
}

// guard:no-backend — current-thread scheduling fixes start/stop ordering without sleeps.
#[tokio::test(flavor = "current_thread")]
async fn queued_image_preparation_returns_stopping_before_shutdown_waits_for_lease() {
    use host::image_privacy::{PreparationError, UploadPreparer};
    let root = TempDir::new().expect("media root");
    let preparer = UploadPreparer::new(env!("CARGO_BIN_EXE_jaunder").into());
    let mut attempt = preparer.admit(root.path()).expect("admitted attempt");
    attempt.write(CLEAN_JPEG).expect("private input");
    let mut preparing = Box::pin(attempt.prepare("image/jpeg"));
    // The current-thread owner cannot run until we yield. This poll queues the
    // start message; stop is made ready before the owner's first selection.
    assert!(futures_util::poll!(&mut preparing).is_pending());
    preparer.stop();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), preparing).await;
    attempt.finish().await.expect("owned spool cleaned");
    preparer.shutdown().await.expect("owner joined");
    assert!(
        matches!(result, Ok(Err(PreparationError::Stopping))),
        "queued preparation did not return stopping before lease cleanup"
    );
    assert!(
        std::fs::read_dir(root.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
}

// guard:no-backend — private admission and shutdown own only files and job slots.
#[tokio::test]
async fn image_jobs_are_two_slot_private_and_shutdown_cleans_receiving_attempts() {
    use host::image_privacy::{PreparationError, UploadPreparer};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().expect("media root");
    let preparer = UploadPreparer::new(env!("CARGO_BIN_EXE_jaunder").into());
    let first = preparer.admit(root.path()).expect("first slot");
    let second = preparer.admit(root.path()).expect("second slot");
    assert!(matches!(
        preparer.admit(root.path()),
        Err(PreparationError::Busy)
    ));
    #[cfg(unix)]
    {
        let spool = first.output().parent().expect("owned spool");
        assert_eq!(
            std::fs::metadata(spool).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for name in ["input", "output"] {
            assert_eq!(
                std::fs::metadata(spool.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    preparer.stop();
    first
        .finish()
        .await
        .expect("first receiving attempt cleaned");
    second
        .finish()
        .await
        .expect("second receiving attempt cleaned");
    preparer.shutdown().await.expect("all owners joined");
    assert!(matches!(
        preparer.admit(root.path()),
        Err(PreparationError::Stopping)
    ));
    assert!(
        std::fs::read_dir(root.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[cfg(unix)]
// guard:no-backend — filesystem fault injection observes owned cleanup and slot retention.
#[tokio::test]
async fn image_cleanup_failure_retains_slot_until_owned_spool_is_removed() {
    use host::image_privacy::{PreparationError, UploadPreparer};
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().expect("media root");
    let preparer = UploadPreparer::new(env!("CARGO_BIN_EXE_jaunder").into());
    let mut first = preparer.admit(root.path()).expect("first slot");
    first.write(CLEAN_JPEG).expect("private input");
    let second = preparer.admit(root.path()).expect("second slot");
    let spool = first.output().parent().unwrap().to_path_buf();
    let temporary = root.path().join("media/tmp");
    std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o500)).unwrap();
    let finishing = tokio::spawn(first.finish());
    // Deleting the input proves the owner attempted cleanup; removing the spool
    // itself then needs write permission on its parent, which is faulted above.
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while spool.join("input").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cleanup reaches the injected parent failure");
    let still_owned = matches!(preparer.admit(root.path()), Err(PreparationError::Busy));
    // Restore permissions before assertions so a red test cannot strand its fixture.
    std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let cleanup_error = finishing
        .await
        .unwrap()
        .expect_err("retain the actual cleanup failure");
    assert!(matches!(cleanup_error, PreparationError::Io(_)));
    second.finish().await.expect("second attempt cleaned");
    preparer.shutdown().await.expect("cleanup owners joined");
    assert!(
        still_owned,
        "failed cleanup released its slot before removing the owned spool"
    );
    assert!(
        !spool.exists(),
        "owned spool remains after cleanup completion"
    );
}

// guard:no-backend — the actual application worker returns a typed native I/O cause.
#[tokio::test]
async fn image_worker_native_io_failure_retains_source_and_cleans_private_input() {
    use host::image_privacy::{PreparationError, UploadPreparer};
    use std::error::Error as _;
    let root = TempDir::new().expect("media root");
    let preparer = UploadPreparer::new(env!("CARGO_BIN_EXE_jaunder").into());
    let mut attempt = preparer.admit(root.path()).expect("slot");
    attempt.write(CLEAN_JPEG).expect("private input");
    std::fs::remove_file(attempt.output().with_file_name("input")).expect("inject missing input");
    let error = attempt
        .prepare("image/jpeg")
        .await
        .expect_err("worker must reject missing input");
    assert!(matches!(error, PreparationError::Native(_)));
    let source = error
        .source()
        .expect("native source")
        .source()
        .expect("native I/O cause")
        .downcast_ref::<std::io::Error>()
        .expect("typed I/O cause");
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    attempt.finish().await.expect("failed worker spool cleaned");
    preparer.shutdown().await.expect("worker owner joined");
    assert!(
        std::fs::read_dir(root.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// Serve tests
// ---------------------------------------------------------------------------

#[apply(backends)]
#[tokio::test]
async fn serve_returns_200_with_cache_headers(#[case] backend: Backend) {
    let env = backend.setup().await;

    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    // Upload via the `upload_media` server fn so a file lands on `storage`'s disk;
    // the fn returns 200 with a confirmed `UploadedMedia` mutation payload.
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "serve_test.png",
            content_type: "image/png",
            bytes: CLEAN_JPEG,
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload must succeed");

    let upload_json: serde_json::Value = confirmed_mutation(&body);
    let url = upload_json["url"].as_str().unwrap().to_owned();

    // The same root-composed router serves the persisted file.

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
    let env = backend.setup().await;
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

    let response = make_app!(&env, &storage)
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
    let env = backend.setup().await;
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "disappearing.png",
            content_type: "image/png",
            bytes: CLEAN_JPEG,
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "seed upload");
    let upload: serde_json::Value = confirmed_mutation(&body);
    let url = upload["url"].as_str().expect("uploaded media URL");

    // The router was built while both metadata and bytes existed; remove only
    // the bytes to deterministically exercise the post-lookup disappearance
    // path on every platform without relying on chmod semantics under root.
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
    let env = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

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
    let env = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let response = make_app!(&env, &storage)
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
    let env = backend.setup().await;

    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    // Upload via the `upload_media` server fn so a file lands on `storage`'s disk.
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "etag_test.png",
            content_type: "image/png",
            bytes: CLEAN_JPEG,
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
    let env = backend.setup().await;

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

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
    assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none());
}

#[apply(backends)]
#[tokio::test]
async fn proxy_redirects_authenticated_to_canonical_location(#[case] backend: Backend) {
    let env = backend.setup().await;

    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let cookie = session.cookie();

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

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
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let cookie = session.cookie();
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
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
    let env = backend.setup().await;

    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let user_id = session.user_id;
    let cookie = session.cookie();

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

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
