//! Axum handler for Jaunder's Leptos server functions.
//!
//! `web` owns the `#[server]` functions and their in-body context helpers;
//! `server` owns the router. This adapter is the composition seam between them:
//! it dispatches through `server_fn::axum`, provides the Leptos owner/context
//! values required by ADR-0016, and applies response mutations after the server
//! function body has run.

use axum::{
    body::Body,
    http::{
        HeaderValue, Request, Response, StatusCode,
        header::{ACCEPT, LOCATION, REFERER},
        request::Parts,
    },
};
use leptos::{
    context::provide_context,
    reactive::{computed::ScopedFuture, owner::Owner},
};
use web::server_fn_adapter::ResponseOptions;

pub async fn handle_server_fns_with_context(
    additional_context: impl Fn() + 'static + Clone + Send,
    req: Request<Body>,
) -> Response<Body> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let (req, parts) = request_and_parts(req);

    if let Some(mut service) = server_fn::axum::get_server_fn_service(&path, method) {
        let owner = Owner::new();
        owner
            .with(|| {
                ScopedFuture::new(async move {
                    provide_context(parts);
                    let response_options = ResponseOptions::default();
                    provide_context(response_options.clone());
                    additional_context();

                    let accepts_html = req
                        .headers()
                        .get(ACCEPT)
                        .and_then(|value| value.to_str().ok())
                        .is_some_and(|value| value.contains("text/html"));
                    let referer = req.headers().get(REFERER).cloned();

                    let mut response = service.run(req).await;
                    apply_plain_form_fallback(&mut response, accepts_html, referer);
                    response_options.merge_into(&mut response);
                    response
                })
            })
            .await
    } else {
        missing_server_fn_response(&path)
    }
}

fn request_and_parts(req: Request<Body>) -> (Request<Body>, Parts) {
    let (parts, body) = req.into_parts();
    let context_parts = parts.clone();
    (Request::from_parts(parts, body), context_parts)
}

fn apply_plain_form_fallback(
    response: &mut Response<Body>,
    accepts_html: bool,
    referer: Option<HeaderValue>,
) {
    if !accepts_html || response.headers().contains_key(LOCATION) {
        return;
    }
    if let Some(referer) = referer {
        *response.status_mut() = StatusCode::FOUND;
        response.headers_mut().insert(LOCATION, referer);
    }
}

fn missing_server_fn_response(path: &str) -> Response<Body> {
    let body = Body::from(format!(
        "Could not find a server function at the route {path}. \n\nIt's likely that either\n 1. The API prefix you specify in the `#[server]` macro doesn't match the prefix at which your server function handler is mounted, or \n2. You are on a platform that doesn't support automatic server function registration and you need to call ServerFn::register_explicit() on the server function type, somewhere in your `main` function.",
    ));
    match Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(body)
    {
        Ok(response) => response,
        Err(error) => Response::new(Body::from(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_form_fallback_redirects_to_referer_without_location() {
        let mut response = Response::new(Body::empty());
        apply_plain_form_fallback(
            &mut response,
            true,
            Some(HeaderValue::from_static("/from-form")),
        );

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response.headers().get(LOCATION),
            Some(&HeaderValue::from_static("/from-form"))
        );
    }

    #[test]
    fn plain_form_fallback_preserves_explicit_location() {
        let mut response = Response::new(Body::empty());
        response
            .headers_mut()
            .insert(LOCATION, HeaderValue::from_static("/explicit"));
        apply_plain_form_fallback(
            &mut response,
            true,
            Some(HeaderValue::from_static("/from-form")),
        );

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(LOCATION),
            Some(&HeaderValue::from_static("/explicit"))
        );
    }
}
