use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Router, middleware};

use super::{media, posts, rsd, service};

/// Builds the `AtomPub` routes (mergeable into the main application router).
///
/// The handlers read shared state via `Extension`, so the routes are generic
/// over the application's router state type.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/atompub/service", get(service::service_document))
        .route(
            "/atompub/{username}/posts",
            get(posts::collection_get).post(posts::collection_post),
        )
        .route(
            "/atompub/{username}/posts/{post_id}",
            get(posts::member_get)
                .put(posts::member_put)
                .delete(posts::member_delete),
        )
        .route("/atompub/{username}/media", post(media::collection_post))
        .route(
            "/atompub/{username}/media/{sha}/{filename}",
            get(media::member_get).delete(media::member_delete),
        )
        .layer(middleware::from_fn(add_basic_auth_challenge))
        .route("/~{username}/rsd.xml", get(rsd::rsd_document))
        .layer(middleware::from_fn(record_atompub_request))
}

/// Keep intermediaries from encoding `AtomPub` responses and changing strong
/// Post `ETags` into validators that the server cannot accept on a later write.
pub(crate) async fn prevent_atompub_transformation(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let is_atompub = request.uri().path().starts_with("/atompub/");
    let mut response = next.run(request).await;
    if !is_atompub {
        return response;
    }
    let headers = response.headers_mut();
    let mut cache_control = Vec::new();
    for value in headers.get_all(header::CACHE_CONTROL) {
        if !cache_control.is_empty() {
            cache_control.extend_from_slice(b", ");
        }
        cache_control.extend_from_slice(value.as_bytes());
    }
    if !cache_control.is_empty() {
        cache_control.extend_from_slice(b", ");
    }
    cache_control.extend_from_slice(b"no-transform");
    if let Ok(value) = axum::http::HeaderValue::from_bytes(&cache_control) {
        headers.insert(header::CACHE_CONTROL, value);
    } else {
        // Preserve unusual existing directives if they cannot be joined.
        headers.append(
            header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-transform"),
        );
    }
    response
}

/// Projects authentication failures from protected `AtomPub` routes onto the
/// HTTP Basic challenge required by protocol clients.
async fn add_basic_auth_challenge(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut response = next.run(request).await;
    if response.status() == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static(r#"Basic realm="Jaunder AtomPub""#),
        );
    }
    response
}

/// Records `jaunder.atompub.requests{op, result}` for every routed `AtomPub`
/// request, deriving the bounded `op` from the matched route + method and the
/// `result` class from the response status. A single chokepoint so handlers stay
/// free of metric plumbing.
async fn record_atompub_request(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let op = atompub_op(
        request
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(axum::extract::MatchedPath::as_str),
        request.method(),
    );
    let response = next.run(request).await;
    if let Some(op) = op {
        host::metrics::atompub_request(op, atompub_result(response.status()));
    }
    response
}

/// Maps a matched route template + method to the bounded `op` attribute, or
/// `None` for anything outside the `AtomPub` surface.
fn atompub_op(matched_path: Option<&str>, method: &axum::http::Method) -> Option<&'static str> {
    use axum::http::Method;
    match (matched_path?, method) {
        ("/atompub/service", &Method::GET) => Some("service_document"),
        ("/atompub/{username}/posts", &Method::GET) => Some("collection_get"),
        ("/atompub/{username}/posts", &Method::POST) => Some("collection_post"),
        ("/atompub/{username}/posts/{post_id}", &Method::GET) => Some("member_get"),
        ("/atompub/{username}/posts/{post_id}", &Method::PUT) => Some("member_put"),
        ("/atompub/{username}/posts/{post_id}", &Method::DELETE) => Some("member_delete"),
        ("/atompub/{username}/media", &Method::POST) => Some("media_collection_post"),
        ("/atompub/{username}/media/{sha}/{filename}", &Method::GET) => Some("media_member_get"),
        ("/atompub/{username}/media/{sha}/{filename}", &Method::DELETE) => {
            Some("media_member_delete")
        }
        ("/~{username}/rsd.xml", &Method::GET) => Some("rsd_document"),
        _ => None,
    }
}

/// Classifies a response status into the bounded `result` attribute.
fn atompub_result(status: StatusCode) -> host::metrics::AtompubResult {
    if status.is_server_error() {
        host::metrics::AtompubResult::ServerError
    } else if status.is_client_error() {
        host::metrics::AtompubResult::ClientError
    } else {
        host::metrics::AtompubResult::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::{atompub_op, atompub_result, prevent_atompub_transformation};
    use axum::body::Body;
    use axum::http::{HeaderValue, Method, Request, StatusCode, header};
    use axum::{Router, middleware, response::Response, routing::get};
    use tower::ServiceExt;

    #[tokio::test]
    async fn no_transform_retains_all_existing_cache_restrictions() {
        let app = Router::new()
            .route(
                "/atompub/service",
                get(|| async {
                    let mut response = Response::new(Body::from("service"));
                    response
                        .headers_mut()
                        .append(header::CACHE_CONTROL, HeaderValue::from_static("private"));
                    response
                        .headers_mut()
                        .append(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                    response
                }),
            )
            .layer(middleware::from_fn(prevent_atompub_transformation));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/atompub/service")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store, no-transform"
        );
    }

    #[test]
    fn atompub_op_maps_every_route_and_method() {
        let cases = [
            ("/atompub/service", Method::GET, Some("service_document")),
            (
                "/atompub/{username}/posts",
                Method::GET,
                Some("collection_get"),
            ),
            (
                "/atompub/{username}/posts",
                Method::POST,
                Some("collection_post"),
            ),
            (
                "/atompub/{username}/posts/{post_id}",
                Method::GET,
                Some("member_get"),
            ),
            (
                "/atompub/{username}/posts/{post_id}",
                Method::PUT,
                Some("member_put"),
            ),
            (
                "/atompub/{username}/posts/{post_id}",
                Method::DELETE,
                Some("member_delete"),
            ),
            (
                "/atompub/{username}/media",
                Method::POST,
                Some("media_collection_post"),
            ),
            (
                "/atompub/{username}/media/{sha}/{filename}",
                Method::GET,
                Some("media_member_get"),
            ),
            (
                "/atompub/{username}/media/{sha}/{filename}",
                Method::DELETE,
                Some("media_member_delete"),
            ),
            ("/~{username}/rsd.xml", Method::GET, Some("rsd_document")),
        ];
        for (path, method, expected) in cases {
            assert_eq!(atompub_op(Some(path), &method), expected, "{path} {method}");
        }
        // Unmatched route/method and absent matched path both yield None.
        assert_eq!(atompub_op(Some("/atompub/service"), &Method::POST), None);
        assert_eq!(atompub_op(None, &Method::GET), None);
    }

    #[test]
    fn atompub_result_classifies_status_ranges() {
        use host::metrics::AtompubResult;
        assert!(matches!(atompub_result(StatusCode::OK), AtompubResult::Ok));
        assert!(matches!(
            atompub_result(StatusCode::CREATED),
            AtompubResult::Ok
        ));
        assert!(matches!(
            atompub_result(StatusCode::NOT_FOUND),
            AtompubResult::ClientError
        ));
        assert!(matches!(
            atompub_result(StatusCode::INTERNAL_SERVER_ERROR),
            AtompubResult::ServerError
        ));
    }
}
