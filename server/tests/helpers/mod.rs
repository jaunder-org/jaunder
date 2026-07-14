// Shared test helpers for the server integration suite. Compiled once as the single
// `mod helpers;` of the `integration` test binary, so every item is reachable from
// some subsystem module and no dead-code/unused suppression is needed.
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::mailer::MailSender;
use leptos::prelude::LeptosOptions;
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;
use tower::ServiceExt;

// The both-backend test harness — `Backend`, `TestEnv`, per-test DB provisioning,
// and the `backends`/`sqlite_only`/`postgres_only` rstest templates — lives in
// `storage::test_support` (gated by storage's `test-support` feature; ADR-0033) so
// `storage`'s own tests can use it from the same crate instance. Test files import
// what they need directly from `storage::test_support`; `helpers`' own bodies pull
// in only `noop_mailer`.
use storage::test_support::noop_mailer;

mod websub_capturing;
// The capturing WebSub client used by `feed_worker.rs`.
pub use websub_capturing::CapturingWebSubClient;

pub fn ensure_server_fns_registered() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        server_fn::axum::register_explicit::<web::auth::CurrentUser>();
        server_fn::axum::register_explicit::<web::backup::BackupWarningVisible>();
        server_fn::axum::register_explicit::<web::backup::CurrentUserIsOperator>();
        server_fn::axum::register_explicit::<web::backup::GetBackupSettings>();
        server_fn::axum::register_explicit::<web::backup::UpdateBackupSettings>();
        server_fn::axum::register_explicit::<web::auth::GetRegistrationPolicy>();
        server_fn::axum::register_explicit::<web::auth::Register>();
        server_fn::axum::register_explicit::<web::auth::Login>();
        server_fn::axum::register_explicit::<web::auth::Logout>();
        server_fn::axum::register_explicit::<web::email::RequestEmailVerification>();
        server_fn::axum::register_explicit::<web::email::VerifyEmail>();
        server_fn::axum::register_explicit::<web::profile::GetProfile>();
        server_fn::axum::register_explicit::<web::profile::UpdateProfile>();
        server_fn::axum::register_explicit::<web::profile::GetDefaultPostFormat>();
        server_fn::axum::register_explicit::<web::profile::SetDefaultPostFormat>();
        server_fn::axum::register_explicit::<web::sessions::ListSessions>();
        server_fn::axum::register_explicit::<web::sessions::RevokeSession>();
        server_fn::axum::register_explicit::<web::sessions::CreateAppPassword>();
        server_fn::axum::register_explicit::<web::invites::CreateInvite>();
        server_fn::axum::register_explicit::<web::invites::ListInvites>();
        server_fn::axum::register_explicit::<web::password_reset::RequestPasswordReset>();
        server_fn::axum::register_explicit::<web::password_reset::ConfirmPasswordReset>();
        server_fn::axum::register_explicit::<web::posts::CreatePost>();
        server_fn::axum::register_explicit::<web::posts::GetPost>();
        server_fn::axum::register_explicit::<web::posts::GetPostPreview>();
        server_fn::axum::register_explicit::<web::posts::UpdatePost>();
        server_fn::axum::register_explicit::<web::posts::ListDrafts>();
        server_fn::axum::register_explicit::<web::posts::PublishPost>();
        server_fn::axum::register_explicit::<web::posts::ListUserPosts>();
        server_fn::axum::register_explicit::<web::posts::ListLocalTimeline>();
        server_fn::axum::register_explicit::<web::posts::ListHomeFeed>();
        server_fn::axum::register_explicit::<web::posts::ListPostsByTag>();
        server_fn::axum::register_explicit::<web::posts::ListUserPostsByTag>();
        server_fn::axum::register_explicit::<web::posts::DeletePost>();
        server_fn::axum::register_explicit::<web::posts::UnpublishPost>();
        server_fn::axum::register_explicit::<web::posts::DefaultAudienceSelection>();
        server_fn::axum::register_explicit::<web::posts::PostAudienceSelection>();
        server_fn::axum::register_explicit::<web::site::GetSiteIdentity>();
        server_fn::axum::register_explicit::<web::site::UpdateSiteIdentity>();
        server_fn::axum::register_explicit::<web::media::ListMyMedia>();
        server_fn::axum::register_explicit::<web::media::MediaUsage>();
        server_fn::axum::register_explicit::<web::media::DeleteMedia>();
        server_fn::axum::register_explicit::<web::tags::ListTags>();
        server_fn::axum::register_explicit::<web::subscriptions::SubscribeTo>();
        server_fn::axum::register_explicit::<web::subscriptions::UnsubscribeFrom>();
        server_fn::axum::register_explicit::<web::subscriptions::IsSubscribedTo>();
        server_fn::axum::register_explicit::<web::audiences::CreateAudience>();
        server_fn::axum::register_explicit::<web::audiences::RenameAudience>();
        server_fn::axum::register_explicit::<web::audiences::DeleteAudience>();
        server_fn::axum::register_explicit::<web::audiences::ListMyAudiences>();
        server_fn::axum::register_explicit::<web::audiences::ListMySubscribers>();
        server_fn::axum::register_explicit::<web::audiences::AddSubscriberToAudience>();
        server_fn::axum::register_explicit::<web::audiences::RemoveSubscriberFromAudience>();
        server_fn::axum::register_explicit::<web::audiences::ListAudienceMembers>();
    });
}

pub fn test_options() -> LeptosOptions {
    LeptosOptions::builder().output_name("test").build()
}

/// Returns a `PathBuf` pointing to a temporary directory usable as a storage
/// root.  The caller is responsible for keeping the `TempDir` alive; this
/// function returns the inner path for convenience when lifetime management is
/// not needed (e.g. when storage is never actually written to in the test).
pub fn tmp_storage_path() -> std::path::PathBuf {
    // Return the system temp dir — the media subdirectories are created on
    // demand by the handlers, so the root just needs to exist.
    std::env::temp_dir().join("jaunder-test-storage")
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
pub fn make_app(state: Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    std::fs::create_dir_all(storage_path.join("media").join("upload")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("cached")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("tmp")).unwrap();
    jaunder::create_router(test_options(), state, noop_mailer(), false, storage_path)
}

/// How a `post_form` request authenticates. Cookie and bearer are mutually
/// exclusive — no caller sends both — so they are one argument, not two.
enum Auth<'a> {
    None,
    Cookie(&'a str),
    Bearer(&'a str),
}

/// A POST body paired with its content type — the two always travel together, so
/// they are one argument. `Form` is `application/x-www-form-urlencoded`, `Json` is
/// `application/json`.
enum PostBody {
    Form(String),
    Json(String),
}

impl PostBody {
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

/// The single implementation behind every `post_form*`/`post_json` helper: build
/// a fresh router from `state` (with `mailer` and `secure_cookies`), send one POST
/// with the given `body` (and its content type), and return `(status, Set-Cookie,
/// body)`. The public wrappers below fix the arguments most callers don't vary.
async fn post_inner(
    state: Arc<storage::AppState>,
    mailer: Arc<dyn MailSender>,
    uri: &str,
    body: PostBody,
    auth: Auth<'_>,
    user_agent: Option<&str>,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String) {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, body.content_type());
    match auth {
        Auth::None => {}
        Auth::Cookie(c) => builder = builder.header(header::COOKIE, c),
        Auth::Bearer(token) => {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
    }
    if let Some(ua) = user_agent {
        builder = builder.header(header::USER_AGENT, ua);
    }
    let request = builder
        .body(Body::from(body.into_string()))
        .expect("failed to build request");

    let app = jaunder::create_router(
        test_options(),
        state,
        mailer,
        secure_cookies,
        tmp_storage_path(),
    );
    let response = app.oneshot(request).await.expect("router oneshot failed");

    let status = response.status();
    let set_cookie = response.headers().get(header::SET_COOKIE).map(|v| {
        v.to_str()
            .expect("Set-Cookie header is not valid UTF-8")
            .to_string()
    });
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = String::from_utf8(bytes.to_vec()).expect("response body is not valid UTF-8");

    (status, set_cookie, body_str)
}

/// Canonical case: noop mailer, secure cookies, cookie auth, `Set-Cookie` dropped.
pub async fn post_form(
    state: Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let auth = cookie.map_or(Auth::None, Auth::Cookie);
    let (status, _set_cookie, body) = post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Form(body.into()),
        auth,
        None,
        true,
    )
    .await;
    (status, body)
}

/// Like [`post_form`], but injects a specific `mailer` (e.g. a capturing sender)
/// instead of the noop.
pub async fn post_form_with_mailer(
    state: Arc<storage::AppState>,
    mailer: Arc<dyn MailSender>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let auth = cookie.map_or(Auth::None, Auth::Cookie);
    let (status, _set_cookie, body) = post_inner(
        state,
        mailer,
        uri,
        PostBody::Form(body.into()),
        auth,
        None,
        true,
    )
    .await;
    (status, body)
}

/// Exposes the `secure_cookies` toggle and returns the `Set-Cookie` value —
/// what the auth/session tests need over the canonical [`post_form`].
pub async fn post_form_with_secure_flag(
    state: Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String) {
    let auth = cookie.map_or(Auth::None, Auth::Cookie);
    post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Form(body.into()),
        auth,
        None,
        secure_cookies,
    )
    .await
}

/// Like [`post_form_with_secure_flag`], but also sets a `User-Agent` header.
pub async fn post_form_with_ua(
    state: Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
    user_agent: &str,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String) {
    let auth = cookie.map_or(Auth::None, Auth::Cookie);
    post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Form(body.into()),
        auth,
        Some(user_agent),
        secure_cookies,
    )
    .await
}

/// Authenticates with an `Authorization: Bearer <token>` header instead of a
/// cookie. Returns the `Set-Cookie` value like the other auth helpers.
pub async fn post_form_with_bearer(
    state: Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    bearer: &str,
) -> (StatusCode, Option<String>, String) {
    post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Form(body.into()),
        Auth::Bearer(bearer),
        None,
        true,
    )
    .await
}

/// POST a JSON body (`Content-Type: application/json`) with secure cookies and
/// optional cookie auth; returns `(status, body)` — drops `Set-Cookie`, like the
/// canonical [`post_form`].
pub async fn post_json(
    state: Arc<storage::AppState>,
    uri: &str,
    body: serde_json::Value,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let auth = cookie.map_or(Auth::None, Auth::Cookie);
    let (status, _set_cookie, body) = post_inner(
        state,
        noop_mailer(),
        uri,
        PostBody::Json(body.to_string()),
        auth,
        None,
        true,
    )
    .await;
    (status, body)
}
