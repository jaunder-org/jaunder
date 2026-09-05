use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::mailer::MailSender;
use common::media::MediaReferenceForm;
use common::tagged_url::BaseUrl;
use storage::test_support::{Backend, TestEnv, confirmed_for, noop_mailer};
use storage::{
    ForeignEvidenceSink, InstanceId, MediaReferenceEvidence, MediaReferenceOwnershipResolver,
    PasswordResetStorage, PersistedMediaReference, SiteConfigStorage, UserStorage, WriteScope,
};
use tempfile::TempDir;
use tower::ServiceExt;

use super::registrar::ensure_server_fns_registered;
use super::session::tmp_storage_path;
pub fn confirmed_mutation<T: serde::de::DeserializeOwned>(body: &str) -> T {
    let outcome: common::MutationOutcome<T> =
        serde_json::from_str(body).expect("parse mutation outcome");
    confirmed_for(outcome, "integration test backend")
}

/// Read a response body fully and decode it as UTF-8.
pub async fn body_string(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Build a fresh router from `state` over `storage` as the media root, with the
/// noop mailer and insecure cookies. Always creates the `media/{upload,cached,tmp}`
/// layout so upload-exercising and read-only tests share one helper (the dirs are
/// harmless empty setup for tests that never upload).
pub fn make_app(state: &Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    std::fs::create_dir_all(storage_path.join("media").join("upload")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("cached")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("tmp")).unwrap();
    jaunder::create_router(
        Arc::clone(state),
        storage::InstanceId::new(),
        noop_mailer(),
        false,
        storage_path,
    )
    .expect("canonical instance identity is an HTTP header")
}

/// Build a router with a deterministic ownership resolver for media-deletion
/// integration tests. The production helper above deliberately keeps the live
/// resolver so ordinary route tests exercise normal composition.
pub fn make_app_with_media_ownership_resolver(
    state: &Arc<storage::AppState>,
    storage: &TempDir,
    resolver: Arc<dyn storage::MediaReferenceOwnershipResolver>,
) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    std::fs::create_dir_all(storage_path.join("media").join("upload")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("cached")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("tmp")).unwrap();
    jaunder::create_router_with_media_reference_ownership_resolver(
        Arc::clone(state),
        storage::InstanceId::new(),
        noop_mailer(),
        false,
        storage_path,
        resolver,
    )
    .expect("canonical instance identity is an HTTP header")
}

/// Deterministic resolver that proves only configured exact persisted URL forms
/// foreign. It records each input batch so endpoint tests can assert one global
/// resolution before deletion takes locks.
pub struct ForeignReferenceResolver {
    foreign_forms: Mutex<BTreeSet<MediaReferenceForm>>,
    calls: Mutex<Vec<Vec<PersistedMediaReference>>>,
}

impl ForeignReferenceResolver {
    pub fn new(foreign_forms: impl IntoIterator<Item = MediaReferenceForm>) -> Self {
        Self {
            foreign_forms: Mutex::new(foreign_forms.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn insert_foreign_form(&self, reference_form: MediaReferenceForm) {
        self.foreign_forms
            .lock()
            .expect("foreign forms lock")
            .insert(reference_form);
    }

    pub fn calls(&self) -> Vec<Vec<PersistedMediaReference>> {
        self.calls.lock().expect("resolver calls lock").clone()
    }
}

#[async_trait]
impl MediaReferenceOwnershipResolver for ForeignReferenceResolver {
    async fn resolve(
        &self,
        references: &[PersistedMediaReference],
        _instance_id: &InstanceId,
        _base_url: Option<&BaseUrl>,
        mut foreign: ForeignEvidenceSink,
    ) -> MediaReferenceEvidence {
        self.calls
            .lock()
            .expect("resolver calls lock")
            .push(references.to_vec());
        for reference in references {
            if self
                .foreign_forms
                .lock()
                .expect("foreign forms lock")
                .contains(reference.reference_form())
            {
                foreign.prove_foreign(reference.clone());
            }
        }
        foreign.finish()
    }
}

struct RequestCredentials<'a> {
    cookie: Option<&'a str>,
    authorization: Option<&'a str>,
}

/// A POST body paired with its content type — the two always travel together, so
/// they are one argument. `Form` is `application/x-www-form-urlencoded`, `Json` is
/// `application/json`.
enum PostBody {
    Form(String),
    Json(String),
}

impl PostBody {
    fn server_fn<I>(input: &I) -> Self
    where
        I: serde::Serialize,
    {
        Self::Form(serde_qs::to_string(input).expect("failed to encode server-function input"))
    }

    fn content_type(&self) -> &'static str {
        match self {
            PostBody::Form(_) => "application/x-www-form-urlencoded",
            PostBody::Json(_) => "application/json",
        }
    }

    fn into_string(self) -> String {
        match self {
            PostBody::Form(s) | PostBody::Json(s) => s,
        }
    }
}

/// Full response data for auth-sensitive request helpers.
pub struct TestHttpResponse {
    pub status: StatusCode,
    pub set_cookies: Vec<String>,
    pub body: String,
}

impl TestHttpResponse {
    fn into_without_cookies(self) -> (StatusCode, String) {
        (self.status, self.body)
    }

    fn into_first_cookie(self) -> (StatusCode, Option<String>, String) {
        (self.status, self.set_cookies.into_iter().next(), self.body)
    }
}

/// The single implementation behind every `post_form*`/`post_json` helper: build
/// a fresh router from `state` (with `mailer` and `secure_cookies`), send one POST
/// with the given body, and return the complete auth-relevant response.
async fn post_inner(
    state: &Arc<storage::AppState>,
    mailer: Arc<dyn MailSender>,
    uri: &str,
    body: PostBody,
    credentials: RequestCredentials<'_>,
    user_agent: Option<&str>,
    secure_cookies: bool,
) -> TestHttpResponse {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, body.content_type());
    if let Some(cookie) = credentials.cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(authorization) = credentials.authorization {
        builder = builder.header(header::AUTHORIZATION, authorization);
    }
    if let Some(ua) = user_agent {
        builder = builder.header(header::USER_AGENT, ua);
    }
    let request = builder
        .body(Body::from(body.into_string()))
        .expect("failed to build request");

    let app = jaunder::create_router(
        Arc::clone(state),
        storage::InstanceId::new(),
        mailer,
        secure_cookies,
        tmp_storage_path(),
    )
    .expect("canonical instance identity is an HTTP header");
    let response = app.oneshot(request).await.expect("router oneshot failed");

    let status = response.status();
    let set_cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| {
            value
                .to_str()
                .expect("Set-Cookie header is not valid UTF-8")
                .to_owned()
        })
        .collect();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body = String::from_utf8(bytes.to_vec()).expect("response body is not valid UTF-8");

    TestHttpResponse {
        status,
        set_cookies,
        body,
    }
}

/// Canonical case: noop mailer, secure cookies, cookie auth, `Set-Cookie` dropped.
pub async fn post_form(
    state: &Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Form(body.into()),
        RequestCredentials {
            cookie,
            authorization: None,
        },
        None,
        true,
    )
    .await
    .into_without_cookies()
}

/// Shared typed/fixture server-function dispatcher. `F` selects the generated
/// endpoint path; `I` supplies the serializable wire shape.
async fn post_server_fn_inner<F, I>(
    state: &Arc<storage::AppState>,
    mailer: Arc<dyn MailSender>,
    input: &I,
    cookie: Option<&str>,
    user_agent: Option<&str>,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String)
where
    F: server_fn::ServerFn,
    I: serde::Serialize,
{
    post_inner(
        state,
        mailer,
        F::PATH,
        PostBody::server_fn(input),
        RequestCredentials {
            cookie,
            authorization: None,
        },
        user_agent,
        secure_cookies,
    )
    .await
    .into_first_cookie()
}

/// POST one typed server-function input using that function's derived path and
/// default URL-encoded input codec.
pub async fn post_server_fn<F>(
    state: &Arc<storage::AppState>,
    input: &F,
    cookie: Option<&str>,
) -> (StatusCode, String)
where
    F: serde::Serialize + server_fn::ServerFn,
{
    let (status, _set_cookie, body) =
        post_server_fn_inner::<F, F>(state, noop_mailer(), input, cookie, None, true).await;
    (status, body)
}

/// Posts a typed server function through a router with deterministic ownership
/// evidence, keeping media-deletion integration tests off the network.
pub async fn post_server_fn_with_media_ownership_resolver<F>(
    state: &Arc<storage::AppState>,
    resolver: Arc<dyn storage::MediaReferenceOwnershipResolver>,
    input: &F,
    cookie: Option<&str>,
) -> (StatusCode, String)
where
    F: serde::Serialize + server_fn::ServerFn,
{
    let storage = TempDir::new().unwrap();
    let app = make_app_with_media_ownership_resolver(state, &storage, resolver);
    let mut builder = Request::builder()
        .method("POST")
        .uri(F::PATH)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let request = builder
        .body(Body::from(
            serde_qs::to_string(input).expect("server function input encodes"),
        ))
        .expect("server function request builds");
    let response = app.oneshot(request).await.expect("router request succeeds");
    let status = response.status();
    let body = body_string(response).await;
    (status, body)
}

/// Posts the reset request through the production router while replacing only
/// the detached reset worker's erased dependencies.
pub async fn post_password_reset_request_with_dependencies(
    state: &Arc<storage::AppState>,
    mailer: Arc<dyn MailSender>,
    input: &web::password_reset::Request,
    users: Arc<dyn UserStorage>,
    password_resets: Arc<dyn PasswordResetStorage>,
    write_scope: WriteScope,
    site_config: Arc<dyn SiteConfigStorage>,
) -> (StatusCode, String) {
    post_password_reset_form_with_dependencies(
        state,
        mailer,
        serde_qs::to_string(input).expect("server function input encodes"),
        users,
        password_resets,
        write_scope,
        site_config,
    )
    .await
}

/// Posts a raw password-reset request form through the production router with
/// explicit detached-worker dependencies. Decode-rejection tests use this to
/// prove invalid input never starts detached work.
pub async fn post_password_reset_form_with_dependencies(
    state: &Arc<storage::AppState>,
    mailer: Arc<dyn MailSender>,
    body: impl Into<String>,
    users: Arc<dyn UserStorage>,
    password_resets: Arc<dyn PasswordResetStorage>,
    write_scope: WriteScope,
    site_config: Arc<dyn SiteConfigStorage>,
) -> (StatusCode, String) {
    let storage = TempDir::new().expect("test storage directory");
    let app = jaunder::create_router_with_password_reset_dependencies_for_test(
        Arc::clone(state),
        mailer,
        storage.path().to_path_buf(),
        users,
        password_resets,
        write_scope,
        site_config,
    )
    .expect("canonical instance identity is an HTTP header");
    let request = Request::builder()
        .method("POST")
        .uri(<web::password_reset::Request as server_fn::ServerFn>::PATH)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.into()))
        .expect("server function request builds");
    let response = app.oneshot(request).await.expect("router request succeeds");
    let status = response.status();
    let body = body_string(response).await;
    (status, body)
}

#[derive(serde::Serialize)]
struct RequestFixture<'a, R> {
    request: &'a R,
}

/// POST a serializable request-aggregate fixture to server function `F`.
///
/// Decode-rejection tests use this when an invalid value cannot inhabit the
/// operation's typed request. Valid requests use [`post_server_fn`].
pub async fn post_server_fn_request_fixture<F, R>(
    state: &Arc<storage::AppState>,
    request: &R,
    cookie: Option<&str>,
) -> (StatusCode, String)
where
    F: server_fn::ServerFn,
    R: serde::Serialize,
{
    let input = RequestFixture { request };
    let (status, _set_cookie, body) =
        post_server_fn_inner::<F, _>(state, noop_mailer(), &input, cookie, None, true).await;
    (status, body)
}

/// Like [`post_server_fn`], but injects a specific mailer.
pub async fn post_server_fn_with_mailer<M, F>(
    state: &Arc<storage::AppState>,
    mailer: &Arc<M>,
    input: &F,
    cookie: Option<&str>,
) -> (StatusCode, String)
where
    M: MailSender + 'static,
    F: serde::Serialize + server_fn::ServerFn,
{
    let mailer: Arc<dyn MailSender> = mailer.clone();
    let (status, _set_cookie, body) =
        post_server_fn_inner::<F, F>(state, mailer, input, cookie, None, true).await;
    (status, body)
}

/// Fixture counterpart to [`post_server_fn_with_mailer`].
pub async fn post_server_fn_request_fixture_with_mailer<F, M, R>(
    state: &Arc<storage::AppState>,
    mailer: &Arc<M>,
    request: &R,
    cookie: Option<&str>,
) -> (StatusCode, String)
where
    M: MailSender + 'static,
    F: server_fn::ServerFn,
    R: serde::Serialize,
{
    let input = RequestFixture { request };
    let mailer: Arc<dyn MailSender> = mailer.clone();
    let (status, _set_cookie, body) =
        post_server_fn_inner::<F, _>(state, mailer, &input, cookie, None, true).await;
    (status, body)
}

/// Like [`post_server_fn`], but exposes the secure-cookie toggle and returns
/// `Set-Cookie`.
pub async fn post_server_fn_with_secure_flag<F>(
    state: &Arc<storage::AppState>,
    input: &F,
    cookie: Option<&str>,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String)
where
    F: serde::Serialize + server_fn::ServerFn,
{
    post_server_fn_inner::<F, F>(state, noop_mailer(), input, cookie, None, secure_cookies).await
}

/// Fixture counterpart to [`post_server_fn_with_secure_flag`].
pub async fn post_server_fn_request_fixture_with_secure_flag<F, R>(
    state: &Arc<storage::AppState>,
    request: &R,
    cookie: Option<&str>,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String)
where
    F: server_fn::ServerFn,
    R: serde::Serialize,
{
    let input = RequestFixture { request };
    post_server_fn_inner::<F, _>(state, noop_mailer(), &input, cookie, None, secure_cookies).await
}

/// Like [`post_server_fn_with_secure_flag`], but also sets `User-Agent`.
pub async fn post_server_fn_with_ua<F>(
    state: &Arc<storage::AppState>,
    input: &F,
    cookie: Option<&str>,
    user_agent: &str,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String)
where
    F: serde::Serialize + server_fn::ServerFn,
{
    post_server_fn_inner::<F, F>(
        state,
        noop_mailer(),
        input,
        cookie,
        Some(user_agent),
        secure_cookies,
    )
    .await
}

/// Like [`post_form`], but injects a specific `mailer` (e.g. a capturing sender)
/// instead of the noop.
pub async fn post_form_with_mailer<M: MailSender + 'static>(
    state: &Arc<storage::AppState>,
    mailer: &Arc<M>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    // The router consumes an owned `Arc<dyn MailSender>`; borrow at the call site and
    // do the single clone-and-unsize (`Arc<M>` -> `Arc<dyn MailSender>`) here.
    let mailer: Arc<dyn MailSender> = mailer.clone();
    post_inner(
        state,
        mailer,
        uri,
        PostBody::Form(body.into()),
        RequestCredentials {
            cookie,
            authorization: None,
        },
        None,
        true,
    )
    .await
    .into_without_cookies()
}

/// Exposes the `secure_cookies` toggle and returns the `Set-Cookie` value —
/// what the auth/session tests need over the canonical [`post_form`].
pub async fn post_form_with_secure_flag(
    state: &Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String) {
    post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Form(body.into()),
        RequestCredentials {
            cookie,
            authorization: None,
        },
        None,
        secure_cookies,
    )
    .await
    .into_first_cookie()
}

/// Authenticates with an `Authorization: Bearer <token>` header instead of a
/// cookie. Returns the first `Set-Cookie` value like the existing auth helpers.
pub async fn post_form_with_bearer(
    state: &Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    bearer: &str,
) -> (StatusCode, Option<String>, String) {
    let authorization = format!("Bearer {bearer}");
    post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Form(body.into()),
        RequestCredentials {
            cookie: None,
            authorization: Some(&authorization),
        },
        None,
        true,
    )
    .await
    .into_first_cookie()
}

/// Sends form data with cookie and Authorization headers controlled independently.
pub async fn post_form_with_credentials(
    state: &Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
    authorization: Option<&str>,
    secure_cookies: bool,
) -> TestHttpResponse {
    post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Form(body.into()),
        RequestCredentials {
            cookie,
            authorization,
        },
        None,
        secure_cookies,
    )
    .await
}

/// POST a JSON body (`Content-Type: application/json`) with secure cookies and
/// optional cookie auth; returns `(status, body)` — drops `Set-Cookie`, like the
/// canonical [`post_form`].
pub async fn post_json(
    state: &Arc<storage::AppState>,
    uri: &str,
    body: serde_json::Value,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json_with_credentials(state, uri, body, cookie, None, true)
        .await
        .into_without_cookies()
}

/// Sends JSON with cookie and Authorization headers controlled independently.
pub async fn post_json_with_credentials(
    state: &Arc<storage::AppState>,
    uri: &str,
    body: serde_json::Value,
    cookie: Option<&str>,
    authorization: Option<&str>,
    secure_cookies: bool,
) -> TestHttpResponse {
    post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Json(body.to_string()),
        RequestCredentials {
            cookie,
            authorization,
        },
        None,
        secure_cookies,
    )
    .await
}

/// A single `multipart/form-data` file field, as [`post_multipart`] sends it.
pub struct MultipartFile<'a> {
    pub filename: &'a str,
    pub content_type: &'a str,
    pub bytes: &'a [u8],
}

/// POST a single-file `multipart/form-data` body to `uri` against a router built
/// over `storage` as a real writable media root (via [`make_app`]), so the upload
/// lands on disk. Returns `(status, body)`. Mirrors the exact CRLF framing of the
/// multipart request in `misc/media_handlers.rs`.
pub async fn post_multipart(
    state: &Arc<storage::AppState>,
    storage: &TempDir,
    uri: &str,
    file: MultipartFile<'_>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let boundary = "----testboundary1234";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
            file.filename, file.content_type,
        )
        .as_bytes(),
    );
    body.extend_from_slice(file.bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let app = make_app(state, storage);
    let mut builder = Request::builder().method("POST").uri(uri).header(
        header::CONTENT_TYPE,
        format!("multipart/form-data; boundary={boundary}"),
    );
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder
        .body(Body::from(body))
        .expect("failed to build request");
    let response = app.oneshot(request).await.expect("router oneshot failed");
    let status = response.status();
    (status, body_string(response).await)
}

/// GET a static asset and return `(status, Content-Type)`. Pins the Sqlite backend
/// — static-asset serving never touches storage, so it need not run on both.
pub async fn get_asset(uri: &str) -> (StatusCode, Option<String>) {
    let TestEnv { state, base } = Backend::Sqlite.setup().await;

    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();

    let app = jaunder::create_router(
        state,
        base.instance_id().clone(),
        noop_mailer(),
        false,
        tmp_storage_path(),
    )
    .expect("canonical instance identity is an HTTP header");
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string());

    (status, content_type)
}
