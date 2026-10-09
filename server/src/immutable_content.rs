//! Shared HTTP response contract for immutable digest-addressed content.

use std::io;

use axum::{
    body::Body,
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use web::error::InternalError;

const CACHE_CONTROL: HeaderValue = HeaderValue::from_static("public, max-age=31536000, immutable");
const NOSNIFF: HeaderValue = HeaderValue::from_static("nosniff");

/// Builds an immutable content response with its validator and MIME safeguards.
///
/// `invalid_mime_boundary` preserves the serving consumer's error attribution
/// when storage has supplied a MIME value that cannot be represented in HTTP.
pub(crate) fn response(
    status: StatusCode,
    body: Body,
    mime: &str,
    etag: &common::etag::ETag,
    invalid_mime_boundary: &'static str,
) -> Result<Response, StatusCode> {
    let content_type = HeaderValue::from_str(mime).map_err(|_| {
        InternalError::server(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid stored theme MIME",
        ))
        .with_context("boundary", invalid_mime_boundary)
        .emit_boundary_failure();
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let etag =
        HeaderValue::from_str(etag.as_ref()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, NOSNIFF);
    headers.insert(header::CACHE_CONTROL, CACHE_CONTROL);
    headers.insert(header::ETAG, etag);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{StatusCode, header},
    };

    use super::response;

    const DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ERROR_BOUNDARY: &str = "server.immutable_content.tests";

    fn etag() -> common::etag::ETag {
        host::etag::from_content_hash(&DIGEST.parse().expect("valid content hash"))
    }

    #[tokio::test]
    async fn response_has_immutable_content_headers_and_body() {
        let response = response(
            StatusCode::OK,
            Body::from("stylesheet"),
            "text/css; charset=utf-8",
            &etag(),
            ERROR_BOUNDARY,
        )
        .expect("valid immutable response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/css; charset=utf-8"
        );
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
            "\"sha256-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\""
        );
        assert_eq!(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("response body"),
            "stylesheet"
        );
    }

    #[tokio::test]
    async fn not_modified_response_retains_immutable_headers_and_empty_body() {
        let response = response(
            StatusCode::NOT_MODIFIED,
            Body::empty(),
            "text/css; charset=utf-8",
            &etag(),
            ERROR_BOUNDARY,
        )
        .expect("valid immutable response");

        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
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

    #[test]
    fn response_rejects_invalid_stored_mime_without_serving_content() {
        assert!(matches!(
            response(
                StatusCode::OK,
                Body::empty(),
                "invalid\r\nmime",
                &etag(),
                ERROR_BOUNDARY,
            ),
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        ));
    }
}
