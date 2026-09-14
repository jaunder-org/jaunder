//! Loopback preview server and private Chromium `DevTools` adapter for thumbnails.

use std::{collections::HashSet, path::Path, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, anyhow, bail};
use axum::{
    Router,
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};
use axum_embed::ServeEmbed;
use futures_util::{SinkExt, StreamExt};
use host::error;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::oneshot,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const VIEWPORT_WIDTH: u32 = 1200;
const VIEWPORT_HEIGHT: u32 = 800;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
const READY_TIMEOUT: Duration = Duration::from_secs(20);

/// Produces a PNG only after repository admission, so a rejected source cannot spawn a process.
/// # Errors
///
/// Returns an explicit browser, protocol, readiness, serving, or publication failure.
pub async fn cmd_theme_thumbnail(
    repository: &Path,
    browser: &Path,
    output: &Path,
) -> anyhow::Result<()> {
    let accepted =
        host::theme_repository::accept_theme_repository(repository).with_context(|| {
            format!(
                "theme repository validation failed: {}",
                repository.display()
            )
        })?;
    let logo_url = thumbnail_asset_url(accepted.revision().default_logo_path())?;
    let header_path = storage::select_packaged_header_default(
        accepted.revision().default_header_paths(),
        accepted.publication_revision(),
        &common::theme::PublicThemeRoute::site(),
    );
    let header_url = thumbnail_asset_url(header_path)?;
    let fixture =
        web::themes::thumbnail_document(logo_url, header_url).context("build thumbnail fixture")?;
    let accepted = Arc::new(accepted);
    let profile = tempfile::tempdir().context("create temporary Chromium profile")?;
    let preview = match PreviewServer::start(fixture, Arc::clone(&accepted)).await {
        Ok(preview) => preview,
        // cov:ignore-start: A loopback bind or preview-task startup failure needs an OS or Tokio fault that host tests cannot deterministically inject.
        Err(error) => {
            return merge_primary_and_cleanup(
                Err(error),
                profile.close().context("remove temporary Chromium profile"),
                "clean up Chromium profile after preview startup failure",
            );
        } // cov:ignore-stop
    };
    let requests = PreviewRequestPolicy::new(preview.origin(), accepted.revision());
    let capture = capture(browser, profile.path(), &requests).await;
    let capture =
        merge_primary_and_cleanup(capture, preview.stop().await, "stop thumbnail preview");
    let png = merge_primary_and_cleanup(
        capture,
        profile.close().context("remove temporary Chromium profile"),
        "clean up Chromium profile",
    )?;
    super::theme_artifact::publish_replace(output, &png, "thumbnail")?;
    println!("Theme thumbnail created: {}", output.display());
    Ok(())
}

fn thumbnail_asset_url(
    path: Option<&str>,
) -> anyhow::Result<Option<common::root_relative_url::RootRelativeUrl>> {
    path.map(|path| {
        format!(
            "/theme-assets/{}",
            host::theme_package::percent_encode_asset_path(path)
        )
        .parse()
        .context("build thumbnail default asset URL")
    })
    .transpose()
}
struct PreviewServer {
    origin: String,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<anyhow::Result<()>>,
}
impl PreviewServer {
    async fn start(
        document: String,
        accepted: Arc<host::theme_repository::AcceptedThemeRepository>,
    ) -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind loopback thumbnail preview")?;
        let address = listener
            .local_addr()
            .context("read loopback thumbnail preview address")?;
        let (shutdown, receiver) = oneshot::channel();
        let app = Router::new()
            .nest_service("/style", ServeEmbed::<crate::assets::StaticAssets>::new())
            .route("/", get(document_handler))
            .route("/theme.css", get(css_handler))
            .route("/theme-assets/{*path}", get(asset_handler))
            .with_state(Arc::new(PreviewState { document, accepted }));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    // Dropping the owner is an intentional shutdown signal.
                    let _ = receiver.await;
                })
                .await
                .context("serve loopback thumbnail preview")
        });
        Ok(Self {
            origin: format!("http://{address}"),
            shutdown: Some(shutdown),
            task,
        })
    }
    fn origin(&self) -> &str {
        &self.origin
    }
    async fn stop(mut self) -> anyhow::Result<()> {
        let shutdown = self.shutdown.take().map_or(Ok(()), |sender| {
            sender
                .send(())
                .map_err(|()| anyhow!("signal loopback thumbnail preview shutdown"))
        });
        let serve = self
            .task
            .await
            .context("join loopback thumbnail preview task")
            .and_then(|result| result);
        merge_primary_and_cleanup(serve, shutdown, "signal thumbnail preview shutdown")
    }
}
struct PreviewState {
    document: String,
    accepted: Arc<host::theme_repository::AcceptedThemeRepository>,
}
async fn document_handler(State(state): State<Arc<PreviewState>>) -> Response {
    response(
        StatusCode::OK,
        "text/html; charset=utf-8",
        state.document.as_bytes(),
    )
}
async fn css_handler(State(state): State<Arc<PreviewState>>) -> Response {
    response(
        StatusCode::OK,
        "text/css; charset=utf-8",
        state.accepted.revision().css().bytes(),
    )
}
async fn asset_handler(
    AxumPath(path): AxumPath<String>,
    State(state): State<Arc<PreviewState>>,
) -> Response {
    match state.accepted.revision().asset(&path) {
        Some((mime, bytes, _)) => response(StatusCode::OK, mime, bytes),
        None => response(StatusCode::NOT_FOUND, "text/plain", b"not found"),
    }
}
fn response(status: StatusCode, mime: &str, bytes: &[u8]) -> Response {
    let mut response = Response::new(Body::from(bytes.to_vec()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    response
}

async fn capture(
    browser: &Path,
    profile: &Path,
    requests: &PreviewRequestPolicy,
) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new(browser);
    // Disable Chromium's background network services before the process starts and
    // refuse non-local hostname resolution. CDP interception below separately rejects
    // every page request outside the loopback preview origin.
    command
        .args([
            "--headless=new",
            "--disable-gpu",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-dev-shm-usage",
            "--disable-domain-reliability",
            "--disable-features=AutofillServerCommunication,MediaRouter,OptimizationHints",
            "--disable-font-subpixel-positioning",
            "--disable-lcd-text",
            "--disable-quic",
            "--disable-skia-runtime-opts",
            "--disable-sync",
            "--force-color-profile=srgb",
            "--force-device-scale-factor=1",
            "--font-render-hinting=none",
            "--host-resolver-rules=EXCLUDE localhost, EXCLUDE 127.0.0.1, MAP * ~NOTFOUND",
            "--metrics-recording-only",
            "--no-pings",
            "--no-sandbox",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        // Cancellation before explicit cleanup still must not leave Chromium running.
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .with_context(|| format!("start Chromium browser {}", browser.display()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("capture Chromium diagnostics"))?;
    let (endpoint_tx, endpoint_rx) = oneshot::channel();
    let stderr_task = tokio::spawn(read_chromium_diagnostics(
        BufReader::new(stderr),
        endpoint_tx,
    ));
    let endpoint = timeout(READY_TIMEOUT, endpoint_rx)
        .await
        .context("wait for Chromium DevTools endpoint")
        .and_then(|result| result.context("Chromium exited before DevTools became available"));
    let capture = capture_discovered_endpoint(endpoint, requests).await;
    merge_primary_and_cleanup(
        capture,
        stop_chromium(&mut child, stderr_task).await,
        "clean up Chromium capture process",
    )
}

async fn read_chromium_diagnostics<R>(
    reader: R,
    endpoint: oneshot::Sender<String>,
) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
{
    let mut endpoint = Some(endpoint);
    let mut lines = reader.lines();
    while let Some(line) = lines
        .next_line()
        .await
        .context("read Chromium diagnostics")?
    {
        if let Some(url) = line
            .split_whitespace()
            .find(|token| token.starts_with("ws://127.0.0.1:"))
            && let Some(endpoint) = endpoint.take()
        {
            // The receiver legitimately leaves after readiness or capture fails.
            let _ = endpoint.send(url.to_owned());
        }
    }
    Ok(())
}

async fn capture_discovered_endpoint(
    endpoint: anyhow::Result<String>,
    requests: &PreviewRequestPolicy,
) -> anyhow::Result<Vec<u8>> {
    let endpoint = endpoint?;
    let page = page_endpoint(&endpoint).await?;
    cdp_capture(&page, requests).await
}

fn merge_primary_and_cleanup<T>(
    primary: anyhow::Result<T>,
    cleanup: anyhow::Result<()>,
    cleanup_context: &'static str,
) -> anyhow::Result<T> {
    match (primary, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error.context(cleanup_context)),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => {
            let cleanup = cleanup.context(cleanup_context);
            error::report_swallowed(
                error::ErrorKind::Internal,
                error::ErrorClass::Transient,
                cleanup_context,
                error::SwallowedSource::Error(cleanup.root_cause()),
            );
            Err(error)
        }
    }
}

async fn stop_chromium(
    child: &mut Child,
    stderr_task: JoinHandle<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let terminate = child.start_kill().context("request Chromium termination");
    let wait = wait_for_chromium(child).await;
    let cleanup = merge_primary_and_cleanup(terminate, wait, "wait for Chromium termination");
    let stderr = join_chromium_stderr(stderr_task).await;
    merge_primary_and_cleanup(cleanup, stderr, "join Chromium diagnostics reader")
}

async fn wait_for_chromium(child: &mut Child) -> anyhow::Result<()> {
    let status = timeout(CLEANUP_TIMEOUT, child.wait())
        .await
        .context("time out waiting for Chromium termination")?
        .context("wait for Chromium termination")?;
    tracing::debug!(?status, "Chromium terminated after thumbnail capture");
    Ok(())
}

async fn join_chromium_stderr(stderr_task: JoinHandle<anyhow::Result<()>>) -> anyhow::Result<()> {
    join_chromium_stderr_with_timeout(stderr_task, CLEANUP_TIMEOUT).await
}

async fn join_chromium_stderr_with_timeout(
    mut stderr_task: JoinHandle<anyhow::Result<()>>,
    cleanup_timeout: Duration,
) -> anyhow::Result<()> {
    if let Ok(result) = timeout(cleanup_timeout, &mut stderr_task).await {
        result
            .context("join Chromium diagnostics reader")?
            .context("read Chromium diagnostics")
    } else {
        stderr_task.abort();
        match stderr_task.await {
            Err(error) if error.is_cancelled() => {
                bail!("time out joining Chromium diagnostics reader");
            }
            // cov:ignore-start: Abort cancellation is the deterministic timeout outcome; these arms retain a raced task's more specific failure if Tokio completes it concurrently.
            Err(error) => Err(error).context("join Chromium diagnostics reader after timeout"),
            Ok(Err(error)) => Err(error).context("read Chromium diagnostics after timeout"),
            Ok(Ok(())) => bail!("time out joining Chromium diagnostics reader"),
            // cov:ignore-stop
        }
    }
}

async fn page_endpoint(browser_endpoint: &str) -> anyhow::Result<String> {
    let endpoint = url::Url::parse(browser_endpoint).context("parse Chromium DevTools endpoint")?;
    let host = endpoint
        .host_str()
        .ok_or_else(|| anyhow!("Chromium DevTools endpoint has no host"))?;
    let port = endpoint
        .port()
        .ok_or_else(|| anyhow!("Chromium DevTools endpoint has no port"))?;
    if endpoint.scheme() != "ws" || host != "127.0.0.1" {
        bail!("Chromium DevTools browser endpoint is not loopback WebSocket");
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("build isolated Chromium DevTools client")?;
    let response = timeout(
        READY_TIMEOUT,
        client.get(format!("http://{host}:{port}/json/list")).send(),
    )
    .await
    .context("connect to Chromium DevTools targets")?
    .context("list Chromium DevTools targets")?;
    if response.status().is_redirection() {
        bail!("Chromium DevTools target listing attempted an HTTP redirect");
    }
    let response = response
        .error_for_status()
        .context("read Chromium DevTools targets")?;
    let body = timeout(READY_TIMEOUT, response.bytes())
        .await
        .context("read Chromium DevTools targets")?
        .context("read Chromium DevTools targets")?;
    let pages: Vec<Value> =
        serde_json::from_slice(&body).context("decode Chromium DevTools targets")?;
    let page = pages
        .into_iter()
        .find(|page| page.get("type").and_then(Value::as_str) == Some("page"))
        .and_then(|page| {
            page.get("webSocketDebuggerUrl")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(|| anyhow!("Chromium exposed no page DevTools target"))?;
    let page_endpoint = url::Url::parse(&page).context("parse Chromium page DevTools endpoint")?;
    if page_endpoint.scheme() != "ws"
        || page_endpoint.host_str() != Some(host)
        || page_endpoint.port() != Some(port)
    {
        bail!("Chromium page DevTools endpoint left the loopback debugging listener");
    }
    Ok(page)
}

fn preview_origin_allowed(origin: &str, request_url: &str) -> bool {
    url::Url::parse(request_url)
        .ok()
        .is_some_and(|request| request.origin().ascii_serialization() == origin)
}
struct PreviewRequestPolicy {
    origin: String,
    allowed: HashSet<String>,
}

impl PreviewRequestPolicy {
    fn new(origin: &str, revision: &host::theme_package::CompiledThemeRevision) -> Self {
        let mut allowed = HashSet::from([
            format!("{origin}/"),
            format!("{origin}/style/jaunder.css"),
            format!("{origin}/style/jaunder-themes.css"),
            format!("{origin}/theme.css"),
        ]);
        allowed.extend(revision.assets().map(|(path, _, _, _)| {
            format!(
                "{origin}/theme-assets/{}",
                host::theme_package::percent_encode_asset_path(path)
            )
        }));
        Self {
            origin: origin.to_owned(),
            allowed,
        }
    }

    fn allows(&self, request_url: &str) -> bool {
        preview_origin_allowed(&self.origin, request_url) && self.allowed.contains(request_url)
    }

    fn verify_required_requests(&self, accepted: &HashSet<String>) -> anyhow::Result<()> {
        for required in [
            format!("{}/", self.origin),
            format!("{}/style/jaunder.css", self.origin),
            format!("{}/style/jaunder-themes.css", self.origin),
            format!("{}/theme.css", self.origin),
        ] {
            if !accepted.contains(&required) {
                bail!("Chromium did not request required thumbnail resource: {required}");
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct ProtocolState {
    next_id: u64,
    in_flight: HashSet<String>,
    accepted_requests: HashSet<String>,
    network_idle: bool,
}

/// A single Chromium `DevTools` connection, including its request admission policy.
struct CdpSession<'a, W, R> {
    writer: W,
    reader: R,
    state: ProtocolState,
    requests: &'a PreviewRequestPolicy,
}

impl<W, R> CdpSession<'_, W, R>
where
    W: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    R: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    async fn send_command(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.state.next_id += 1;
        let request = self.state.next_id;
        timeout(
            READY_TIMEOUT,
            self.writer.send(Message::Text(
                json!({"id":request,"method":method,"params":params})
                    .to_string()
                    .into(),
            )),
        )
        .await
        .context("write Chromium DevTools command")?
        .context("write Chromium DevTools command")?;
        while let Some(message) = timeout(READY_TIMEOUT, self.reader.next())
            .await
            .context("read Chromium DevTools response")?
        {
            let message = message.context("read Chromium DevTools response")?;
            if matches!(message, Message::Close(_)) {
                bail!("Chromium DevTools connection closed during {method}");
            }
            let value: Value = serde_json::from_str(
                &message
                    .into_text()
                    .context("decode Chromium DevTools text")?,
            )
            .context("decode Chromium DevTools response")?;
            match value.get("method").and_then(Value::as_str) {
                Some("Fetch.requestPaused") => self.handle_request_pause(&value).await?,
                Some("Network.loadingFinished" | "Network.loadingFailed") => {
                    if let Some(network_id) =
                        value.pointer("/params/requestId").and_then(Value::as_str)
                    {
                        self.state.in_flight.remove(network_id);
                    }
                }
                Some("Page.lifecycleEvent")
                    if value.pointer("/params/name").and_then(Value::as_str)
                        == Some("networkIdle") =>
                {
                    self.state.network_idle = true;
                }
                _ if value.get("id").and_then(Value::as_u64) == Some(request) => {
                    if let Some(error) = value.get("error") {
                        bail!("Chromium DevTools {method} failed: {error}");
                    }
                    return Ok(value["result"].clone());
                }
                _ => {}
            }
        }
        bail!("Chromium DevTools connection closed during {method}") // cov:ignore: Tungstenite surfaces a close frame or read error before exhausting the production WebSocket stream.
    }

    async fn handle_request_pause(&mut self, value: &Value) -> anyhow::Result<()> {
        let paused = &value["params"];
        let url = paused
            .pointer("/request/url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let fetch_id = paused
            .get("requestId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let allowed = self.requests.allows(url);
        if allowed {
            self.state.network_idle = false;
            self.state.accepted_requests.insert(url.to_owned());
            if let Some(network_id) = paused.get("networkId").and_then(Value::as_str) {
                self.state.in_flight.insert(network_id.to_owned());
            }
        }
        self.state.next_id += 1;
        let action = if allowed {
            "Fetch.continueRequest"
        } else {
            "Fetch.failRequest"
        };
        let params = if allowed {
            json!({"requestId":fetch_id})
        } else {
            json!({"requestId":fetch_id,"errorReason":"BlockedByClient"})
        };
        timeout(
            READY_TIMEOUT,
            self.writer.send(Message::Text(
                json!({"id":self.state.next_id,"method":action,"params":params})
                    .to_string()
                    .into(),
            )),
        )
        .await
        .context("reply to Chromium Fetch pause")?
        .context("reply to Chromium Fetch pause")?;
        if !allowed {
            bail!("Chromium attempted disallowed thumbnail request: {url}");
        }
        Ok(())
    }
}

async fn cdp_capture(endpoint: &str, requests: &PreviewRequestPolicy) -> anyhow::Result<Vec<u8>> {
    let (socket, _) = timeout(READY_TIMEOUT, connect_async(endpoint))
        .await
        .context("connect to Chromium DevTools")?
        .context("connect to Chromium DevTools")?;
    let (writer, reader) = socket.split();
    let mut session = CdpSession {
        writer,
        reader,
        state: ProtocolState::default(),
        requests,
    };
    for (method, params) in [
        ("Page.enable", json!({})),
        ("Network.enable", json!({})),
        ("Page.setLifecycleEventsEnabled", json!({"enabled":true})),
        (
            "Fetch.enable",
            json!({"patterns":[{"urlPattern":"*","requestStage":"Request"}]}),
        ),
        (
            "Emulation.setDeviceMetricsOverride",
            json!({"width": VIEWPORT_WIDTH, "height": VIEWPORT_HEIGHT, "deviceScaleFactor": 1, "mobile": false}),
        ),
        ("Emulation.setEmulatedMedia", json!({"media":"screen"})),
    ] {
        session.send_command(method, params).await?;
    }
    let preview_url = format!("{}/", requests.origin);
    session
        .send_command("Page.navigate", json!({"url":preview_url}))
        .await?;
    timeout(READY_TIMEOUT, async {
        loop {
            let value = session
                .send_command(
                    "Runtime.evaluate",
                    json!({"expression":"document.documentElement.dataset.jaunderThumbnailReady === '1' && Array.from(document.images).every(image => image.complete) && Array.from(document.styleSheets).some(sheet => sheet.href === location.origin + '/theme.css')","returnByValue":true}),
                )
                .await?;
            if value.pointer("/result/value").and_then(Value::as_bool) == Some(true)
                && session.state.in_flight.is_empty()
                && session.state.network_idle
            {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("wait for thumbnail fixture resources and readiness")??;
    session
        .requests
        .verify_required_requests(&session.state.accepted_requests)?;
    session
        .send_command(
            "Runtime.evaluate",
            json!({"expression":"const style=document.createElement('style');style.textContent='*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}';document.head.append(style);document.getAnimations().forEach(animation=>animation.cancel());new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))","awaitPromise":true,"returnByValue":true}),
        )
        .await?;
    let shot = session
        .send_command(
            "Page.captureScreenshot",
            json!({"format":"png","fromSurface":true,"captureBeyondViewport":false}),
        )
        .await?;
    let encoded = shot
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Chromium omitted PNG screenshot bytes"))?;
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .context("decode Chromium PNG screenshot")
}

#[cfg(test)]
mod tests {
    use super::{
        PreviewRequestPolicy, PreviewServer, cdp_capture, cmd_theme_thumbnail,
        join_chromium_stderr_with_timeout, merge_primary_and_cleanup, page_endpoint,
        preview_origin_allowed, read_chromium_diagnostics,
    };
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use std::{collections::HashSet, fs, os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
    use tokio::io::AsyncWriteExt;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    fn repository_with_asset() -> (
        tempfile::TempDir,
        Arc<host::theme_repository::AcceptedThemeRepository>,
    ) {
        let repository = tempfile::tempdir().expect("repository");
        fs::write(
            repository.path().join("theme.json"),
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{"assets/pixel.avif":"image/avif"},"defaults":{}}"#,
        )
        .expect("manifest");
        fs::write(
            repository.path().join("style.css"),
            "body { background-image: url(assets/pixel.avif); }",
        )
        .expect("stylesheet");
        fs::create_dir(repository.path().join("assets")).expect("asset directory");
        fs::write(
            repository.path().join("assets/pixel.avif"),
            include_bytes!("../../../host/src/theme_package/fixtures/one-pixel.avif"),
        )
        .expect("asset");
        let accepted = host::theme_repository::accept_theme_repository(repository.path())
            .expect("accepted repository");
        (repository, Arc::new(accepted))
    }

    #[tokio::test]
    async fn preview_server_serves_fixture_css_and_declared_assets_only() {
        let (_repository, accepted) = repository_with_asset();
        let expected_css = accepted.revision().css().bytes().to_vec();
        let expected_asset = accepted
            .revision()
            .asset("assets/pixel.avif")
            .expect("declared asset")
            .1
            .to_vec();
        let preview = PreviewServer::start("<main>fixture document</main>".to_owned(), accepted)
            .await
            .expect("preview server");
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("isolated preview client");

        let document = client
            .get(format!("{}/", preview.origin()))
            .send()
            .await
            .expect("document response");
        assert_eq!(document.status(), reqwest::StatusCode::OK);
        assert_eq!(
            document.text().await.expect("document body"),
            "<main>fixture document</main>"
        );

        for stylesheet in ["jaunder.css", "jaunder-themes.css"] {
            let response = client
                .get(format!("{}/style/{stylesheet}", preview.origin()))
                .send()
                .await
                .expect("embedded stylesheet response");
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            assert_eq!(
                response.headers().get(reqwest::header::CONTENT_TYPE),
                Some(&reqwest::header::HeaderValue::from_static("text/css"))
            );
            let embedded =
                crate::assets::StaticAssets::get(stylesheet).expect("embedded stylesheet");
            assert_eq!(
                response
                    .bytes()
                    .await
                    .expect("embedded stylesheet body")
                    .as_ref(),
                embedded.data.as_ref()
            );
        }

        let css = client
            .get(format!("{}/theme.css", preview.origin()))
            .send()
            .await
            .expect("stylesheet response");
        assert_eq!(css.status(), reqwest::StatusCode::OK);
        assert_eq!(
            css.bytes().await.expect("stylesheet body").as_ref(),
            expected_css
        );

        let asset = client
            .get(format!(
                "{}/theme-assets/assets/pixel.avif",
                preview.origin()
            ))
            .send()
            .await
            .expect("asset response");
        assert_eq!(asset.status(), reqwest::StatusCode::OK);
        assert_eq!(
            asset.headers().get(reqwest::header::CONTENT_TYPE),
            Some(&reqwest::header::HeaderValue::from_static("image/avif"))
        );
        assert_eq!(
            asset.bytes().await.expect("asset body").as_ref(),
            expected_asset
        );

        let missing = client
            .get(format!(
                "{}/theme-assets/assets/missing.avif",
                preview.origin()
            ))
            .send()
            .await
            .expect("missing asset response");
        assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
        preview.stop().await.expect("stop preview server");
    }

    #[test]
    fn preview_request_policy_admits_only_declared_resources_and_requires_base_fixture_files() {
        let (_repository, accepted) = repository_with_asset();
        let origin = "http://127.0.0.1:41237";
        let requests = PreviewRequestPolicy::new(origin, accepted.revision());

        for path in [
            "/",
            "/style/jaunder.css",
            "/style/jaunder-themes.css",
            "/theme.css",
            "/theme-assets/assets/pixel.avif",
        ] {
            assert!(requests.allows(&format!("{origin}{path}")), "{path}");
        }
        assert!(!requests.allows(&format!("{origin}/theme.css?cache=1")));
        assert!(!requests.allows(&format!("{origin}/theme-assets/assets/missing.avif")));
        assert!(!requests.allows("http://127.0.0.1:41238/theme.css"));

        let accepted = HashSet::from([
            format!("{origin}/"),
            format!("{origin}/style/jaunder.css"),
            format!("{origin}/style/jaunder-themes.css"),
            format!("{origin}/theme.css"),
        ]);
        requests
            .verify_required_requests(&accepted)
            .expect("all required resources were accepted");

        let missing = HashSet::from([format!("{origin}/")]);
        let error = requests
            .verify_required_requests(&missing)
            .expect_err("stylesheet requests are required");
        assert!(
            format!("{error:#}").contains("Chromium did not request required thumbnail resource")
        );
    }

    #[tokio::test]
    async fn rejected_repository_fails_before_a_missing_browser_can_be_spawned() {
        let repository = tempfile::tempdir().expect("repository");
        let output = repository.path().join("preview.png");

        let error = cmd_theme_thumbnail(
            repository.path(),
            &repository.path().join("missing-browser"),
            &output,
        )
        .await
        .expect_err("repository is missing required package members");

        assert!(format!("{error:#}").contains("theme repository validation failed"));
        assert!(!output.exists());
    }

    #[tokio::test]
    async fn admitted_repository_reports_an_unavailable_browser() {
        let repository = tempfile::tempdir().expect("repository");
        fs::write(
            repository.path().join("theme.json"),
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#,
        )
        .expect("manifest");
        fs::write(
            repository.path().join("style.css"),
            "body { color: black; }",
        )
        .expect("css");
        let output = repository.path().join("preview.png");

        let error = cmd_theme_thumbnail(
            repository.path(),
            &repository.path().join("missing-browser"),
            &output,
        )
        .await
        .expect_err("browser executable is unavailable");

        assert!(format!("{error:#}").contains("start Chromium browser"));
        assert!(!output.exists());
    }

    #[tokio::test]
    async fn browser_exit_before_devtools_is_a_command_failure() {
        let repository = tempfile::tempdir().expect("repository");
        fs::write(
            repository.path().join("theme.json"),
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#,
        )
        .expect("manifest");
        fs::write(
            repository.path().join("style.css"),
            "body { color: black; }",
        )
        .expect("css");
        let browser = repository.path().join("browser");
        fs::write(&browser, "#!/bin/sh\nexit 7\n").expect("browser script");
        let mut permissions = fs::metadata(&browser)
            .expect("browser metadata")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&browser, permissions).expect("browser executable");

        let error = cmd_theme_thumbnail(
            repository.path(),
            &browser,
            &repository.path().join("preview.png"),
        )
        .await
        .expect_err("browser exits before exposing DevTools");

        assert!(format!("{error:#}").contains("Chromium exited before DevTools became available"));
    }

    #[tokio::test]
    async fn chromium_diagnostics_forwards_only_the_first_loopback_endpoint() {
        let (mut input, output) = tokio::io::duplex(512);
        input
            .write_all(
                b"noise\nDevTools listening on ws://127.0.0.1:41237/devtools/browser/1\nws://127.0.0.1:41238/devtools/browser/2\n",
            )
            .await
            .expect("diagnostics");
        drop(input);
        let (endpoint, received) = tokio::sync::oneshot::channel();

        read_chromium_diagnostics(tokio::io::BufReader::new(output), endpoint)
            .await
            .expect("read diagnostics");

        assert_eq!(
            received.await.expect("discovered endpoint"),
            "ws://127.0.0.1:41237/devtools/browser/1"
        );
    }

    #[tokio::test]
    async fn thumbnail_command_publishes_a_successful_loopback_capture() {
        let repository = tempfile::tempdir().expect("repository");
        fs::write(
            repository.path().join("theme.json"),
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{"assets/header-a.avif":"image/avif","assets/header-b.avif":"image/avif","assets/logo.avif":"image/avif"},"defaults":{"logo":"assets/logo.avif","header":["assets/header-a.avif","assets/header-b.avif"]}}"#,
        )
        .expect("manifest");
        fs::write(
            repository.path().join("style.css"),
            "body { color: black; }",
        )
        .expect("css");
        fs::create_dir(repository.path().join("assets")).expect("asset directory");
        for name in ["header-a.avif", "header-b.avif", "logo.avif"] {
            fs::write(
                repository.path().join("assets").join(name),
                include_bytes!("../../../host/src/theme_package/fixtures/one-pixel.avif"),
            )
            .expect("default image");
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("DevTools listener");
        let address = listener.local_addr().expect("DevTools address");
        let server = tokio::spawn(async move {
            let (mut target_socket, _) = listener.accept().await.expect("target request");
            let page = format!("ws://{address}/devtools/page/1");
            let body = format!(r#"[{{"type":"page","webSocketDebuggerUrl":"{page}"}}]"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            target_socket
                .write_all(response.as_bytes())
                .await
                .expect("target response");
            drop(target_socket);
            let (stream, _) = listener.accept().await.expect("CDP connection");
            let mut socket = accept_async(stream).await.expect("WebSocket handshake");
            serve_successful_cdp(&mut socket).await;
        });
        let browser = repository.path().join("browser");
        fs::write(
            &browser,
            format!(
                "#!/bin/sh\necho 'DevTools listening on ws://{address}/devtools/browser/1' >&2\nexec sleep 30\n"
            ),
        )
        .expect("browser script");
        let mut permissions = fs::metadata(&browser)
            .expect("browser metadata")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&browser, permissions).expect("browser executable");
        let output = repository.path().join("preview.png");

        cmd_theme_thumbnail(repository.path(), &browser, &output)
            .await
            .expect("successful thumbnail command");

        assert_eq!(fs::read(output).expect("published thumbnail"), b"png");
        server.await.expect("DevTools server");
    }

    #[tokio::test]
    async fn devtools_connection_failure_is_reported() {
        let requests = PreviewRequestPolicy {
            origin: "http://127.0.0.1:9".to_owned(),
            allowed: HashSet::new(),
        };
        let error = cdp_capture("ws://127.0.0.1:9/devtools/page/1", &requests)
            .await
            .expect_err("DevTools endpoint refuses connections");

        assert!(format!("{error:#}").contains("connect to Chromium DevTools"));
    }

    #[tokio::test]
    async fn page_endpoint_requires_a_loopback_browser_and_same_listener_page() {
        let error = page_endpoint("ws://example.invalid:9222/devtools/browser/1")
            .await
            .expect_err("non-loopback browser endpoint");
        assert!(format!("{error:#}").contains("browser endpoint is not loopback WebSocket"));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("request");
            let page = format!("ws://{address}/devtools/page/1");
            let body = format!(r#"[{{"type":"page","webSocketDebuggerUrl":"{page}"}}]"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response");
        });

        assert_eq!(
            page_endpoint(&format!("ws://{address}/devtools/browser/1"))
                .await
                .expect("same-listener page endpoint"),
            format!("ws://{address}/devtools/page/1")
        );
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn devtools_target_http_failure_is_reported() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("request");
            socket
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                .await
                .expect("response");
        });

        let error = page_endpoint(&format!("ws://{address}/devtools/browser/1"))
            .await
            .expect_err("target endpoint rejects request");
        server.await.expect("server task");

        assert!(format!("{error:#}").contains("read Chromium DevTools targets"));
    }

    #[tokio::test]
    async fn devtools_target_http_redirect_is_rejected() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("request");
            socket
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://example.invalid/json/list\r\nContent-Length: 0\r\n\r\n",
                )
                .await
                .expect("response");
        });

        let error = page_endpoint(&format!("ws://{address}/devtools/browser/1"))
            .await
            .expect_err("target endpoint redirect is rejected");
        server.await.expect("server task");

        assert!(format!("{error:#}").contains("attempted an HTTP redirect"));
    }
    #[tokio::test]
    async fn devtools_target_cannot_redirect_the_adapter_off_loopback() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("request");
            let body = r#"[{"type":"page","webSocketDebuggerUrl":"ws://example.invalid/devtools/page/1"}]"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response");
        });

        let error = page_endpoint(&format!("ws://{address}/devtools/browser/1"))
            .await
            .expect_err("page endpoint leaves loopback listener");
        server.await.expect("server task");

        assert!(
            format!("{error:#}")
                .contains("page DevTools endpoint left the loopback debugging listener")
        );
    }

    #[test]
    fn cleanup_failure_prevents_a_successful_capture_from_publishing() {
        let error = merge_primary_and_cleanup(
            Ok::<_, anyhow::Error>(()),
            Err(anyhow::anyhow!("cleanup failed")),
            "clean up capture",
        )
        .expect_err("cleanup failure must fail capture");

        assert!(format!("{error:#}").contains("cleanup failed"));
    }

    #[test]
    fn capture_failure_remains_primary_when_cleanup_also_fails() {
        let error = merge_primary_and_cleanup::<()>(
            Err(anyhow::anyhow!("capture failed")),
            Err(anyhow::anyhow!("cleanup failed")),
            "clean up capture",
        )
        .expect_err("capture must fail");

        assert_eq!(error.to_string(), "capture failed");
    }

    #[test]
    fn request_gate_allows_only_the_preview_origin() {
        let origin = "http://127.0.0.1:41237";
        assert!(preview_origin_allowed(
            origin,
            "http://127.0.0.1:41237/theme.css"
        ));
        assert!(!preview_origin_allowed(
            origin,
            "https://example.invalid/theme.css"
        ));
        assert!(!preview_origin_allowed(
            origin,
            "http://127.0.0.1:41238/theme.css"
        ));
    }
    fn test_request_policy() -> PreviewRequestPolicy {
        let origin = "http://127.0.0.1:41237";
        PreviewRequestPolicy {
            origin: origin.to_owned(),
            allowed: HashSet::from([
                format!("{origin}/"),
                format!("{origin}/style/jaunder.css"),
                format!("{origin}/style/jaunder-themes.css"),
                format!("{origin}/theme.css"),
            ]),
        }
    }

    async fn next_command(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        expected_method: &str,
    ) -> Value {
        let request: Value = serde_json::from_str(
            &socket
                .next()
                .await
                .expect("command")
                .expect("WebSocket command")
                .into_text()
                .expect("text command"),
        )
        .expect("JSON command");
        assert_eq!(request["method"], expected_method);
        request
    }

    async fn reply(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        request: &Value,
        result: Value,
    ) {
        socket
            .send(Message::Text(
                json!({"id":request["id"],"result":result})
                    .to_string()
                    .into(),
            ))
            .await
            .expect("response");
    }
    async fn serve_successful_cdp(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) {
        for method in [
            "Page.enable",
            "Network.enable",
            "Page.setLifecycleEventsEnabled",
            "Fetch.enable",
            "Emulation.setDeviceMetricsOverride",
            "Emulation.setEmulatedMedia",
        ] {
            if method == "Page.enable" {
                socket
                    .send(Message::Text(
                        json!({"method":"Runtime.consoleAPICalled","params":{}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .expect("unrelated event");
            }
            let request = next_command(socket, method).await;
            reply(socket, &request, json!({})).await;
        }
        let navigate = next_command(socket, "Page.navigate").await;
        let origin = url::Url::parse(
            navigate["params"]["url"]
                .as_str()
                .expect("preview navigation URL"),
        )
        .expect("valid preview navigation URL")
        .origin()
        .ascii_serialization();
        for (index, path) in [
            "/",
            "/style/jaunder.css",
            "/style/jaunder-themes.css",
            "/theme.css",
        ]
        .into_iter()
        .enumerate()
        {
            socket
                .send(Message::Text(
                    json!({"method":"Fetch.requestPaused","params":{"requestId":format!("fetch-{index}"),"networkId":format!("network-{index}"),"request":{"url":format!("{origin}{path}")}}})
                        .to_string()
                        .into(),
                ))
                .await
                .expect("request pause");
            let continue_request = next_command(socket, "Fetch.continueRequest").await;
            assert_eq!(
                continue_request["params"]["requestId"],
                format!("fetch-{index}")
            );
        }
        reply(socket, &navigate, json!({})).await;
        let not_ready = next_command(socket, "Runtime.evaluate").await;
        reply(socket, &not_ready, json!({"result":{"value":false}})).await;
        let ready = next_command(socket, "Runtime.evaluate").await;
        for index in 0..4 {
            socket
                .send(Message::Text(
                    json!({"method":"Network.loadingFinished","params":{"requestId":format!("network-{index}")}})
                        .to_string()
                        .into(),
                ))
                .await
                .expect("network complete");
        }
        socket
            .send(Message::Text(
                json!({"method":"Page.lifecycleEvent","params":{"name":"networkIdle"}})
                    .to_string()
                    .into(),
            ))
            .await
            .expect("network idle");
        reply(socket, &ready, json!({"result":{"value":true}})).await;
        let freeze = next_command(socket, "Runtime.evaluate").await;
        reply(socket, &freeze, json!({})).await;
        let screenshot = next_command(socket, "Page.captureScreenshot").await;
        reply(socket, &screenshot, json!({"data":"cG5n"})).await;
    }

    #[tokio::test]
    async fn cdp_capture_intercepts_required_requests_waits_for_readiness_and_decodes_png() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("CDP listener");
        let endpoint = format!(
            "ws://{}/devtools/page/1",
            listener.local_addr().expect("address")
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("CDP connection");
            let mut socket = accept_async(stream).await.expect("WebSocket handshake");
            serve_successful_cdp(&mut socket).await;
        });

        assert_eq!(
            cdp_capture(&endpoint, &test_request_policy())
                .await
                .expect("successful CDP capture"),
            b"png"
        );
        server.await.expect("CDP server");
    }

    #[tokio::test]
    async fn cdp_capture_rejects_disallowed_requests_after_blocking_them() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("CDP listener");
        let endpoint = format!(
            "ws://{}/devtools/page/1",
            listener.local_addr().expect("address")
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("CDP connection");
            let mut socket = accept_async(stream).await.expect("WebSocket handshake");
            let enable = next_command(&mut socket, "Page.enable").await;
            reply(&mut socket, &enable, json!({})).await;
            let network = next_command(&mut socket, "Network.enable").await;
            socket
                .send(Message::Text(
                    json!({"method":"Fetch.requestPaused","params":{"requestId":"blocked","request":{"url":"https://example.invalid/tracker"}}})
                        .to_string()
                        .into(),
                ))
                .await
                .expect("request pause");
            let blocked = next_command(&mut socket, "Fetch.failRequest").await;
            assert_eq!(blocked["params"]["errorReason"], "BlockedByClient");
            reply(&mut socket, &network, json!({})).await;
        });

        let error = cdp_capture(&endpoint, &test_request_policy())
            .await
            .expect_err("disallowed request");
        assert!(format!("{error:#}").contains("disallowed thumbnail request"));
        server.await.expect("CDP server");
    }

    #[tokio::test]
    async fn cdp_capture_reports_protocol_errors_and_closed_connections() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("CDP listener");
        let endpoint = format!(
            "ws://{}/devtools/page/1",
            listener.local_addr().expect("address")
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("CDP connection");
            let mut socket = accept_async(stream).await.expect("WebSocket handshake");
            let request = next_command(&mut socket, "Page.enable").await;
            socket
                .send(Message::Text(
                    json!({"id":request["id"],"error":{"message":"denied"}})
                        .to_string()
                        .into(),
                ))
                .await
                .expect("protocol error");
        });
        let error = cdp_capture(&endpoint, &test_request_policy())
            .await
            .expect_err("CDP error");
        assert!(format!("{error:#}").contains("Page.enable failed"));
        server.await.expect("CDP server");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("CDP listener");
        let endpoint = format!(
            "ws://{}/devtools/page/1",
            listener.local_addr().expect("address")
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("CDP connection");
            let mut socket = accept_async(stream).await.expect("WebSocket handshake");
            socket.close(None).await.expect("close CDP connection");
        });
        let error = cdp_capture(&endpoint, &test_request_policy())
            .await
            .expect_err("closed CDP connection");
        assert!(format!("{error:#}").contains("connection closed during Page.enable"));
        server.await.expect("CDP server");
    }

    #[tokio::test]
    async fn chromium_stderr_join_timeout_cancels_the_reader() {
        let task = tokio::spawn(std::future::pending::<anyhow::Result<()>>());
        let error = join_chromium_stderr_with_timeout(task, Duration::from_millis(1))
            .await
            .expect_err("stalled stderr reader");
        assert!(format!("{error:#}").contains("time out joining Chromium diagnostics reader"));
    }
}
