use async_trait::async_trait;
use reqwest::{StatusCode, redirect::Policy};
use std::{
    collections::HashSet,
    io,
    sync::LazyLock,
    time::{Duration, SystemTime},
};

use super::{RetryableWebSubError, TerminalWebSubError, WebSubClient, WebSubError};
use common::tagged_url::{FeedUrl, HubUrl};
use url::Url;

pub struct HttpWebSubClient {
    client: LazyLock<Result<reqwest::Client, String>>,
    timeout: Duration,
}

impl HttpWebSubClient {
    #[must_use]
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(5))
    }

    /// Builds a client with a custom per-request timeout. Tests use a short
    /// timeout against a non-responding hub to exercise the timeout branch
    /// deterministically.
    #[must_use]
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            client: LazyLock::new(Self::build_client),
            timeout,
        }
    }

    fn build_client() -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .map_err(|error| error.to_string())
    }

    fn client(&self) -> Result<&reqwest::Client, WebSubError> {
        self.client
            .as_ref()
            .map_err(|error| WebSubError::Retryable {
                reason: RetryableWebSubError::Transport(Box::new(io::Error::other(error.clone()))),
                retry_after: None,
            })
    }
}

impl Default for HttpWebSubClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WebSubClient for HttpWebSubClient {
    async fn send_publish(&self, hub_url: &HubUrl, feed_url: &FeedUrl) -> Result<(), WebSubError> {
        // reqwest is an external type, so reading both inner values out here is
        // the ADR-0063 §5 carve-out. `IntoUrl` is sealed and has no impl for our
        // newtype, so `post` needs the `&str` explicitly.
        let form = [("hub.mode", "publish"), ("hub.url", feed_url.as_ref())];
        // cov:ignore-start -- HubUrl validates its serialized URL at construction,
        // so this defensive parse failure is unreachable through the typed boundary.
        let mut target = Url::parse(hub_url.as_ref()).map_err(|source| WebSubError::Retryable {
            reason: RetryableWebSubError::Transport(Box::new(source)),
            retry_after: None,
        })?;
        // cov:ignore-stop
        let mut visited = HashSet::from([target.clone()]);
        let mut redirects = 0;

        loop {
            let response = self
                .client()?
                .post(target.clone())
                .timeout(self.timeout)
                .form(&form)
                .send()
                .await
                .map_err(|source| WebSubError::Retryable {
                    reason: RetryableWebSubError::Transport(Box::new(source)),
                    retry_after: None,
                })?;
            let status = response.status();

            if status.is_success() {
                return Ok(());
            }

            if status == StatusCode::TEMPORARY_REDIRECT || status == StatusCode::PERMANENT_REDIRECT
            {
                if redirects == 3 {
                    return Err(WebSubError::Terminal {
                        reason: TerminalWebSubError::TooManyRedirects {
                            status: status.as_u16(),
                        },
                    });
                }
                let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                    return Err(WebSubError::Terminal {
                        reason: TerminalWebSubError::MissingLocation {
                            status: status.as_u16(),
                        },
                    });
                };
                let Ok(location) = location.to_str() else {
                    return Err(WebSubError::Terminal {
                        reason: TerminalWebSubError::InvalidLocation {
                            status: status.as_u16(),
                        },
                    });
                };
                let Ok(mut next) = target.join(location) else {
                    return Err(WebSubError::Terminal {
                        reason: TerminalWebSubError::InvalidLocation {
                            status: status.as_u16(),
                        },
                    });
                };
                if !matches!(next.scheme(), "http" | "https") {
                    return Err(WebSubError::Terminal {
                        reason: TerminalWebSubError::UnsupportedLocationScheme {
                            status: status.as_u16(),
                        },
                    });
                }
                next.set_fragment(None);
                if !visited.insert(next.clone()) {
                    return Err(WebSubError::Terminal {
                        reason: TerminalWebSubError::RedirectLoop {
                            status: status.as_u16(),
                        },
                    });
                }
                target = next;
                redirects += 1;
                continue;
            }

            if status == StatusCode::REQUEST_TIMEOUT
                || status == StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
            {
                return Err(WebSubError::Retryable {
                    reason: RetryableWebSubError::Http {
                        status: status.as_u16(),
                    },
                    retry_after: retry_after(response.headers()),
                });
            }

            return Err(WebSubError::Terminal {
                reason: TerminalWebSubError::Http {
                    status: status.as_u16(),
                },
            });
        }
    }
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    const MAX_RETRY_AFTER: Duration = Duration::from_hours(24);

    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let delay = value
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            httpdate::parse_http_date(value)
                .ok()?
                .duration_since(SystemTime::now())
                .ok()
                .filter(|delay| !delay.is_zero())
        })?;
    Some(delay.min(MAX_RETRY_AFTER))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        extract::{Form, State},
        http::{HeaderValue, Uri, header},
        response::Response,
        routing::post,
    };
    use common::test_support::parse_url;
    use serde::Deserialize;
    use std::{
        collections::HashMap,
        net::SocketAddr,
        sync::Arc,
        time::{Duration, SystemTime},
    };
    use tokio::sync::Mutex;

    #[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
    struct HubForm {
        #[serde(rename = "hub.mode")]
        mode: String,
        #[serde(rename = "hub.url")]
        url: String,
    }

    #[derive(Debug, Clone)]
    struct HubResponse {
        status: StatusCode,
        location: Option<HeaderValue>,
        retry_after: Option<HeaderValue>,
    }

    impl HubResponse {
        fn status(status: StatusCode) -> Self {
            Self {
                status,
                location: None,
                retry_after: None,
            }
        }
    }

    #[derive(Clone)]
    struct HubState {
        received: Arc<Mutex<Vec<(String, HubForm)>>>,
        responses: HashMap<String, HubResponse>,
    }

    async fn respond(
        State(state): State<HubState>,
        uri: Uri,
        Form(form): Form<HubForm>,
    ) -> Response {
        let path = uri.path().to_owned();
        state.received.lock().await.push((path.clone(), form));
        let response = state
            .responses
            .get(&path)
            .unwrap_or_else(|| panic!("unexpected hub request to {path}"));
        let mut builder = Response::builder().status(response.status);
        if let Some(location) = &response.location {
            builder = builder.header(header::LOCATION, location.clone());
        }
        if let Some(retry_after) = &response.retry_after {
            builder = builder.header(header::RETRY_AFTER, retry_after.clone());
        }
        builder.body(Body::empty()).expect("response is valid")
    }

    async fn spawn_hub(
        responses: impl IntoIterator<Item = (impl Into<String>, HubResponse)>,
    ) -> (SocketAddr, Arc<Mutex<Vec<(String, HubForm)>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let state = HubState {
            received: received.clone(),
            responses: responses
                .into_iter()
                .map(|(path, response)| (path.into(), response))
                .collect(),
        };
        let app = Router::new().fallback(post(respond)).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener binds");
        let addr = listener.local_addr().expect("test listener has address");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                // The test-owned server is aborted at test teardown, so its terminal
                // result is not observable by the client scenario.
                .expect("test hub serves requests"); // cov:ignore
        }); // cov:ignore
        (addr, received)
    }

    fn hub_at(addr: SocketAddr, path: &str) -> HubUrl {
        parse_url(&format!("http://{addr}{path}"))
    }

    fn feed_url() -> FeedUrl {
        parse_url("https://example.com/feed.rss")
    }

    fn retryable(error: WebSubError) -> (Option<Duration>, RetryableWebSubError) {
        match error {
            WebSubError::Retryable {
                retry_after,
                reason,
            } => (retry_after, reason),
            // cov:ignore-start — assertion helper's impossible variant in retryable-only tests
            WebSubError::Terminal { reason } => {
                panic!("expected retryable failure, got terminal {reason}")
            } // cov:ignore-stop
        }
    }

    fn failed_client_build() -> Result<reqwest::Client, String> {
        Err("test client initialization failure".to_owned())
    }

    #[test]
    fn client_initialization_failure_is_retryable_transport() {
        let client = HttpWebSubClient {
            client: LazyLock::new(failed_client_build),
            timeout: Duration::from_secs(1),
        };

        let error = client
            .client()
            .expect_err("failed client build is retryable");
        let (delay, reason) = retryable(error);
        assert_eq!(delay, None);
        assert!(matches!(reason, RetryableWebSubError::Transport(_)));
    }

    #[test]
    fn default_impl_constructs_client() {
        let _ = HttpWebSubClient::default();
    }

    #[tokio::test]
    async fn succeeds_for_every_2xx_status() {
        let responses = (200..300)
            .map(|status| {
                (
                    format!("/{status}"),
                    HubResponse::status(StatusCode::from_u16(status).expect("2xx status")),
                )
            })
            .collect::<Vec<_>>();
        let (addr, _) = spawn_hub(responses).await;
        let client = HttpWebSubClient::new();

        for status in 200..300 {
            client
                .send_publish(&hub_at(addr, &format!("/{status}")), &feed_url())
                .await
                .unwrap_or_else(|error| panic!("{status} must succeed: {error}"));
        }
    }

    #[tokio::test]
    async fn classifies_retryable_http_statuses_without_remote_delay() {
        let statuses = std::iter::once(408)
            .chain(std::iter::once(429))
            .chain(500..600)
            .collect::<Vec<_>>();
        let responses = statuses.iter().map(|status| {
            (
                format!("/{status}"),
                HubResponse::status(StatusCode::from_u16(*status).expect("retryable status")),
            )
        });
        let (addr, _) = spawn_hub(responses).await;
        let client = HttpWebSubClient::new();

        for status in statuses {
            let error = client
                .send_publish(&hub_at(addr, &format!("/{status}")), &feed_url())
                .await
                .expect_err("retryable HTTP response");
            let (delay, reason) = retryable(error);
            assert_eq!(delay, None);
            assert!(
                matches!(reason, RetryableWebSubError::Http { status: actual } if actual == status)
            );
        }
    }

    #[tokio::test]
    async fn classifies_every_other_3xx_and_4xx_status_as_terminal() {
        let statuses = (300..500)
            .filter(|status| !matches!(status, 307 | 308 | 408 | 429))
            .collect::<Vec<_>>();
        let responses = statuses.iter().map(|status| {
            (
                format!("/{status}"),
                HubResponse::status(StatusCode::from_u16(*status).expect("terminal status")),
            )
        });
        let (addr, _) = spawn_hub(responses).await;
        let client = HttpWebSubClient::new();

        for status in statuses {
            let error = client
                .send_publish(&hub_at(addr, &format!("/{status}")), &feed_url())
                .await
                .expect_err("terminal HTTP response");
            assert!(matches!(
                error,
                WebSubError::Terminal {
                    reason: TerminalWebSubError::Http { status: actual }
                } if actual == status
            ));
        }
    }

    #[tokio::test]
    async fn follows_three_307_and_308_redirects_preserving_post_form() {
        let responses = [
            (
                "/start",
                HubResponse {
                    status: StatusCode::TEMPORARY_REDIRECT,
                    location: Some(HeaderValue::from_static("/first")),
                    retry_after: None,
                },
            ),
            (
                "/first",
                HubResponse {
                    status: StatusCode::PERMANENT_REDIRECT,
                    location: Some(HeaderValue::from_static("/second")),
                    retry_after: None,
                },
            ),
            (
                "/second",
                HubResponse {
                    status: StatusCode::TEMPORARY_REDIRECT,
                    location: Some(HeaderValue::from_static("/complete")),
                    retry_after: None,
                },
            ),
            ("/complete", HubResponse::status(StatusCode::NO_CONTENT)),
        ];
        let (addr, received) = spawn_hub(responses).await;
        HttpWebSubClient::new()
            .send_publish(&hub_at(addr, "/start"), &feed_url())
            .await
            .expect("three preserving redirects succeed");

        let received = received.lock().await.clone();
        assert_eq!(
            received.iter().map(|(path, _)| path).collect::<Vec<_>>(),
            vec!["/start", "/first", "/second", "/complete"]
        );
        assert!(received.iter().all(|(_, form)| {
            form == &HubForm {
                mode: "publish".into(),
                url: "https://example.com/feed.rss".into(),
            }
        }));
    }

    async fn assert_rejected_redirect(start: &str, responses: Vec<(&'static str, HubResponse)>) {
        let (addr, received) = spawn_hub(responses).await;
        let error = HttpWebSubClient::new()
            .send_publish(&hub_at(addr, start), &feed_url())
            .await
            .expect_err("disallowed redirect is terminal");
        let expected_diagnostic = match start {
            "/missing" => "without Location",
            "/invalid" | "/bad-url" => "invalid Location",
            "/non-http" => "non-HTTP(S) Location",
            "/loop" => "redirect loop",
            "/fourth" => "redirect limit",
            _ => unreachable!("known redirect case"),
        };
        assert!(error.to_string().contains(expected_diagnostic));
        assert!(matches!(
            (start, error),
            (
                "/missing",
                WebSubError::Terminal {
                    reason: TerminalWebSubError::MissingLocation { status: 307 },
                },
            ) | (
                "/invalid",
                WebSubError::Terminal {
                    reason: TerminalWebSubError::InvalidLocation { status: 308 },
                },
            ) | (
                "/bad-url",
                WebSubError::Terminal {
                    reason: TerminalWebSubError::InvalidLocation { status: 307 },
                },
            ) | (
                "/non-http",
                WebSubError::Terminal {
                    reason: TerminalWebSubError::UnsupportedLocationScheme { status: 307 },
                },
            ) | (
                "/loop",
                WebSubError::Terminal {
                    reason: TerminalWebSubError::RedirectLoop { status: 308 },
                },
            ) | (
                "/fourth",
                WebSubError::Terminal {
                    reason: TerminalWebSubError::TooManyRedirects { status: 308 },
                },
            )
        ));
        assert_eq!(
            received.lock().await.len(),
            if start == "/fourth" { 4 } else { 1 },
            "the client must not follow a rejected redirect"
        );
    }

    #[tokio::test]
    async fn rejects_missing_invalid_and_non_http_redirect_locations() {
        for (start, response) in [
            (
                "/missing",
                HubResponse::status(StatusCode::TEMPORARY_REDIRECT),
            ),
            (
                "/invalid",
                HubResponse {
                    status: StatusCode::PERMANENT_REDIRECT,
                    location: Some(HeaderValue::from_bytes(b"\xff").expect("opaque header")),
                    retry_after: None,
                },
            ),
            (
                "/bad-url",
                HubResponse {
                    status: StatusCode::TEMPORARY_REDIRECT,
                    location: Some(HeaderValue::from_static("http://[::1")),
                    retry_after: None,
                },
            ),
            (
                "/non-http",
                HubResponse {
                    status: StatusCode::TEMPORARY_REDIRECT,
                    location: Some(HeaderValue::from_static("mailto:hub@example.com")),
                    retry_after: None,
                },
            ),
        ] {
            assert_rejected_redirect(start, vec![(start, response)]).await;
        }
    }

    #[tokio::test]
    async fn rejects_redirect_loops_and_fourth_hop() {
        assert_rejected_redirect(
            "/loop",
            vec![(
                "/loop",
                HubResponse {
                    status: StatusCode::PERMANENT_REDIRECT,
                    location: Some(HeaderValue::from_static("/loop")),
                    retry_after: None,
                },
            )],
        )
        .await;
        assert_rejected_redirect(
            "/fourth",
            vec![
                (
                    "/fourth",
                    HubResponse {
                        status: StatusCode::TEMPORARY_REDIRECT,
                        location: Some(HeaderValue::from_static("/one")),
                        retry_after: None,
                    },
                ),
                (
                    "/one",
                    HubResponse {
                        status: StatusCode::PERMANENT_REDIRECT,
                        location: Some(HeaderValue::from_static("/two")),
                        retry_after: None,
                    },
                ),
                (
                    "/two",
                    HubResponse {
                        status: StatusCode::TEMPORARY_REDIRECT,
                        location: Some(HeaderValue::from_static("/three")),
                        retry_after: None,
                    },
                ),
                (
                    "/three",
                    HubResponse {
                        status: StatusCode::PERMANENT_REDIRECT,
                        location: Some(HeaderValue::from_static("/ignored")),
                        retry_after: None,
                    },
                ),
            ],
        )
        .await;
    }

    #[tokio::test]
    async fn parses_and_caps_retry_after_delta_seconds_and_http_dates() {
        let future = httpdate::fmt_http_date(SystemTime::now() + Duration::from_hours(48));
        let responses = [
            (
                "/delta",
                HubResponse {
                    status: StatusCode::TOO_MANY_REQUESTS,
                    location: None,
                    retry_after: Some(HeaderValue::from_static("120")),
                },
            ),
            (
                "/delta-cap",
                HubResponse {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    location: None,
                    retry_after: Some(HeaderValue::from_static("172800")),
                },
            ),
            (
                "/date",
                HubResponse {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    location: None,
                    retry_after: Some(HeaderValue::from_str(&future).expect("HTTP date header")),
                },
            ),
        ];
        let (addr, _) = spawn_hub(responses).await;
        let client = HttpWebSubClient::new();

        let (delay, _) = retryable(
            client
                .send_publish(&hub_at(addr, "/delta"), &feed_url())
                .await
                .expect_err("retryable delta response"),
        );
        assert_eq!(delay, Some(Duration::from_mins(2)));
        let (delay, _) = retryable(
            client
                .send_publish(&hub_at(addr, "/delta-cap"), &feed_url())
                .await
                .expect_err("retryable capped delta response"),
        );
        assert_eq!(delay, Some(Duration::from_hours(24)));
        let (delay, _) = retryable(
            client
                .send_publish(&hub_at(addr, "/date"), &feed_url())
                .await
                .expect_err("retryable date response"),
        );
        assert_eq!(delay, Some(Duration::from_hours(24)));
    }

    #[tokio::test]
    async fn ignores_missing_invalid_and_past_retry_after() {
        let past = httpdate::fmt_http_date(SystemTime::now() - Duration::from_mins(1));
        let responses = [
            (
                "/missing",
                HubResponse::status(StatusCode::TOO_MANY_REQUESTS),
            ),
            (
                "/invalid",
                HubResponse {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    location: None,
                    retry_after: Some(HeaderValue::from_static("tomorrow")),
                },
            ),
            (
                "/past",
                HubResponse {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    location: None,
                    retry_after: Some(HeaderValue::from_str(&past).expect("HTTP date header")),
                },
            ),
        ];
        let (addr, _) = spawn_hub(responses).await;
        let client = HttpWebSubClient::new();

        for path in ["/missing", "/invalid", "/past"] {
            let (delay, _) = retryable(
                client
                    .send_publish(&hub_at(addr, path), &feed_url())
                    .await
                    .expect_err("retryable response"),
            );
            assert_eq!(delay, None);
        }
    }

    async fn spawn_hanging_hub() -> SocketAddr {
        let app = Router::new().fallback(post(|| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            StatusCode::ACCEPTED // cov:ignore timeout cancels this test-only handler first
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener binds");
        let addr = listener.local_addr().expect("test listener has address");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                // The hanging server is intentionally cancelled after the client times out.
                .expect("test hub serves requests"); // cov:ignore
        }); // cov:ignore
        addr
    }

    #[tokio::test]
    async fn preserves_typed_transport_and_timeout_sources() {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener binds");
        let refused = probe.local_addr().expect("test listener has address");
        drop(probe);

        let (_, reason) = retryable(
            HttpWebSubClient::new()
                .send_publish(&hub_at(refused, "/"), &feed_url())
                .await
                .expect_err("refused connection is retryable"),
        );
        let RetryableWebSubError::Transport(source) = reason else {
            panic!("transport failure has typed transport reason"); // cov:ignore
        };
        let source = source
            .downcast_ref::<reqwest::Error>()
            .expect("typed reqwest source for refused connection");
        assert!(!source.is_timeout());

        let hanging = spawn_hanging_hub().await;
        let (_, reason) = retryable(
            HttpWebSubClient::with_timeout(Duration::from_millis(100))
                .send_publish(&hub_at(hanging, "/"), &feed_url())
                .await
                .expect_err("timeout is retryable"),
        );
        let RetryableWebSubError::Transport(source) = reason else {
            panic!("timeout has typed transport reason"); // cov:ignore
        };
        let source = source
            .downcast_ref::<reqwest::Error>()
            .expect("typed reqwest source for timeout");
        assert!(source.is_timeout());
    }
}
