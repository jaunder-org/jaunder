---
root: false
targets:
  - '*'
---
- Use rust, except e2e tests using playwright and typescript.
- Web framework is leptos using SSR via cargo-leptos.
- The production deployment will be behind a reverse proxy providing https.
- API methods are automatically prefixed with `/api`
- **Server Functions:** Define `#[server]` functions in the relevant `web/src/*.rs` module (e.g., `web/src/auth.rs`). Use `#[cfg(feature = "ssr")]` for server-only imports and logic within these files.
- **Authentication:** Use `require_auth().await?` at the start of any server function that requires a logged-in user. It returns an `AuthUser` struct containing `user_id`, `username`, and `token_hash`.
- **Session Management:** Use `set_session_cookie(raw_token)` and `clear_session_cookie()` (defined in `web/src/auth.rs`) to manage the `session` cookie in server functions.
