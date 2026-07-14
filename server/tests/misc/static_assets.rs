use axum::http::StatusCode;

use crate::helpers::get_asset;

// guard:no-backend — drives the real asset router via create_router/oneshot to
// serve an embedded static asset; exercises no database.
#[tokio::test]
async fn test_jaunder_css_served() {
    let (status, content_type) = get_asset("/style/jaunder.css").await;
    assert_eq!(status, StatusCode::OK);
    let ct = content_type.expect("content-type header should be present");
    assert!(
        ct.contains("text/css"),
        "expected content-type to contain text/css, got: {ct}"
    );
}

// guard:no-backend — drives the real asset router via create_router/oneshot to
// serve an embedded static asset; exercises no database.
#[tokio::test]
async fn test_jaunder_themes_css_served() {
    let (status, content_type) = get_asset("/style/jaunder-themes.css").await;
    assert_eq!(status, StatusCode::OK);
    let ct = content_type.expect("content-type header should be present");
    assert!(
        ct.contains("text/css"),
        "expected content-type to contain text/css, got: {ct}"
    );
}
