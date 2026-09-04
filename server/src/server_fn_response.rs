//! Owns the HTTP policy for Leptos server-function responses.
//!
//! `server_fn` encodes every application error as HTTP 500, including typed
//! argument decode and forbidden-operation errors. This adapter consumes `WebError`'s
//! classifications at the single `/api` seam, emits the stable public error body with the
//! matching HTTP status, and removes redirects that Leptos adds to malformed
//! progressive-enhancement requests. It inspects only exact-sized, already
//! buffered framework error bodies; successful and streaming responses pass
//! through untouched.

use axum::{
    body::{self, Body, HttpBody},
    extract::Request,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use leptos::server_fn::error;
use web::error::WebError;
pub(crate) async fn handle_with_context(
    additional_context: impl Fn() + 'static + Clone + Send,
    request: Request,
) -> Response {
    normalize(leptos_axum::handle_server_fns_with_context(additional_context, request).await).await
}

pub(crate) async fn normalize(response: impl IntoResponse) -> Response {
    let response = response.into_response();
    if !response
        .headers()
        .contains_key(error::SERVER_FN_ERROR_HEADER)
    {
        return response;
    }

    let Some(length) = response.body().size_hint().exact() else {
        return response;
    };
    let limit = usize::try_from(length).unwrap_or(usize::MAX);

    let (mut parts, body) = response.into_parts();
    let Ok(body) = body::to_bytes(body, limit).await else {
        unreachable!("server_fn constructs error responses from in-memory bytes");
    };
    let status = WebError::server_fn_error_status(&body);
    let body = WebError::normalize_server_fn_error_body(body.clone()).unwrap_or(body);
    let Some(status) = status else {
        return Response::from_parts(parts, Body::from(body));
    };

    let (status, remove_location) = match status {
        400 => (StatusCode::BAD_REQUEST, true),
        403 => (StatusCode::FORBIDDEN, false),
        // `WebError::server_fn_error_status` only returns `Some(400)`, `Some(403)`, or
        // `None`; the preceding `let Some` has already excluded `None`.
        _ => return Response::from_parts(parts, Body::from(body)), // cov:ignore
    };
    parts.status = status;
    if remove_location {
        parts.headers.remove(header::LOCATION);
    }
    Response::from_parts(parts, Body::from(body))
}

#[cfg(test)]
mod tests {
    use super::normalize;
    use std::convert::Infallible;

    use axum::{
        body::{Body, Bytes, to_bytes},
        http::{StatusCode, header::LOCATION},
        response::Response,
    };
    use leptos::server_fn::error::{FromServerFnError, SERVER_FN_ERROR_HEADER, ServerFnErrorErr};
    use web::error::WebError;

    fn error_response(error: ServerFnErrorErr) -> Response {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(SERVER_FN_ERROR_HEADER, "/api/test/error")
            .body(Body::from(WebError::from_server_fn_error(error).ser()))
            .unwrap()
    }

    #[tokio::test]
    async fn malformed_input_becomes_bad_request_without_changing_public_body() {
        for error in [
            ServerFnErrorErr::Args("bad argument".to_string()),
            ServerFnErrorErr::MissingArg("missing".to_string()),
            ServerFnErrorErr::Deserialization("bad input".to_string()),
        ] {
            let expected = WebError::server_function(error.to_string());
            let mut response = error_response(error);
            response
                .headers_mut()
                .insert(LOCATION, "/form".parse().unwrap());

            let response = normalize(response).await;

            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert!(!response.headers().contains_key(LOCATION));
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(WebError::de(body), expected);
        }
    }

    #[tokio::test]
    async fn forbidden_application_error_becomes_forbidden_without_rewriting_body() {
        let expected = WebError::forbidden("media uploads are disabled");
        let response = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(SERVER_FN_ERROR_HEADER, "/api/test/error")
            .body(Body::from(expected.clone().ser()))
            .unwrap();

        let response = normalize(response).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(WebError::de(body), expected);
    }

    #[tokio::test]
    async fn streaming_error_body_passes_through() {
        let body = Body::from_stream(futures_util::stream::once(async {
            Ok::<_, Infallible>(Bytes::from_static(b"streaming"))
        }));
        let response = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(SERVER_FN_ERROR_HEADER, "/api/test/error")
            .header(LOCATION, "/form")
            .body(body)
            .unwrap();

        let response = normalize(response).await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.headers()[LOCATION], "/form");
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "streaming");
    }

    #[tokio::test]
    async fn output_serialization_remains_an_internal_server_error() {
        let response = error_response(ServerFnErrorErr::Serialization("bad output".to_string()));

        let response = normalize(response).await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            WebError::de(body),
            WebError::server_function(
                ServerFnErrorErr::Serialization("bad output".to_string()).to_string()
            )
        );
    }
}
