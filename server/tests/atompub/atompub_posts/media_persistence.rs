use axum::http::StatusCode;
use common::ids::PostId;
use common::media::MediaReferenceKind;
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{atompub_post_xml, create_user_and_session, make_app, setup_with_base_url};
use storage::test_support::{
    Backend, TestEnv, backends, fetch_post_media, media_ref_for, media_url_for,
};

use super::fixtures::{entry_xml, location_post_id};

#[apply(backends)]
#[tokio::test]
async fn create_writes_the_entrys_media_rows(#[case] backend: Backend) {
    // The AtomPub half of A14 (#711). The storage tests cover the web path; this one
    // drives the router, because the AtomPub handler reaches storage by its own route
    // and nothing in `storage` would notice if that path stopped recording references.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    // An `html` entry, so the escaped `<img>` is unescaped into the stored body and the
    // renderer sees a real element rather than literal text.
    let content = format!("&lt;img src=\"{}\"&gt;", media_url_for("photo.jpg"));
    let xml = entry_xml("Photo entry", "html", &content);

    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = PostId::from(location_post_id(&response));

    assert_eq!(
        fetch_post_media(&base, post_id).await,
        vec![(
            media_ref_for("photo.jpg"),
            MediaReferenceKind::Local,
            media_url_for("photo.jpg")
                .parse()
                .expect("valid media reference form"),
        )]
    );
}
