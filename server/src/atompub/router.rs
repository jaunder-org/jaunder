use axum::Router;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};

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
        .route("/~{username}/rsd.xml", get(rsd::rsd_document))
        .layer(axum::middleware::from_fn(record_atompub_request))
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
    use super::{atompub_op, atompub_result};
    use axum::http::{Method, StatusCode};

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
