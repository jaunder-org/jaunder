# M3 Step 10: Email Verification Web UI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the email verification flow — server functions, Leptos components, routes, integration tests, e2e tests, and the `FileMailSender` needed by e2e.

**Architecture:** Add `FileMailSender` to `server/src/mailer.rs` (selected via `JAUNDER_MAIL_CAPTURE_FILE` env var at startup); extend `ProfileData` with email fields; add `web/src/email.rs` with two server functions and two Leptos components; wire routes into `App`; cover with 4 integration tests (using `CapturingMailSender`) and 2 e2e tests (using the file capture).

**Tech Stack:** Rust, Leptos 0.8.2, leptos_axum, sqlx/SQLite, lettre, serde_json, Playwright/TypeScript, NixOS VM tests.

---

## File Map

| File | Change |
|---|---|
| `Cargo.toml` | Add `serde_json` workspace dep |
| `server/Cargo.toml` | Add `serde_json` dep |
| `server/src/mailer.rs` | Add `FileMailSender` |
| `server/src/storage/mod.rs` | Update `build_mailer` to check `JAUNDER_MAIL_CAPTURE_FILE` |
| `web/Cargo.toml` | Add `email_address` optional dep under `ssr` |
| `web/src/profile.rs` | Add `email` and `email_verified` to `ProfileData`; update `get_profile` |
| `web/src/email.rs` | New: `request_email_verification`, `verify_email`, `EmailPage`, `VerifyEmailPage` |
| `web/src/lib.rs` | Add `pub mod email;`, import components, add two routes |
| `server/tests/web_email.rs` | New: 4 integration tests |
| `end2end/tests/email.spec.ts` | New: 2 e2e tests |
| `flake.nix` | Add `JAUNDER_MAIL_CAPTURE_FILE` to systemd env and playwright command |
| `docs/milestones/M3.md` | Check off Step 10 items |

---

## Task 1: Add serde_json dependency

**Files:**
- Modify: `Cargo.toml`
- Modify: `server/Cargo.toml`

- [ ] **Step 1: Add serde_json to workspace deps**

In `Cargo.toml`, add to `[workspace.dependencies]` (after the `serde` line):
```toml
serde_json = "1"
```

- [ ] **Step 2: Add serde_json to server deps**

In `server/Cargo.toml`, add to `[dependencies]` (after `thiserror`):
```toml
serde_json.workspace = true
```

- [ ] **Step 3: Verify build**

```bash
cargo build -p server
```
Expected: compiles without errors.

---

## Task 2: FileMailSender

**Files:**
- Modify: `server/src/mailer.rs`

- [ ] **Step 1: Add FileMailSender to server/src/mailer.rs**

After the closing `}` of the `LettreMailSender` `impl MailSender` block (before the `#[cfg(test)]` block), add:

```rust
// ---------------------------------------------------------------------------
// FileMailSender
// ---------------------------------------------------------------------------

/// A [`MailSender`] that appends each outgoing message as a JSON line to a
/// file on disk.  Used when `JAUNDER_MAIL_CAPTURE_FILE` is set in the
/// environment (typically for end-to-end tests).
pub struct FileMailSender {
    path: std::path::PathBuf,
}

impl FileMailSender {
    /// Create a new `FileMailSender` that writes to `path`.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl MailSender for FileMailSender {
    async fn send_email(&self, message: &EmailMessage) -> Result<(), MailError> {
        use std::io::Write;

        let to: Vec<&str> = message.to.iter().map(|a| a.as_str()).collect();
        let record = serde_json::json!({
            "to": to,
            "from": message.from.as_ref().map(|a| a.as_str()),
            "subject": message.subject,
            "body_text": message.body_text,
        });
        let mut line = serde_json::to_string(&record)
            .map_err(|e| MailError::Send(e.to_string()))?;
        line.push('\n');

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| MailError::Send(e.to_string()))?;
        file.write_all(line.as_bytes())
            .map_err(|e| MailError::Send(e.to_string()))?;

        Ok(())
    }
}
```

- [ ] **Step 2: Add unit test for FileMailSender**

In the `#[cfg(test)] mod tests` block in `server/src/mailer.rs`, add after the existing tests:

```rust
    #[tokio::test]
    async fn file_mail_sender_appends_json_line() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("mail.jsonl");
        let sender = FileMailSender::new(&path);

        let msg = EmailMessage {
            from: None,
            to: vec!["bob@example.com".parse::<email_address::EmailAddress>().unwrap()],
            subject: "Hello".to_string(),
            body_text: "World".to_string(),
        };
        sender.send_email(&msg).await.expect("send");

        let content = std::fs::read_to_string(&path).expect("read");
        let record: serde_json::Value = serde_json::from_str(content.trim()).expect("parse");
        assert_eq!(record["subject"], "Hello");
        assert_eq!(record["body_text"], "World");
        assert_eq!(record["to"][0], "bob@example.com");
        assert!(record["from"].is_null());
    }

    #[tokio::test]
    async fn file_mail_sender_appends_multiple_lines() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("mail.jsonl");
        let sender = FileMailSender::new(&path);

        for i in 0..3u8 {
            let msg = EmailMessage {
                from: None,
                to: vec!["x@example.com".parse::<email_address::EmailAddress>().unwrap()],
                subject: format!("msg{i}"),
                body_text: String::new(),
            };
            sender.send_email(&msg).await.expect("send");
        }

        let content = std::fs::read_to_string(&path).expect("read");
        assert_eq!(content.lines().count(), 3);
    }
```

Note: `tempfile` is already in `server/Cargo.toml` dev-dependencies, but the unit test is in the library (not a dev context). Add `tempfile` to `[dependencies]` for the test build — actually, since `tempfile` is only used in `#[cfg(test)]`, we need to add it to `[dev-dependencies]` in `server/Cargo.toml`, which it already is. The `#[cfg(test)]` block can access dev-deps.

Wait — `tempfile` IS already in `[dev-dependencies]`. Unit tests inside library code (`src/`) use dev-dependencies. Good, no change needed.

- [ ] **Step 3: Run the new tests**

```bash
cargo nextest run -E 'test(file_mail_sender)'
```
Expected: 2 tests pass.

---

## Task 3: Update build_mailer to check JAUNDER_MAIL_CAPTURE_FILE

**Files:**
- Modify: `server/src/storage/mod.rs`

- [ ] **Step 1: Add FileMailSender import**

At the top of `server/src/storage/mod.rs`, update the mailer import line (currently `use common::mailer::{MailSender, NoopMailSender};`) to:

```rust
use common::mailer::{MailSender, NoopMailSender};
use crate::mailer::FileMailSender;
```

- [ ] **Step 2: Update build_mailer**

Replace the current `build_mailer` function:

```rust
async fn build_mailer(site_config: &SqliteSiteConfigStorage) -> Arc<dyn MailSender> {
    match load_smtp_config(site_config).await {
        Ok(Some(cfg)) => match crate::mailer::LettreMailSender::from_config(&cfg) {
            Ok(sender) => Arc::new(sender),
            Err(_) => Arc::new(NoopMailSender),
        },
        Ok(None) | Err(_) => Arc::new(NoopMailSender),
    }
}
```

with:

```rust
async fn build_mailer(site_config: &SqliteSiteConfigStorage) -> Arc<dyn MailSender> {
    if let Ok(path) = std::env::var("JAUNDER_MAIL_CAPTURE_FILE") {
        return Arc::new(FileMailSender::new(path));
    }
    match load_smtp_config(site_config).await {
        Ok(Some(cfg)) => match crate::mailer::LettreMailSender::from_config(&cfg) {
            Ok(sender) => Arc::new(sender),
            Err(_) => Arc::new(NoopMailSender),
        },
        Ok(None) | Err(_) => Arc::new(NoopMailSender),
    }
}
```

- [ ] **Step 3: Verify build and tests still pass**

```bash
cargo build -p server
cargo nextest run
```
Expected: all tests pass.

---

## Task 4: Extend ProfileData with email fields

**Files:**
- Modify: `web/src/profile.rs`

- [ ] **Step 1: Add email fields to ProfileData**

In `web/src/profile.rs`, replace the `ProfileData` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileData {
    pub username: String,
    pub display_name: Option<String>,
    pub bio: Option<String>,
}
```

with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileData {
    pub username: String,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub email: Option<String>,
    pub email_verified: bool,
}
```

- [ ] **Step 2: Update get_profile to populate email fields**

In `web/src/profile.rs`, replace the `Ok(ProfileData { ... })` in `get_profile`:

```rust
    Ok(ProfileData {
        username: user.username.to_string(),
        display_name: user.display_name,
        bio: user.bio,
    })
```

with:

```rust
    Ok(ProfileData {
        username: user.username.to_string(),
        display_name: user.display_name,
        bio: user.bio,
        email: user.email.map(|e| e.to_string()),
        email_verified: user.email_verified,
    })
```

- [ ] **Step 3: Verify build and tests**

```bash
cargo build
cargo nextest run
```
Expected: compiles and all tests pass. (No tests check email fields in ProfileData yet.)

---

## Task 5: Create web/src/email.rs — server functions

**Files:**
- Modify: `web/Cargo.toml`
- Create: `web/src/email.rs`
- Modify: `web/src/lib.rs`

- [ ] **Step 1: Add email_address optional dep to web**

In `web/Cargo.toml`, add to `[dependencies]` (after the `thiserror` line):

```toml
email_address = { workspace = true, optional = true }
```

And add it to the `ssr` feature list:

```toml
ssr = [
    "leptos/ssr",
    "leptos_meta/ssr",
    "leptos_router/ssr",
    "dep:leptos_axum",
    "dep:axum",
    "dep:chrono",
    "dep:common",
    "dep:email_address",
]
```

- [ ] **Step 2: Create web/src/email.rs with SSR imports and server functions**

Create `web/src/email.rs` with the following content:

```rust
#[cfg(feature = "ssr")]
use crate::auth::require_auth;
#[cfg(feature = "ssr")]
use common::mailer::EmailMessage;
#[cfg(feature = "ssr")]
use common::storage::AppState;
#[cfg(feature = "ssr")]
use std::sync::Arc;

use leptos::prelude::*;

/// Sends a verification email to `email`. Requires authentication.
///
/// Creates a 24-hour verification token, sends a link to `/verify-email?token=…`
/// via the configured mailer.
#[server(endpoint = "/request_email_verification")]
pub async fn request_email_verification(email: String) -> Result<(), ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();

    let email_addr = email
        .parse::<email_address::EmailAddress>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(auth.user_id, &email, expires_at)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let link = format!("/verify-email?token={raw_token}");
    let message = EmailMessage {
        from: None,
        to: vec![email_addr],
        subject: "Verify your email address".to_string(),
        body_text: format!(
            "Click the link below to verify your email address:\n\n{link}\n\nThis link expires in 24 hours."
        ),
    };

    state
        .mailer
        .send_email(&message)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Consumes a verification token and marks the associated email as verified
/// on the user account.
#[server(endpoint = "/verify_email")]
pub async fn verify_email(token: String) -> Result<(), ServerFnError> {
    let state = expect_context::<Arc<AppState>>();

    let (user_id, email_str) = state
        .email_verifications
        .use_email_verification(&token)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let email_addr = email_str
        .parse::<email_address::EmailAddress>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    state
        .users
        .set_email(user_id, Some(&email_addr), true)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
```

- [ ] **Step 3: Declare the module in web/src/lib.rs**

In `web/src/lib.rs`, add after the existing `pub mod` lines (e.g., after `pub mod sessions;`):

```rust
pub mod email;
```

- [ ] **Step 4: Verify build**

```bash
cargo build
```
Expected: compiles without errors or warnings.

---

## Task 6: Add components and routes

**Files:**
- Modify: `web/src/email.rs`
- Modify: `web/src/lib.rs`

- [ ] **Step 1: Add EmailPage and VerifyEmailPage to web/src/email.rs**

Append to the end of `web/src/email.rs`:

```rust
use crate::profile::get_profile;

/// Email settings page — shows current email and verification status;
/// form to submit a new email address for verification.
#[component]
pub fn EmailPage() -> impl IntoView {
    let request_action = ServerAction::<RequestEmailVerification>::new();
    let profile = Resource::new(move || request_action.version().get(), |_| get_profile());

    view! {
        <h1>"Email Settings"</h1>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || Suspend::new(async move {
                match profile.await {
                    Ok(data) => {
                        let email_status = match (data.email.clone(), data.email_verified) {
                            (Some(ref e), true) => format!("{e} (verified)"),
                            (Some(ref e), false) => format!("{e} (unverified)"),
                            (None, _) => "No email set".to_string(),
                        };
                        view! {
                            <p>"Current email: " {email_status}</p>
                            <ActionForm action=request_action>
                                <label>
                                    "New email address"
                                    <input type="email" name="email" />
                                </label>
                                <button type="submit">"Send verification link"</button>
                            </ActionForm>
                        }
                        .into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            request_action
                .value()
                .get()
                .map(|r: Result<(), ServerFnError>| match r {
                    Ok(()) => {
                        view! { <p>"Check your email for a verification link."</p> }.into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                })
        }}
    }
}

/// Reads the `token` query parameter and calls `verify_email` on mount.
/// Renders a success message or an appropriate error.
#[component]
pub fn VerifyEmailPage() -> impl IntoView {
    use leptos_router::hooks::use_query_map;

    let query = use_query_map();
    let token = move || query.with(|q| q.get("token").cloned().unwrap_or_default());
    let result = Resource::new(token, |t| verify_email(t));

    view! {
        <h1>"Verify Email"</h1>
        <Suspense fallback=|| view! { <p>"Verifying..."</p> }>
            {move || Suspend::new(async move {
                match result.await {
                    Ok(()) => {
                        view! { <p>"Your email address has been verified."</p> }.into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}
```

- [ ] **Step 2: Add imports and routes in web/src/lib.rs**

In `web/src/lib.rs`, add the import for the new components. After the existing `use crate::...` imports, add:

```rust
use crate::email::{EmailPage, VerifyEmailPage};
```

Then add two routes inside the `<Routes>` block, after the `invites` route:

```rust
<Route path=(StaticSegment("profile"), StaticSegment("email")) view=EmailPage />
<Route path=StaticSegment("verify-email") view=VerifyEmailPage />
```

- [ ] **Step 3: Verify build**

```bash
cargo build
```
Expected: compiles. If `use_query_map` is not importable from `leptos_router::hooks`, check with `cargo doc -p leptos_router --open` to find the correct path.

- [ ] **Step 4: Run all tests**

```bash
cargo nextest run
```
Expected: all existing tests pass.

---

## Task 7: Integration tests

**Files:**
- Create: `server/tests/web_email.rs`

- [ ] **Step 1: Create server/tests/web_email.rs**

Create `server/tests/web_email.rs` with the following content:

```rust
use std::sync::{Arc, OnceLock};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::Utc;
use common::mailer::{test_utils::CapturingMailSender, MailSender};
use leptos::prelude::LeptosOptions;
use server::storage::{
    AppState, SqliteAtomicOps, SqliteEmailVerificationStorage, SqliteInviteStorage,
    SqlitePasswordResetStorage, SqliteSessionStorage, SqliteSiteConfigStorage,
    SqliteUserStorage,
};
use server::username::Username;
use sqlx::SqlitePool;
use tempfile::TempDir;
use tower::ServiceExt;

fn ensure_server_fns_registered() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        server_fn::axum::register_explicit::<web::email::RequestEmailVerification>();
        server_fn::axum::register_explicit::<web::email::VerifyEmail>();
    });
}

async fn open_pool(base: &TempDir) -> SqlitePool {
    let opts: sqlx::sqlite::SqliteConnectOptions =
        format!("sqlite:{}", base.path().join("test.db").display())
            .parse()
            .unwrap();
    let pool = SqlitePool::connect_with(opts.create_if_missing(true))
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

async fn test_state_with_mailer(
    base: &TempDir,
) -> (Arc<AppState>, Arc<CapturingMailSender>) {
    let pool = open_pool(base).await;
    let mailer = Arc::new(CapturingMailSender::new());
    let state = Arc::new(AppState {
        site_config: Arc::new(SqliteSiteConfigStorage::new(pool.clone())),
        users: Arc::new(SqliteUserStorage::new(pool.clone())),
        sessions: Arc::new(SqliteSessionStorage::new(pool.clone())),
        invites: Arc::new(SqliteInviteStorage::new(pool.clone())),
        atomic: Arc::new(SqliteAtomicOps::new(pool.clone())),
        email_verifications: Arc::new(SqliteEmailVerificationStorage::new(pool.clone())),
        password_resets: Arc::new(SqlitePasswordResetStorage::new(pool)),
        mailer: Arc::clone(&mailer) as Arc<dyn MailSender>,
    });
    (state, mailer)
}

fn test_options() -> LeptosOptions {
    LeptosOptions::builder().output_name("test").build()
}

async fn post_form(
    state: Arc<AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder.body(Body::from(body.into())).unwrap();

    let app = server::create_router(test_options(), state);
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

// M3.10.7: request_email_verification creates a row and sends an email via CapturingMailSender.
#[tokio::test]
async fn request_email_verification_creates_row_and_sends_email() {
    let base = TempDir::new().unwrap();
    let (state, mailer) = test_state_with_mailer(&base).await;

    let user_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let raw_token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={raw_token}");

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/request_email_verification",
        "email=alice%40example.com",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let sent = mailer.sent();
    assert_eq!(sent.len(), 1, "expected one email to be sent");
    assert_eq!(sent[0].to.len(), 1);
    assert_eq!(sent[0].to[0].as_str(), "alice@example.com");
    assert!(
        sent[0].body_text.contains("/verify-email?token="),
        "email body should contain verification link, got: {}",
        sent[0].body_text
    );
}

// M3.10.8: verify_email with a valid token sets the email as verified.
#[tokio::test]
async fn verify_email_with_valid_token_sets_email_verified() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let user_id = state
        .users
        .create_user(
            &"bob".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, "bob@example.com", expires_at)
        .await
        .unwrap();

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/verify_email",
        format!("token={raw_token}"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(
        user.email.as_ref().map(|e| e.as_str()),
        Some("bob@example.com")
    );
    assert!(user.email_verified, "email should be marked as verified");
}

// M3.10.9: verify_email with an expired token returns an error.
#[tokio::test]
async fn verify_email_with_expired_token_returns_error() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let user_id = state
        .users
        .create_user(
            &"carol".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, "carol@example.com", expires_at)
        .await
        .unwrap();

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/verify_email",
        format!("token={raw_token}"),
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.10.10: verify_email with an unknown token returns an error.
#[tokio::test]
async fn verify_email_with_unknown_token_returns_error() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/verify_email",
        "token=this_token_does_not_exist",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}
```

- [ ] **Step 2: Run integration tests**

```bash
cargo nextest run -E 'test(verify_email) | test(request_email_verification)'
```
Expected: all 4 tests pass.

- [ ] **Step 3: Run all tests**

```bash
cargo nextest run
```
Expected: all tests pass.

- [ ] **Step 4: Check clippy**

```bash
cargo clippy -- -D warnings
```
Expected: clean.

---

## Task 8: E2E tests

**Files:**
- Create: `end2end/tests/email.spec.ts`

- [ ] **Step 1: Create end2end/tests/email.spec.ts**

Create `end2end/tests/email.spec.ts`:

```typescript
import { test, expect, type Page } from "@playwright/test";
import * as fs from "fs";

const MAIL_CAPTURE_FILE =
  process.env.JAUNDER_MAIL_CAPTURE_FILE ?? "/tmp/jaunder-mail.jsonl";

interface CapturedEmail {
  to: string[];
  from: string | null;
  subject: string;
  body_text: string;
}

function readLatestEmail(): CapturedEmail | null {
  if (!fs.existsSync(MAIL_CAPTURE_FILE)) return null;
  const content = fs.readFileSync(MAIL_CAPTURE_FILE, "utf-8");
  const lines = content
    .trim()
    .split("\n")
    .filter((l) => l.trim());
  if (lines.length === 0) return null;
  return JSON.parse(lines[lines.length - 1]) as CapturedEmail;
}

async function waitForHydration(page: Page): Promise<void> {
  await page.waitForSelector("body[data-hydrated]");
}

// M3.10.11: Full email verification flow.
test("email verification flow completes successfully", async ({ page }) => {
  // Log in
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  // Navigate to email settings and submit an address
  await page.goto("http://localhost:3000/profile/email");
  await waitForHydration(page);
  await page.fill('input[name="email"]', "testlogin@example.com");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  await expect(
    page.locator('p:has-text("Check your email")'),
  ).toBeVisible();

  // Extract the verification token from the captured mail file
  const email = readLatestEmail();
  expect(email).not.toBeNull();
  const tokenMatch = email!.body_text.match(/token=([^\s]+)/);
  expect(tokenMatch).not.toBeNull();
  const token = tokenMatch![1];

  // Visit the verification link
  await page.goto(`http://localhost:3000/verify-email?token=${token}`);
  await page.waitForLoadState("networkidle");
  await expect(page.locator('p:has-text("verified")')).toBeVisible();

  // Confirm email is shown as verified on the profile page
  await page.goto("http://localhost:3000/profile/email");
  await page.waitForLoadState("networkidle");
  await expect(page.locator("p")).toContainText("verified");
});

// M3.10.12: Invalid token shows an error.
test("visiting verify-email with invalid token shows error", async ({
  page,
}) => {
  await page.goto(
    "http://localhost:3000/verify-email?token=totally_invalid_token",
  );
  await page.waitForLoadState("networkidle");
  await expect(page.locator(".error")).toBeVisible();
});
```

- [ ] **Step 2: Format the new file**

```bash
scripts/format end2end/tests/email.spec.ts
```
Expected: file formatted without errors.

---

## Task 9: Update nix flake for email capture

**Files:**
- Modify: `flake.nix`

- [ ] **Step 1: Add JAUNDER_MAIL_CAPTURE_FILE to systemd environment**

In `flake.nix`, in the `systemd.services.jaunder.environment` attribute set, add after `RUST_LOG = "info";`:

```nix
JAUNDER_MAIL_CAPTURE_FILE = "/tmp/jaunder-mail.jsonl";
```

- [ ] **Step 2: Pass JAUNDER_MAIL_CAPTURE_FILE to the playwright command**

In `flake.nix`, update the `machine.succeed(...)` call that runs playwright. The current last line of the playwright command is:

```
+ " --config playwright.nix.config.js"
```

Change it to:

```
+ " --config playwright.nix.config.js"
+ " JAUNDER_MAIL_CAPTURE_FILE=/tmp/jaunder-mail.jsonl"
```

The current playwright command in `flake.nix` is:
```nix
machine.succeed(
  "cd /tmp/e2e"
  + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
  + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
  + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
  + " --config playwright.nix.config.js"
)
```

Add `JAUNDER_MAIL_CAPTURE_FILE=/tmp/jaunder-mail.jsonl` alongside the other env vars:

```nix
machine.succeed(
  "cd /tmp/e2e"
  + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
  + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
  + " JAUNDER_MAIL_CAPTURE_FILE=/tmp/jaunder-mail.jsonl"
  + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
  + " --config playwright.nix.config.js"
)
```

- [ ] **Step 3: Verify flake check locally (optional — takes a while)**

```bash
nix flake check
```
Expected: all checks pass including the new e2e tests.

---

## Task 10: Final verification and check off milestone items

- [ ] **Step 1: Full build**

```bash
cargo build
```
Expected: clean.

- [ ] **Step 2: Full test suite**

```bash
cargo nextest run
```
Expected: all tests pass.

- [ ] **Step 3: Clippy**

```bash
cargo clippy -- -D warnings
```
Expected: clean.

- [ ] **Step 4: Coverage baseline**

```bash
scripts/check-coverage
```
Expected: passes.

- [ ] **Step 5: Check off milestone items**

In `docs/milestones/M3.md`, mark all Step 10 items as done:

```markdown
### Step 10: Web UI — email verification flow

1. [x] Create `web/src/email.rs`; declare as `pub mod email;` in `web/src/lib.rs`.
2. [x] Implement `request_email_verification(email: String) -> Result<(), ServerFnError>` ...
3. [x] Implement `verify_email(token: String) -> Result<(), ServerFnError>` ...
4. [x] Define `EmailPage` component ...
5. [x] Define `VerifyEmailPage` component ...
6. [x] Add `/profile/email` and `/verify-email` routes to `App` in `web/src/lib.rs`.
7. [x] Integration test: `request_email_verification` creates a verification row ...
8. [x] Integration test: `verify_email` with a valid token sets the email as verified ...
9. [x] Integration test: `verify_email` with an expired token returns an error.
10. [x] Integration test: `verify_email` with an unknown token returns an error.
11. [x] E2E test: user adds an email address ...
12. [x] E2E test: visiting `/verify-email` with an invalid token shows an error.
```

- [ ] **Step 6: Request user review**

Present diff to user and wait for review confirmation before committing.

- [ ] **Step 7: Commit (only after user approves review)**

```bash
git add \
  Cargo.toml \
  server/Cargo.toml \
  server/src/mailer.rs \
  server/src/storage/mod.rs \
  server/tests/web_email.rs \
  web/Cargo.toml \
  web/src/email.rs \
  web/src/lib.rs \
  web/src/profile.rs \
  end2end/tests/email.spec.ts \
  flake.nix \
  docs/milestones/M3.md

git commit -m "$(cat <<'EOF'
M3.10.1-12: Add email verification web UI, FileMailSender, and e2e support

- FileMailSender writes JSON lines to JAUNDER_MAIL_CAPTURE_FILE for e2e
- ProfileData gains email/email_verified fields
- request_email_verification and verify_email server functions
- EmailPage and VerifyEmailPage components at /profile/email and /verify-email
- 4 integration tests via CapturingMailSender
- 2 e2e tests via file capture
- Nix e2e harness sets JAUNDER_MAIL_CAPTURE_FILE

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```
