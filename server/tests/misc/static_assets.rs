use axum::http::StatusCode;

use crate::helpers::get_asset;

// guard:no-backend — drives the real asset router via create_router/oneshot to
// prove retired stable stylesheet paths stop before the SPA fallback.
#[tokio::test]
async fn legacy_application_stylesheet_is_not_the_spa_shell() {
    let (status, content_type) = get_asset("/style/jaunder.css").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_ne!(content_type.as_deref(), Some("text/html; charset=utf-8"));
}

// guard:no-backend — drives the real asset router via create_router/oneshot to
// prove retired stable stylesheet paths stop before the SPA fallback.
#[tokio::test]
async fn legacy_theme_stylesheet_is_not_the_spa_shell() {
    let (status, content_type) = get_asset("/style/jaunder-themes.css").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_ne!(content_type.as_deref(), Some("text/html; charset=utf-8"));
}
