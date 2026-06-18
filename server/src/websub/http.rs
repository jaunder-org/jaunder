use async_trait::async_trait;
use std::time::Duration;

use super::{WebSubClient, WebSubError};

pub struct HttpWebSubClient {
    client: reqwest::Client,
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
            client: reqwest::Client::new(),
            timeout,
        }
    }
}

impl Default for HttpWebSubClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WebSubClient for HttpWebSubClient {
    async fn send_publish(&self, hub_url: &str, feed_url: &str) -> Result<(), WebSubError> {
        let form = [("hub.mode", "publish"), ("hub.url", feed_url)];
        let res = self
            .client
            .post(hub_url)
            .timeout(self.timeout)
            .form(&form)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    WebSubError::Timeout(self.timeout)
                } else {
                    WebSubError::Http(e.to_string())
                }
            })?;
        let status = res.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(WebSubError::HubRefused {
                status: status.as_u16(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Form, http::StatusCode, routing::post, Router};
    use serde::Deserialize;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Debug, Deserialize, Clone)]
    struct HubForm {
        #[serde(rename = "hub.mode")]
        mode: String,
        #[serde(rename = "hub.url")]
        url: String,
    }

    async fn spawn_hub(received: Arc<Mutex<Vec<HubForm>>>, status: StatusCode) -> SocketAddr {
        let app = Router::new().route(
            "/",
            post({
                let received = received.clone();
                move |Form(form): Form<HubForm>| {
                    let received = received.clone();
                    async move {
                        received.lock().await.push(form);
                        status
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    #[test]
    fn default_impl_constructs_client() {
        let _ = HttpWebSubClient::default();
    }

    #[tokio::test]
    async fn returns_http_error_for_invalid_url_scheme() {
        let c = HttpWebSubClient::new();
        let err = c
            .send_publish("not-a-valid-url", "https://example.com/feed.rss")
            .await
            .expect_err("invalid URL should fail");
        assert!(matches!(err, WebSubError::Http(_)));
    }

    #[tokio::test]
    async fn posts_form_body_to_hub_on_success() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let addr = spawn_hub(received.clone(), StatusCode::ACCEPTED).await;
        let c = HttpWebSubClient::new();
        c.send_publish(&format!("http://{addr}/"), "https://example.com/feed.rss")
            .await
            .unwrap();
        let got = received.lock().await.clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].mode, "publish");
        assert_eq!(got[0].url, "https://example.com/feed.rss");
    }

    #[tokio::test]
    async fn returns_hub_refused_on_4xx() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let addr = spawn_hub(received.clone(), StatusCode::BAD_REQUEST).await;
        let c = HttpWebSubClient::new();
        let err = c
            .send_publish(&format!("http://{addr}/"), "https://example.com/feed.rss")
            .await
            .unwrap_err();
        assert!(matches!(err, WebSubError::HubRefused { status: 400 }));
    }

    /// A hub that accepts the connection but never replies within a test's
    /// lifetime, so a short client timeout deterministically fires.
    async fn spawn_hanging_hub() -> SocketAddr {
        let app = Router::new().route(
            "/",
            post(|| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                StatusCode::ACCEPTED
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn returns_http_error_on_connection_refused() {
        // Bind a port, then drop the listener so nothing is listening: the
        // connect is refused immediately — a deterministic non-timeout error
        // (covers the `else` arm of the error mapping).
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);

        let c = HttpWebSubClient::new();
        let err = c
            .send_publish(&format!("http://{addr}/"), "https://example.com/feed.rss")
            .await
            .unwrap_err();
        assert!(matches!(err, WebSubError::Http(_)));
    }

    #[tokio::test]
    async fn returns_timeout_when_hub_does_not_respond() {
        // A hub that never replies + a tiny client timeout deterministically
        // exercises the `is_timeout` branch.
        let addr = spawn_hanging_hub().await;
        let c = HttpWebSubClient::with_timeout(Duration::from_millis(100));
        let err = c
            .send_publish(&format!("http://{addr}/"), "https://example.com/feed.rss")
            .await
            .unwrap_err();
        assert!(matches!(err, WebSubError::Timeout(_)));
    }
}
