use std::{fmt::Write as _, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{MutationOutcome, theme::ThemeContentDigest};
use rstest::*;
use rstest_reuse::*;
use storage::{
    ThemeAssetManager, ThemeDraft, ThemeDraftAsset, ThemeOwner,
    test_support::{
        Backend, TestEnv, backends, compiled_theme_fixture, confirmed_for, create_site_theme,
        theme_quota_limits,
    },
};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::helpers::{create_user_and_session, make_app};

struct PublishedFixture {
    stylesheet_digest: String,
    stylesheet: Vec<u8>,
    image_digest: String,
    image: Vec<u8>,
}

fn hex_digest(bytes: [u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

async fn publish_fixture(state: &Arc<storage::AppState>, storage: &TempDir) -> PublishedFixture {
    let compiled = compiled_theme_fixture();
    let stylesheet_digest = hex_digest(compiled.css().digest());
    let stylesheet = compiled.css().bytes().to_vec();
    let (_, _, image, image_digest) = compiled.assets().next().expect("fixture contains image");
    let image_digest = hex_digest(image_digest);
    let image = image.to_vec();
    let theme_id = create_site_theme(
        Arc::clone(&state.themes),
        state.write_scope.clone(),
        &compiled,
    )
    .await;
    let manager = ThemeAssetManager::new(
        Arc::clone(&state.themes),
        state.write_scope.clone(),
        Arc::new(storage.path().to_path_buf()),
    );
    let outcome = manager
        .publish(
            ThemeOwner::Site,
            theme_id,
            &compiled,
            theme_quota_limits(i64::MAX),
            0,
        )
        .await
        .expect("publish fixture");
    assert!(matches!(outcome, MutationOutcome::Confirmed(_)));

    PublishedFixture {
        stylesheet_digest,
        stylesheet,
        image_digest,
        image,
    }
}

async fn get(app: &axum::Router, uri: String) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router response")
}

#[apply(backends)]
#[tokio::test]
async fn author_draft_asset_is_private_to_its_owner(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let owner = create_user_and_session(&state).await;
    let stranger = create_user_and_session(&state).await;
    let asset = ThemeDraftAsset {
        path: "assets/private.png".to_owned(),
        mime: "image/png".to_owned(),
        bytes: vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31,
            0, 2, 235, 1, 245, 105, 91, 156, 64, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ],
        digest: "b".repeat(64).parse().expect("valid asset digest"),
    };
    let draft = ThemeDraft {
        theme_id: 0.into(),
        manifest: br#"{"assets":{"assets/private.png":"image/png"},"defaults":{},"name":"Private","schema":1,"style_contract":1}"#.to_vec(),
        stylesheet: b"body { color: black; }".to_vec(),
        source_digest: "a".repeat(64).parse().expect("valid source digest"),
        assets: vec![asset.clone()],
    };
    let themes = Arc::clone(&state.themes);
    let owner_id = owner.user_id;
    let theme_id = confirmed_for(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .create_theme(
                            transaction,
                            ThemeOwner::Author(owner_id),
                            "Private",
                            &draft,
                            theme_quota_limits(i64::MAX),
                        )
                        .await
                })
            })
            .await
            .expect("create author draft"),
        "author draft creation",
    );
    let storage = TempDir::new().expect("temporary content root");
    let app = make_app(&state, &storage);
    let uri = format!("/theme/draft/{theme_id}/{}", asset.path);

    let owner_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&uri)
                .header(header::COOKIE, owner.cookie())
                .body(Body::empty())
                .expect("owner request builds"),
        )
        .await
        .expect("owner router response");
    assert_eq!(owner_response.status(), StatusCode::OK);
    assert_eq!(owner_response.headers()[header::CONTENT_TYPE], asset.mime);
    assert_eq!(
        owner_response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert_eq!(
        owner_response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    let owner_body = axum::body::to_bytes(owner_response.into_body(), usize::MAX)
        .await
        .expect("owner response body");
    assert_eq!(owner_body.as_ref(), asset.bytes);

    for (request, expected_status) in [
        (
            Request::builder()
                .uri(&uri)
                .header(header::COOKIE, stranger.cookie())
                .body(Body::empty())
                .expect("stranger request builds"),
            StatusCode::NOT_FOUND,
        ),
        (
            Request::builder()
                .uri(&uri)
                .body(Body::empty())
                .expect("anonymous request builds"),
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("denied router response");
        assert_eq!(
            response.status(),
            expected_status,
            "draft asset denial must not disclose cross-owner existence"
        );
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn public_theme_content_serves_stored_css_and_image_with_immutable_headers(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let storage = TempDir::new().expect("temporary content root");
    let fixture = publish_fixture(&state, &storage).await;
    let app = make_app(&state, &storage);

    for (digest, expected_mime, expected_body) in [
        (
            fixture.stylesheet_digest.as_str(),
            "text/css; charset=utf-8",
            fixture.stylesheet.as_slice(),
        ),
        (
            fixture.image_digest.as_str(),
            "image/png",
            fixture.image.as_slice(),
        ),
    ] {
        let response = get(&app, format!("/theme/{digest}")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], expected_mime);
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        assert_eq!(
            response.headers()[header::ETAG],
            format!("\"sha256-{digest}\"")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        assert_eq!(body.as_ref(), expected_body);
    }
    let encoded_alias = format!(
        "/theme/%{:02X}{}",
        fixture.stylesheet_digest.as_bytes()[0],
        &fixture.stylesheet_digest[1..]
    );
    assert_eq!(
        get(&app, encoded_alias).await.status(),
        StatusCode::NOT_FOUND,
        "percent-encoded aliases must not reach public immutable content"
    );
}

#[apply(backends)]
#[tokio::test]
async fn public_theme_content_returns_not_modified_for_exact_etag(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let storage = TempDir::new().expect("temporary content root");
    let fixture = publish_fixture(&state, &storage).await;
    let app = make_app(&state, &storage);
    let etag = format!("\"sha256-{}\"", fixture.stylesheet_digest);
    for condition in [etag.clone(), format!("\"other\", W/{etag}"), "*".to_owned()] {
        let request = Request::builder()
            .uri(format!("/theme/{}", fixture.stylesheet_digest))
            .header(header::IF_NONE_MATCH, condition)
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(request).await.expect("router response");
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(response.headers()[header::ETAG], etag);
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert!(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("empty response body")
                .is_empty()
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn only_eligible_theme_content_is_public(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let storage = TempDir::new().expect("temporary content root");
    let digest = "a".repeat(64);
    let file = storage
        .path()
        .join("themes")
        .join(&digest[..2])
        .join(&digest[2..4])
        .join(&digest);
    std::fs::create_dir_all(file.parent().expect("content parent")).expect("create content parent");
    std::fs::write(&file, b"draft css").expect("write known draft bytes");
    let app = make_app(&state, &storage);

    assert_eq!(
        get(&app, format!("/theme/{digest}")).await.status(),
        StatusCode::NOT_FOUND
    );

    let typed_digest: ThemeContentDigest = digest.parse().expect("canonical digest");
    let themes = Arc::clone(&state.themes);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .upsert_content_eligibility(
                        transaction,
                        &storage::ThemeContentEligibility {
                            digest: typed_digest,
                            mime: "text/css; charset=utf-8".into(),
                            retained_until_unix_seconds: 0,
                        },
                    )
                    .await
            })
        })
        .await
        .expect("admit content");
    confirmed_for(outcome, "integration test backend");

    let response = get(&app, format!("/theme/{digest}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body")
            .as_ref(),
        b"draft css"
    );
    std::fs::remove_file(&file).expect("remove eligible backing bytes");
    assert_eq!(
        get(&app, format!("/theme/{digest}")).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "eligible missing content is an operational failure, not public absence"
    );
}

#[apply(backends)]
#[tokio::test]
async fn removed_theme_content_remains_public_through_retention(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let storage = TempDir::new().expect("temporary content root");
    let fixture = publish_fixture(&state, &storage).await;
    let theme = state
        .themes
        .list_themes(ThemeOwner::Site)
        .await
        .expect("list published theme")
        .into_iter()
        .next()
        .expect("published theme");
    let themes = Arc::clone(&state.themes);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .remove_theme(transaction, ThemeOwner::Site, theme.id, 100)
                    .await
            })
        })
        .await
        .expect("remove theme with retention");
    confirmed_for(outcome, "integration test backend");

    let app = make_app(&state, &storage);
    let response = get(&app, format!("/theme/{}", fixture.image_digest)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body")
            .as_ref(),
        fixture.image.as_slice()
    );
}

#[apply(backends)]
#[tokio::test]
async fn public_theme_content_rejects_noncanonical_addresses(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let storage = TempDir::new().expect("temporary content root");
    let app = make_app(&state, &storage);
    for address in [
        "A".repeat(64),
        "a".repeat(63),
        format!("{}/extra", "a".repeat(64)),
    ] {
        assert_eq!(
            get(&app, format!("/theme/{address}")).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
