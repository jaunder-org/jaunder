//! Web-side server-function adapter contexts.
//!
//! This module deliberately owns only the pieces that `#[server]` bodies touch:
//! request-head extraction, response-option mutation, and CSR redirect signaling.
//! The Axum request handler and `server_fn::axum` dispatch live in `server`, the
//! composition boundary that owns routing. Keeping the split explicit preserves
//! ADR-0016's per-trait Leptos-context DI while avoiding a new router boundary in
//! `web`.

use std::sync::{Arc, RwLock};

use axum::{
    extract::FromRequestParts,
    http::{
        HeaderMap, HeaderName, HeaderValue, Response, StatusCode,
        header::{ACCEPT, LOCATION},
        request::Parts,
    },
};
use leptos::context::use_context;
use server_fn::{error::ServerFnErrorErr, redirect::REDIRECT_HEADER};

#[derive(Debug, Clone, Default)]
pub struct ResponseOptions(Arc<RwLock<ResponseParts>>);

#[derive(Debug, Default)]
struct ResponseParts {
    status: Option<StatusCode>,
    headers: HeaderMap,
}

impl ResponseOptions {
    pub fn set_status(&self, status: StatusCode) {
        with_write(&self.0, |parts| {
            parts.status = Some(status);
        });
    }

    pub fn insert_header(&self, key: HeaderName, value: HeaderValue) {
        with_write(&self.0, |parts| {
            parts.headers.insert(key, value);
        });
    }

    pub fn merge_into<B>(&self, response: &mut Response<B>) {
        with_write(&self.0, |parts| {
            if let Some(status) = parts.status {
                *response.status_mut() = status;
            }
            response
                .headers_mut()
                .extend(std::mem::take(&mut parts.headers));
        });
    }
}

fn with_write<T>(lock: &RwLock<ResponseParts>, body: impl FnOnce(&mut ResponseParts) -> T) -> T {
    match lock.write() {
        Ok(mut guard) => body(&mut guard),
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            body(&mut guard)
        }
    }
}

/// Extracts a request-head value from the `Parts` context provided by the server adapter.
///
/// # Errors
///
/// Returns `ServerFnErrorErr` when the server adapter did not provide request
/// `Parts` or when the requested axum extractor rejects those parts.
pub async fn extract<T>() -> Result<T, ServerFnErrorErr>
where
    T: Sized + FromRequestParts<()>,
    T::Rejection: std::fmt::Debug,
{
    let mut parts = use_context::<Parts>().ok_or_else(|| {
        ServerFnErrorErr::ServerError(
            "should have had Parts provided by Jaunder's server-fn adapter".to_string(),
        )
    })?;
    T::from_request_parts(&mut parts, &())
        .await
        .map_err(|error| ServerFnErrorErr::ServerError(format!("{error:?}")))
}

pub fn redirect(path: &str) {
    let (Some(parts), Some(response_options)) =
        (use_context::<Parts>(), use_context::<ResponseOptions>())
    else {
        tracing::warn!(
            "could not retrieve either Parts or ResponseOptions while trying to redirect"
        );
        return;
    };

    let Ok(location) = HeaderValue::from_str(path) else {
        tracing::warn!(path, "could not create redirect Location header");
        return;
    };
    response_options.insert_header(LOCATION, location);

    let accepts_html = parts
        .headers
        .get(ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/html"));
    if accepts_html {
        response_options.set_status(StatusCode::FOUND);
    } else {
        response_options.insert_header(
            HeaderName::from_static(REDIRECT_HEADER),
            HeaderValue::from_static(""),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use leptos::{context::provide_context, reactive::owner::Owner};

    #[test]
    fn response_options_merge_status_and_headers() {
        let options = ResponseOptions::default();
        options.set_status(StatusCode::NOT_FOUND);
        options.insert_header(
            HeaderName::from_static("x-test"),
            HeaderValue::from_static("set"),
        );

        let mut response = Response::new(Body::empty());
        options.merge_into(&mut response);

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers().get("x-test"),
            Some(&HeaderValue::from_static("set"))
        );
    }

    #[test]
    fn redirect_for_enhanced_request_sets_client_redirect_header_without_302() {
        Owner::new().with(|| {
            let parts = request_parts(
                Request::builder()
                    .uri("/api/auth/login")
                    .body(Body::empty()),
            );
            let options = ResponseOptions::default();
            provide_context(parts);
            provide_context(options.clone());

            redirect("/");

            let mut response = Response::new(Body::empty());
            options.merge_into(&mut response);
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(LOCATION),
                Some(&HeaderValue::from_static("/"))
            );
            assert_eq!(
                response.headers().get(REDIRECT_HEADER),
                Some(&HeaderValue::from_static(""))
            );
        });
    }

    #[test]
    fn redirect_for_html_request_sets_302() {
        Owner::new().with(|| {
            let parts = request_parts(
                Request::builder()
                    .uri("/api/auth/login")
                    .header(ACCEPT, "text/html")
                    .body(Body::empty()),
            );
            let options = ResponseOptions::default();
            provide_context(parts);
            provide_context(options.clone());

            redirect("/");

            let mut response = Response::new(Body::empty());
            options.merge_into(&mut response);
            assert_eq!(response.status(), StatusCode::FOUND);
            assert_eq!(
                response.headers().get(LOCATION),
                Some(&HeaderValue::from_static("/"))
            );
            assert_eq!(response.headers().get(REDIRECT_HEADER), None);
        });
    }

    fn request_parts(result: Result<Request<Body>, http::Error>) -> Parts {
        let Ok(request) = result else {
            panic!("test request should build");
        };
        let (parts, _) = request.into_parts();
        parts
    }
}
