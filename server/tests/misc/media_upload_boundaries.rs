//! Finite real-HTTP admission, size and failure controls. Worker faults are fixed
//! TEST-only modes of the actual application child, never a privacy adapter.
use std::{io, path::Path, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, Bytes},
    http::{Method, Request, StatusCode, header},
};
use common::media::{ContentHash, MediaSource, UserQuota};
use common::pagination::{PageOffset, RowLimit};
use common::test_support::parse_filename;
use host::image_privacy::{UploadPreparer, WorkerTestControl, WorkerTestMode};
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;
use sha2::{Digest, Sha256};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::helpers::{
    MultipartFile, SeededSession, atompub, body_string, create_user_and_session, make_app,
    post_multipart,
};

const JPEG: &[u8] = include_bytes!("../fixtures/image-privacy/jpeg-clean.jpg");

async fn upload(
    app: &Router,
    session: &SeededSession,
    web: bool,
    filename: &str,
    mime: &str,
    bytes: &[u8],
) -> (StatusCode, String) {
    if web {
        post_multipart(
            app.clone(),
            <web::media::Upload as ServerFn>::PATH,
            MultipartFile {
                filename,
                content_type: mime,
                bytes,
            },
            Some(&session.cookie()),
        )
        .await
    } else {
        let response = app
            .clone()
            .oneshot(
                atompub(session, Method::POST, "media")
                    .header(header::CONTENT_TYPE, mime)
                    .header("slug", filename)
                    .body(Body::from(bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        (response.status(), body_string(response).await)
    }
}

async fn unpublished(
    env: &TestEnv,
    app: &Router,
    root: &Path,
    session: &SeededSession,
    spools: usize,
) {
    assert_eq!(
        env.media()
            .get_user_upload_usage(session.user_id)
            .await
            .unwrap()
            .value(),
        0
    );
    assert!(
        env.media()
            .list_media(
                session.user_id,
                None,
                RowLimit::at_most(10),
                PageOffset::default()
            )
            .await
            .unwrap()
            .is_empty()
    );
    for source in ["upload", "cached"] {
        assert!(
            std::fs::read_dir(root.join("media").join(source))
                .unwrap()
                .next()
                .is_none(),
            "rejection created public media"
        );
    }
    assert_eq!(
        std::fs::read_dir(root.join("media/tmp")).unwrap().count(),
        spools,
        "rejection created or leaked a private attempt"
    );
    let hash = ContentHash::from_digest(Sha256::digest(JPEG).into());
    let original_url =
        common::media::url(&MediaSource::Upload, &hash, &parse_filename("rejected.jpg"));
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(original_url.as_ref())
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

fn web_size_error(status: StatusCode, body: &str) {
    // Validation retains the existing server-function HTTP status; the typed
    // payload distinguishes the actual size policy from a native/editor failure.
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body).unwrap(),
        serde_json::json!({"validation":{"message":"payload too large"}})
    );
}

#[apply(backends)]
#[tokio::test]
async fn http_uploads_are_busy_before_creating_a_third_private_attempt(#[case] backend: Backend) {
    let env = backend.setup().await;
    let root = TempDir::new().unwrap();
    let preparer = UploadPreparer::new(env!("CARGO_BIN_EXE_jaunder").into());
    let app = make_app!(&env, &root; preparer = Arc::clone(&preparer));
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    // Real receiving leases occupy both shared slots. No child has launched, so
    // this is admission proof, not another child-reaping witness.
    let first = preparer.admit(root.path()).unwrap();
    let second = preparer.admit(root.path()).unwrap();
    for (web, name, mime, bytes) in [
        (true, "rejected.jpg", "image/jpeg", JPEG),
        (false, "rejected.jpg", "image/jpeg", JPEG),
        (
            false,
            "ordinary.pdf",
            "application/pdf",
            b"%PDF-1.7\nowned document\n".as_slice(),
        ),
    ] {
        let (status, body) = tokio::time::timeout(
            Duration::from_secs(2),
            upload(&app, &session, web, name, mime, bytes),
        )
        .await
        .expect("busy request queued behind held leases");
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        if web {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&body).unwrap(),
                serde_json::json!({"unavailable":{"message":"media preparation is busy"}})
            );
        }
        unpublished(&env, &app, root.path(), &session, 2).await;
    }
    first.finish().await.unwrap();
    second.finish().await.unwrap();
    preparer.shutdown().await.unwrap();
    unpublished(&env, &app, root.path(), &session, 0).await;
}

#[apply(backends)]
#[tokio::test]
async fn received_input_cap_rejects_a_valid_atompub_image(#[case] backend: Backend) {
    let cap = JPEG.len() - 1;
    assert!(cap > 0);
    let env = backend
        .setup()
        .media_limits(cap.to_string().parse().unwrap(), UserQuota::default())
        .await;
    let root = TempDir::new().unwrap();
    let app = make_app!(&env, &root);
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let (status, body) = upload(&app, &session, false, "rejected.jpg", "image/jpeg", JPEG).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    unpublished(&env, &app, root.path(), &session, 0).await;
}

#[apply(backends_matrix)]
#[case::web(true)]
#[case::atompub(false)]
#[tokio::test]
async fn sanitized_output_cap_rejects_a_real_child_output_larger_than_its_admitted_input(
    backend: Backend,
    #[case] web: bool,
) {
    let env = backend
        .setup()
        .media_limits(
            JPEG.len().to_string().parse().unwrap(),
            UserQuota::default(),
        )
        .await;
    let root = TempDir::new().unwrap();
    let control = WorkerTestControl::new(WorkerTestMode::ExpandedOutput);
    let preparer =
        UploadPreparer::new_for_test(env!("CARGO_BIN_EXE_jaunder").into(), Arc::clone(&control));
    let app = make_app!(&env, &root; preparer = Arc::clone(&preparer));
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    // Input equals the positive cap. The fixed child fault copies that valid input
    // and appends exactly one byte, so only the distinct output cap can reject it.
    let (status, body) = upload(&app, &session, web, "rejected.jpg", "image/jpeg", JPEG).await;
    if web {
        web_size_error(status, &body);
    } else {
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(body.is_empty());
    }
    tokio::time::timeout(Duration::from_secs(2), control.started())
        .await
        .expect("input was rejected before the real output-producing child launched")
        .unwrap();
    preparer.shutdown().await.unwrap();
    unpublished(&env, &app, root.path(), &session, 0).await;
}

#[apply(backends)]
#[tokio::test]
async fn reported_verification_failure_has_no_atompub_publication_or_charge(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let root = TempDir::new().unwrap();
    let control = WorkerTestControl::new(WorkerTestMode::VerificationFailure);
    let preparer =
        UploadPreparer::new_for_test(env!("CARGO_BIN_EXE_jaunder").into(), Arc::clone(&control));
    let app = make_app!(&env, &root; preparer = Arc::clone(&preparer));
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let (status, body) = upload(&app, &session, false, "rejected.jpg", "image/jpeg", JPEG).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        !body.contains(root.path().to_str().unwrap()),
        "private worker path escaped outward"
    );
    tokio::time::timeout(Duration::from_secs(2), control.started())
        .await
        .expect("reported failure did not come from a launched real child")
        .unwrap();
    preparer.shutdown().await.unwrap();
    unpublished(&env, &app, root.path(), &session, 0).await;
}

#[apply(backends)]
#[tokio::test]
async fn actual_native_output_write_failure_has_no_web_publication_or_charge(
    #[case] backend: Backend,
) {
    use futures_util::{StreamExt, stream};
    let env = backend.setup().await;
    let root = TempDir::new().unwrap();
    let app = make_app!(&env, &root);
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let (release, receive) = tokio::sync::oneshot::channel();
    let boundary = "owned-native-write-fault";
    let head = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"rejected.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n"
    );
    let mut tail = JPEG.to_vec();
    tail.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let body =
        stream::iter([Ok::<_, io::Error>(Bytes::from(head))]).chain(stream::once(async move {
            receive.await.map_err(io::Error::other)?;
            Ok::<_, io::Error>(Bytes::from(tail))
        }));
    let request = Request::builder()
        .method(Method::POST)
        .uri(<web::media::Upload as ServerFn>::PATH)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header(header::COOKIE, session.cookie())
        .body(Body::from_stream(body))
        .unwrap();
    let uploading = tokio::spawn(app.clone().oneshot(request));
    let spool = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(entry) = std::fs::read_dir(root.path().join("media/tmp"))
                .unwrap()
                .next()
            {
                let path = entry.unwrap().path();
                if path.join("output").is_file() {
                    break path;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("streamed upload did not reach private admission");
    // Headers/framing have been read, but content is still gated. Fault only this
    // owned candidate before child launch; originals and public objects are untouched.
    std::fs::remove_file(spool.join("output")).unwrap();
    std::fs::create_dir(spool.join("output")).unwrap();
    release.send(()).unwrap();
    let response = uploading.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body_string(response).await).unwrap(),
        serde_json::json!({"server":{"message":"server operation failed"}})
    );
    unpublished(&env, &app, root.path(), &session, 0).await;
}
