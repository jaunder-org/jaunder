# Clippy Pedantic Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate all 173 warnings produced by the stricter `clippy::pedantic` configuration across the entire workspace.

**Architecture:** Warnings are grouped by fix type: mechanical auto-fixable changes, documentation additions, code structure improvements, and allow-annotations for intentional patterns. Tasks are ordered to minimize conflicts — mechanical single-line fixes first, then structural refactors, then documentation.

**Tech Stack:** Rust, `cargo clippy`, `cargo nextest`

---

## Warning Categories and Counts

| Category | Count | Files |
|---|---|---|
| `must_use` | 56 | common, web, server |
| `missing_errors_doc` | 27 | common, web, server |
| `cast_lossless` (u32→i64) | 20 | server/storage/sqlite.rs, postgres.rs |
| `ignored_unit_patterns` | 10 | common, web |
| `doc_markdown` | 7 | common, server |
| `uninlined_format_args` | 7 | web, server |
| `map_unwrap_or` / `map_unwrap_or_else` | 8 | web, server |
| `needless_pass_by_value` | 5 | web, server |
| `manual_let_else` | 5 | server/storage |
| `redundant_closure_for_method_calls` | 5 | web, server |
| `used_underscore_binding` | 4 | web/src/pages/home.rs |
| `single_match_else` | 3 | server |
| `too_many_lines` | 5 | web/src/pages/ (Leptos components) |
| `match_same_arms` | 2 | server/src/storage/backup/ |
| `cast_possible_truncation` / `cast_sign_loss` / `cast_precision_loss` | 4 | web, server |
| `missing_panics_doc` | 2 | server/src/main.rs |
| `unnested_or_patterns` | 1 | web/src/auth.rs |
| `items_after_statements` | 1 | web/src/pages/ui.rs |
| `wildcard_imports` | 1 | hydrate/src/lib.rs |
| `match_wildcard_for_single_variants` | 1 | server/src/commands.rs |
| `manual_string_new` | 1 | web/src/pages/posts.rs |

---

## Task 1: `common` crate — `#[must_use]` and mechanical fixes

**Files:**
- Modify: `common/src/password.rs`
- Modify: `common/src/slug.rs`
- Modify: `common/src/tag.rs`
- Modify: `common/src/username.rs`
- Modify: `common/src/storage.rs`

- [ ] **Step 1: Add `#[must_use]` to `as_str()` in password, slug, tag, username**

  In `common/src/password.rs:35`, `common/src/slug.rs:39`, `common/src/tag.rs:41`, `common/src/username.rs:34`, add `#[must_use]` before each `pub fn as_str`:

  ```rust
  #[must_use]
  pub fn as_str(&self) -> &str {
  ```

- [ ] **Step 2: Add `#[must_use]` to `slugify_title` in slug.rs**

  In `common/src/slug.rs:54`:
  ```rust
  #[must_use]
  pub fn slugify_title(title: &str) -> Option<String> {
  ```

- [ ] **Step 3: Fix `ignored_unit_patterns` in `common/src/password.rs:65`**

  Change:
  ```rust
  Ok(_) => Ok(true),
  ```
  To:
  ```rust
  Ok(()) => Ok(true),
  ```

- [ ] **Step 4: Fix `doc_markdown` in `common/src/storage.rs:562`**

  Change:
  ```
  /// free of SQLite implementation details.
  ```
  To:
  ```
  /// free of `SQLite` implementation details.
  ```

- [ ] **Step 5: Run clippy for common crate**

  Run: `cargo clippy -p common 2>&1`
  Expected: No warnings from the lints addressed in steps 1–4.

- [ ] **Step 6: Commit**

  ```bash
  git add common/src/password.rs common/src/slug.rs common/src/tag.rs common/src/username.rs common/src/storage.rs
  git commit -m "fix: common crate must_use, unit patterns, doc_markdown"
  ```

---

## Task 2: `common` crate — `# Errors` documentation

**Files:**
- Modify: `common/src/password.rs`
- Modify: `common/src/render.rs`
- Modify: `common/src/smtp.rs`

- [ ] **Step 1: Add `# Errors` to `Password::hash` and `Password::verify` in `common/src/password.rs`**

  Read `common/src/password.rs` to see exact doc comment locations, then add:

  For `hash` (line 43):
  ```rust
  /// # Errors
  ///
  /// Returns `Err` if bcrypt hashing fails.
  pub fn hash(&self) -> Result<String, String> {
  ```

  For `verify` (line 60):
  ```rust
  /// # Errors
  ///
  /// Returns `Err` if bcrypt verification fails.
  pub fn verify(&self, hash: &str) -> Result<bool, String> {
  ```

- [ ] **Step 2: Add `# Errors` to functions in `common/src/render.rs`**

  Read `common/src/render.rs` lines 25, 221, 245, 305 and add appropriate `# Errors` sections. Each should describe the error type returned:

  For `render` (line 25):
  ```rust
  /// # Errors
  ///
  /// Returns `Err(RenderError)` if the body cannot be rendered for the given format.
  pub fn render(body: &str, format: &PostFormat) -> Result<String, RenderError> {
  ```

  For `create_rendered_post` (line 221):
  ```rust
  /// # Errors
  ///
  /// Returns `Err(CreateRenderedPostError)` if rendering fails or the storage layer returns an error.
  pub async fn create_rendered_post(
  ```

  For `update_rendered_post` (line 245):
  ```rust
  /// # Errors
  ///
  /// Returns `Err(UpdateRenderedPostError)` if rendering fails or the storage layer returns an error.
  pub async fn update_rendered_post(
  ```

  For `perform_post_update` (line 305):
  ```rust
  /// # Errors
  ///
  /// Returns `Err(PerformUpdateError)` if rendering fails or the storage layer returns an error.
  pub async fn perform_post_update(
  ```

- [ ] **Step 3: Add `# Errors` to `load_smtp_config` in `common/src/smtp.rs:105`**

  Read the function, then add:
  ```rust
  /// # Errors
  ///
  /// Returns `Err(SmtpConfigError)` if the site config cannot be retrieved from storage.
  pub async fn load_smtp_config(
  ```

- [ ] **Step 4: Run clippy for common crate**

  Run: `cargo clippy -p common 2>&1`
  Expected: Zero warnings.

- [ ] **Step 5: Commit**

  ```bash
  git add common/src/password.rs common/src/render.rs common/src/smtp.rs
  git commit -m "docs: add # Errors sections to common crate public API"
  ```

---

## Task 3: `hydrate` crate — wildcard import

**Files:**
- Modify: `hydrate/src/lib.rs`

- [ ] **Step 1: Fix wildcard import at `hydrate/src/lib.rs:164`**

  Read `hydrate/src/lib.rs` to find the exact context. Change:
  ```rust
  use web::*;
  ```
  To:
  ```rust
  use web::App;
  ```

- [ ] **Step 2: Run clippy for hydrate crate**

  Run: `cargo clippy -p hydrate 2>&1`
  Expected: Zero warnings.

- [ ] **Step 3: Commit**

  ```bash
  git add hydrate/src/lib.rs
  git commit -m "fix: replace wildcard import in hydrate crate"
  ```

---

## Task 4: `web` crate — `#[must_use]` attributes

**Files:**
- Modify: `web/src/lib.rs`
- Modify: `web/src/error.rs`
- Modify: `web/src/pages/auth.rs`
- Modify: `web/src/pages/backup.rs`
- Modify: `web/src/pages/email.rs`
- Modify: `web/src/pages/home.rs`
- Modify: `web/src/pages/invites.rs`
- Modify: `web/src/pages/mod.rs`
- Modify: `web/src/pages/password_reset.rs`
- Modify: `web/src/pages/posts.rs`
- Modify: `web/src/pages/profile.rs`
- Modify: `web/src/pages/sessions.rs`
- Modify: `web/src/pages/ui.rs`

- [ ] **Step 1: Add `#[must_use]` to `shell` in `web/src/lib.rs:52`**

  ```rust
  #[must_use]
  pub fn shell(options: LeptosOptions) -> impl IntoView {
  ```

- [ ] **Step 2: Add `#[must_use]` to error module methods in `web/src/error.rs`**

  Add to `public` (line 159), `operator_message` (line 163), `into_public` (line 167):
  ```rust
  #[must_use]
  pub fn public(&self) -> &WebError {

  #[must_use]
  pub fn operator_message(&self) -> &str {

  #[must_use]
  pub fn into_public(self) -> WebError {
  ```

- [ ] **Step 3: Add `#[must_use]` to all Leptos page component functions**

  Add `#[must_use]` before each `pub fn` that returns `impl IntoView` in these files:
  - `web/src/pages/auth.rs`: `RegisterPage` (line 7), `LoginPage` (line 59), `LogoutPage` (line 94)
  - `web/src/pages/backup.rs`: `BackupSettingsPage` (line 7)
  - `web/src/pages/email.rs`: `EmailPage` (line 9)
  - `web/src/pages/home.rs`: `HomePage` (line 20)
  - `web/src/pages/invites.rs`: `InvitesPage` (line 9)
  - `web/src/pages/mod.rs`: `App` (line 60)
  - `web/src/pages/password_reset.rs`: `ForgotPasswordPage` (line 11), `ResetPasswordPage` (line 42)
  - `web/src/pages/posts.rs`: `CreatePostPage` (line 18), `PostPage` (line 190), `UserTimelinePage` (line 266), `DraftPreviewPage` (line 398), `DraftsPage` (line 670)
  - `web/src/pages/profile.rs`: `ProfilePage` (line 7)
  - `web/src/pages/sessions.rs`: `SessionsPage` (line 6)

  Note: `EditPostPage` (line 487 in posts.rs) also needs `#[must_use]`.

- [ ] **Step 4: Add `#[must_use]` to UI component functions in `web/src/pages/ui.rs`**

  Add `#[must_use]` before each of: `Icon`, `avatar_parts`, `Avatar`, `Dot`, `Chip`, `BackupBanner`, `Topbar`, `ComposerFields`, `PostDisplay`, `PostCard`, `InlineComposer`, `Sidebar` (lines 33, 56, 70, 86, 94, 112, 136, 161, 237, 291, 380, 520).

- [ ] **Step 5: Run clippy — check must_use warnings are gone for web**

  Run: `cargo clippy -p web 2>&1 | grep "must_use_candidate"`
  Expected: No output.

- [ ] **Step 6: Commit**

  ```bash
  git add web/src/lib.rs web/src/error.rs web/src/pages/
  git commit -m "fix: add #[must_use] to web crate public functions and components"
  ```

---

## Task 5: `web` crate — mechanical code fixes

**Files:**
- Modify: `web/src/auth.rs`
- Modify: `web/src/backup.rs`
- Modify: `web/src/posts.rs`
- Modify: `web/src/pages/auth.rs`
- Modify: `web/src/pages/home.rs`
- Modify: `web/src/pages/invites.rs`
- Modify: `web/src/pages/posts.rs`
- Modify: `web/src/pages/ui.rs`

- [ ] **Step 1: Fix `unnested_or_patterns` in `web/src/auth.rs:59`**

  Read the file to see the exact pattern, then change:
  ```rust
  AuthRejection::MissingToken
  | AuthRejection::Session(common::storage::SessionAuthError::InvalidToken)
  | AuthRejection::Session(common::storage::SessionAuthError::SessionNotFound) => {
  ```
  To:
  ```rust
  AuthRejection::MissingToken |
  AuthRejection::Session(
      common::storage::SessionAuthError::InvalidToken |
      common::storage::SessionAuthError::SessionNotFound,
  ) => {
  ```

- [ ] **Step 2: Fix `map_unwrap_or` in `web/src/auth.rs` (2 instances)**

  Lines 156–158 and 181–183. Both follow the same pattern — change:
  ```rust
  .map(|settings| settings.secure)
  .unwrap_or(true)
  ```
  To:
  ```rust
  .map_or(true, |settings| settings.secure)
  ```

- [ ] **Step 3: Fix `uninlined_format_args` in `web/src/auth.rs` (2 instances)**

  Line 166:
  ```rust
  let cookie = format!(
      "session={}; HttpOnly; SameSite=Lax; Path=/{}",
      raw_token, secure_attr
  );
  ```
  To:
  ```rust
  let cookie = format!(
      "session={raw_token}; HttpOnly; SameSite=Lax; Path=/{secure_attr}"
  );
  ```

  Line 191:
  ```rust
  let cookie = format!(
      "session=; HttpOnly; SameSite=Lax; Path=/{}; Max-Age=0",
      secure_attr
  );
  ```
  To:
  ```rust
  let cookie = format!(
      "session=; HttpOnly; SameSite=Lax; Path=/{secure_attr}; Max-Age=0"
  );
  ```

- [ ] **Step 4: Fix `map_unwrap_or_else` (3 instances) in `web/src/backup.rs`**

  Lines 61–64, 68–71, 75–78. Each follows the pattern:
  ```rust
  value
      .filter(|value| some_validator(value))
      .map(|value| value.trim().to_owned())
      .unwrap_or_else(some_default)
  ```
  Change each to:
  ```rust
  value
      .filter(|value| some_validator(value))
      .map_or_else(some_default, |value| value.trim().to_owned())
  ```

  Specifically:
  - Line 61: `.unwrap_or_else(default_backup_schedule)` → `.map_or_else(default_backup_schedule, |value| value.trim().to_owned())`
  - Line 68: `.unwrap_or_else(default_backup_retention_count)` → `.map_or_else(default_backup_retention_count, |value| value.trim().to_owned())`
  - Line 75: `.unwrap_or_else(default_backup_mode)` → `.map_or_else(default_backup_mode, |value| value.trim().to_owned())`

- [ ] **Step 5: Fix `redundant_closure` in `web/src/posts.rs:132`**

  Change:
  ```rust
  .map(|slug| slug.to_ascii_lowercase())
  ```
  To:
  ```rust
  .map(str::to_ascii_lowercase)
  ```

- [ ] **Step 6: Fix `ignored_unit_patterns` in page files**

  In each location, change `|_|` to `|()|` where the argument is `()`:

  - `web/src/pages/auth.rs:9`: `|_| get_registration_policy()` → `|()| get_registration_policy()`
  - `web/src/pages/auth.rs:109`: `Ok(_) =>` → `Ok(()) =>`
  - `web/src/pages/home.rs:30`: `move |_|` → `move |()|`
  - `web/src/pages/invites.rs:11`: `|_| get_registration_policy()` → `|()| get_registration_policy()`
  - `web/src/pages/posts.rs:20`: `|_| current_user()` → `|()| current_user()`
  - `web/src/pages/posts.rs:224`: `move |_|` → `move |()|`
  - `web/src/pages/posts.rs:279`: `move |_|` → `move |()|`
  - `web/src/pages/ui.rs:113`: `|_| backup_warning_visible()` → `|()| backup_warning_visible()`
  - `web/src/pages/ui.rs:428`: `move |_|` → `move |()|`

- [ ] **Step 7: Fix `uninlined_format_args` in `web/src/pages/ui.rs:301`**

  Change:
  ```rust
  let edit_url = format!("/posts/{}/edit", post_id);
  ```
  To:
  ```rust
  let edit_url = format!("/posts/{post_id}/edit");
  ```

- [ ] **Step 8: Fix `redundant_closure` in `web/src/pages/ui.rs:455`**

  Change:
  ```rust
  .and_then(|r| r.err())
  ```
  To:
  ```rust
  .and_then(Result::err)
  ```

- [ ] **Step 9: Fix `manual_string_new` in `web/src/pages/posts.rs:517`**

  Change:
  ```rust
  <Topbar title="Edit Post".to_string() sub="".to_string() />
  ```
  To:
  ```rust
  <Topbar title="Edit Post".to_string() sub=String::new() />
  ```

- [ ] **Step 10: Run tests**

  Run: `cargo nextest run -p web 2>&1`
  Expected: All tests pass.

- [ ] **Step 11: Commit**

  ```bash
  git add web/src/auth.rs web/src/backup.rs web/src/posts.rs web/src/pages/
  git commit -m "fix: web crate mechanical clippy fixes (format args, closures, patterns, map_or)"
  ```

---

## Task 6: `web` crate — allow annotations for intentional patterns

**Files:**
- Modify: `web/src/pages/home.rs`
- Modify: `web/src/pages/posts.rs`
- Modify: `web/src/pages/ui.rs`
- Modify: `web/src/posts.rs`

- [ ] **Step 1: Fix `used_underscore_binding` in `web/src/pages/home.rs`**

  Read the file. The bindings `_next_cursor_created_at` (line 23) and `_next_cursor_post_id` (line 24) are prefixed with `_` but then used on lines 57–58. Remove the `_` prefix in both the binding declarations and all uses:

  Line 23: `let _next_cursor_created_at` → `let next_cursor_created_at`
  Line 24: `let _next_cursor_post_id` → `let next_cursor_post_id`
  Line 57: `_next_cursor_created_at.set(...)` → `next_cursor_created_at.set(...)`
  Line 58: `_next_cursor_post_id.set(...)` → `next_cursor_post_id.set(...)`

- [ ] **Step 2: Add `#[allow]` for `too_many_lines` on Leptos components**

  These are view components that cannot reasonably be split without creating many tiny single-use sub-components. Add `#[allow(clippy::too_many_lines)]` before the function attribute:

  - `web/src/pages/home.rs:20` before `HomePage`
  - `web/src/pages/posts.rs:18` before `CreatePostPage`
  - `web/src/pages/posts.rs:266` before `UserTimelinePage`
  - `web/src/pages/posts.rs:487` before `EditPostPage`
  - `web/src/pages/ui.rs:520` before `Sidebar`

  Example:
  ```rust
  #[allow(clippy::too_many_lines)]
  pub fn HomePage() -> impl IntoView {
  ```

- [ ] **Step 3: Add `#[allow]` for `needless_pass_by_value` on Leptos component props**

  Leptos component props require owned types; changing `String` to `&str` requires lifetime annotations that Leptos `#[component]` does not support. Add allows:

  - `web/src/pages/ui.rs:70` before `Avatar`:
    ```rust
    #[allow(clippy::needless_pass_by_value)]
    pub fn Avatar(name: String, #[prop(default = 38)] size: u32) -> impl IntoView {
    ```

  - `web/src/pages/ui.rs:86` before `Dot`:
    ```rust
    #[allow(clippy::needless_pass_by_value)]
    pub fn Dot(proto: String) -> impl IntoView {
    ```

  - `web/src/pages/ui.rs:238` on `PostDisplay`'s `post` param — read the full function signature. The `post: TimelinePostSummary` is used by value in the component body. Add `#[allow(clippy::needless_pass_by_value)]` before `PostDisplay`.

- [ ] **Step 4: Add `#[allow]` for cast warnings in `avatar_parts` in `web/src/pages/ui.rs:72`**

  The font size calculation `(size as f32 * 0.36).round() as u32` uses intentional float arithmetic on a small integer. The casts are safe in practice (size is a small UI value, never > 10,000). Add before `avatar_parts`:

  ```rust
  #[allow(clippy::cast_precision_loss)]
  #[allow(clippy::cast_possible_truncation)]
  #[allow(clippy::cast_sign_loss)]
  pub fn avatar_parts(name: &str) -> (String, u32) {
  ```

- [ ] **Step 5: Add `#[allow]` for `items_after_statements` in `Sidebar` in `web/src/pages/ui.rs`**

  The `NAV_ITEMS` const is defined mid-function for locality. Read the function to see the exact location. Since it's a `const` (not a runtime expression), this is idiomatic. Add on the `const` declaration:

  ```rust
  #[allow(clippy::items_after_statements)]
  const NAV_ITEMS: &[(&str, &str, &str, Option<&'static str>, bool)] = &[
  ```

- [ ] **Step 6: Fix `needless_pass_by_value` in `web/src/posts.rs:812` (can actually fix)**

  This is not a Leptos component. Read the function:
  ```rust
  fn private_post_not_found_error(error: InternalError) -> InternalError {
  ```
  Change to:
  ```rust
  fn private_post_not_found_error(error: &InternalError) -> InternalError {
  ```
  Then update the call site to pass a reference.

- [ ] **Step 7: Run tests**

  Run: `cargo nextest run -p web 2>&1`
  Expected: All tests pass.

- [ ] **Step 8: Commit**

  ```bash
  git add web/src/pages/home.rs web/src/pages/posts.rs web/src/pages/ui.rs web/src/posts.rs
  git commit -m "fix: web crate allow annotations for Leptos patterns, rename cursor bindings"
  ```

---

## Task 7: `web` crate — documentation

**Files:**
- Modify: `web/src/auth.rs`
- Modify: `web/src/error.rs`

- [ ] **Step 1: Add `# Errors` to `require_auth` in `web/src/auth.rs:120`**

  Read the function to understand when it errors. Then add:
  ```rust
  /// # Errors
  ///
  /// Returns `Err` if the request is not authenticated (missing or invalid session token).
  pub async fn require_auth() -> InternalResult<AuthUser> {
  ```

- [ ] **Step 2: Add `# Errors` to `server_boundary` in `web/src/error.rs:180`**

  Read the function, then add:
  ```rust
  /// # Errors
  ///
  /// Returns `Err(ServerFnError)` if the wrapped future returns an `InternalError`.
  pub async fn server_boundary<T>(
  ```

- [ ] **Step 3: Run clippy for web crate**

  Run: `cargo clippy -p web 2>&1`
  Expected: Zero warnings.

- [ ] **Step 4: Commit**

  ```bash
  git add web/src/auth.rs web/src/error.rs
  git commit -m "docs: add # Errors sections to web crate public API"
  ```

---

## Task 8: `server` crate — `#[must_use]` attributes

**Files:**
- Modify: `server/src/auth.rs`
- Modify: `server/src/cli.rs`
- Modify: `server/src/storage/sqlite.rs`
- Modify: `server/src/storage/postgres.rs`

- [ ] **Step 1: Add `#[must_use]` to `generate_token` in `server/src/auth.rs:12`**

  ```rust
  #[must_use]
  pub fn generate_token() -> String {
  ```

- [ ] **Step 2: Add `#[must_use]` to `is_prod` in `server/src/cli.rs:55`**

  ```rust
  #[must_use]
  pub fn is_prod(self) -> bool {
  ```

- [ ] **Step 3: Add `#[must_use]` to all `new` constructors in `server/src/storage/sqlite.rs`**

  There are 7 storage impl structs, each with a `pub fn new(pool: SqlitePool) -> Self`. Read the file and add `#[must_use]` before each one:
  - Line 89
  - Line 127
  - Line 340
  - Line 434
  - Line 517
  - Line 616
  - Line 690
  - Line 832

  Each becomes:
  ```rust
  #[must_use]
  pub fn new(pool: SqlitePool) -> Self {
  ```

- [ ] **Step 4: Add `#[must_use]` to all `new` constructors in `server/src/storage/postgres.rs`**

  Same pattern — there are 8 `new` constructors. Read the file and add `#[must_use]` before each one:
  - Lines 35, 72, 287, 381, 459, 546, 615, 757

  Each becomes:
  ```rust
  #[must_use]
  pub fn new(pool: PgPool) -> Self {
  ```

- [ ] **Step 5: Run clippy — check must_use warnings are gone for server storage**

  Run: `cargo clippy -p jaunder 2>&1 | grep "must_use_candidate"`
  Expected: No output.

- [ ] **Step 6: Commit**

  ```bash
  git add server/src/auth.rs server/src/cli.rs server/src/storage/sqlite.rs server/src/storage/postgres.rs
  git commit -m "fix: add #[must_use] to server crate constructors and pure functions"
  ```

---

## Task 9: `server` crate — mechanical code fixes

**Files:**
- Modify: `server/src/commands.rs`
- Modify: `server/src/lib.rs`
- Modify: `server/src/mailer.rs`
- Modify: `server/src/observability.rs`
- Modify: `server/src/storage/backup/mod.rs`
- Modify: `server/src/storage/backup/postgres.rs`
- Modify: `server/src/storage/mod.rs`
- Modify: `server/src/storage/sqlite.rs`
- Modify: `server/src/storage/postgres.rs`

- [ ] **Step 1: Fix all `cast_lossless` in `server/src/storage/sqlite.rs` (10 instances)**

  Read the file. Every occurrence of `.bind(limit as i64)` should become `.bind(i64::from(limit))`. The affected lines are: 1022, 1038, 1064, 1077, 1106, 1121, 1267, 1284, 1333, 1352.

  Change each:
  ```rust
  .bind(limit as i64)
  ```
  To:
  ```rust
  .bind(i64::from(limit))
  ```

- [ ] **Step 2: Fix all `cast_lossless` in `server/src/storage/postgres.rs` (10 instances)**

  Same change in postgres.rs at lines: 946, 962, 987, 1000, 1028, 1043, 1187, 1204, 1252, 1271.

- [ ] **Step 3: Fix `uninlined_format_args` in `server/src/commands.rs` (4 instances)**

  Lines 110–113, 128–131, 135–138, 171. Read the file and change each:

  Line 110:
  ```rust
  format!(
      "application role '{}' already exists; refusing to modify existing role state",
      app_role
  ),
  ```
  To:
  ```rust
  format!("application role '{app_role}' already exists; refusing to modify existing role state"),
  ```

  Line 128:
  ```rust
  format!(
      "database '{}' already exists; refusing to modify existing database state",
      database_name
  ),
  ```
  To:
  ```rust
  format!("database '{database_name}' already exists; refusing to modify existing database state"),
  ```

  Line 135:
  ```rust
  println!(
      "PostgreSQL ready: role='{}' database='{}' owner='{}'",
      app_role, database_name, app_role
  );
  ```
  To:
  ```rust
  println!("PostgreSQL ready: role='{app_role}' database='{database_name}' owner='{app_role}'");
  ```

  Line 171:
  ```rust
  println!("Created user '{}' with id {user_id}", username);
  ```
  To:
  ```rust
  println!("Created user '{username}' with id {user_id}");
  ```

- [ ] **Step 4: Fix `redundant_closure` in `server/src/mailer.rs` (2 instances)**

  Line 136:
  ```rust
  .map(|a| a.to_string())
  ```
  To:
  ```rust
  .map(ToString::to_string)
  ```

  Line 139:
  ```rust
  .map(|a| a.to_string())
  ```
  To:
  ```rust
  .map(ToString::to_string)
  ```

- [ ] **Step 5: Fix `redundant_closure` in `server/src/lib.rs:323`**

  Change:
  ```rust
  self.0.keys().map(|name| name.as_str()).collect()
  ```
  To:
  ```rust
  self.0.keys().map(axum::http::HeaderName::as_str).collect()
  ```

- [ ] **Step 6: Fix `map_unwrap_or` in `server/src/observability.rs:54`**

  Change:
  ```rust
  .map(Duration::from_millis)
  .unwrap_or(Duration::from_secs(5))
  ```
  To:
  ```rust
  .map_or(Duration::from_secs(5), Duration::from_millis)
  ```

- [ ] **Step 7: Fix `map_unwrap_or` in `server/src/storage/mod.rs:396`**

  Change:
  ```rust
  .map(Duration::from_millis)
  .unwrap_or(Duration::from_millis(100))
  ```
  To:
  ```rust
  .map_or(Duration::from_millis(100), Duration::from_millis)
  ```

- [ ] **Step 8: Fix `map_unwrap_or` in `server/src/storage/backup/mod.rs:120`**

  Change:
  ```rust
  .map(TemporaryBackupDirectory::path)
  .unwrap_or(options.source_path)
  ```
  To:
  ```rust
  .map_or(options.source_path, TemporaryBackupDirectory::path)
  ```

- [ ] **Step 9: Fix `match_same_arms` in `server/src/storage/backup/postgres.rs:172`**

  Read the match, then remove the redundant `"text"` arm since `_` already covers it:
  ```rust
  "timestamptz" => "TIMESTAMPTZ",
  _ => "TEXT",
  ```
  (Delete the `"text" => "TEXT",` line.)

- [ ] **Step 10: Fix `match_same_arms` in `server/src/storage/backup/mod.rs:241`**

  Merge the three arms that return `&["token_hash"]`:
  ```rust
  "sessions" | "email_verifications" | "password_resets" => &["token_hash"],
  "invites" => &["code"],
  "posts" => &["post_id"],
  ```

- [ ] **Step 11: Fix `match_wildcard_for_single_variants` in `server/src/commands.rs:43`**

  Change:
  ```rust
  _ => Err(anyhow::anyhow!("{label} must be a PostgreSQL URL")),
  ```
  To:
  ```rust
  DbConnectOptions::Sqlite(_) => Err(anyhow::anyhow!("{label} must be a PostgreSQL URL")),
  ```

- [ ] **Step 12: Fix `doc_markdown` in `server/src/cli.rs` (4 instances) and `server/src/storage/sqlite.rs`**

  In `server/src/cli.rs`, change bare `PostgreSQL` to `` `PostgreSQL` `` in doc comments at lines 25, 33, 35, 39, 86.

  In `server/src/storage/sqlite.rs:681`, change `/// SQLite implementation` to `` /// `SQLite` implementation ``.

- [ ] **Step 13: Run tests**

  Run: `cargo nextest run -p jaunder 2>&1`
  Expected: All tests pass.

- [ ] **Step 14: Commit**

  ```bash
  git add server/src/commands.rs server/src/lib.rs server/src/mailer.rs server/src/observability.rs server/src/cli.rs server/src/storage/
  git commit -m "fix: server crate mechanical clippy fixes (casts, format args, closures, match arms)"
  ```

---

## Task 10: `server` crate — structural refactors

**Files:**
- Modify: `server/src/commands.rs`
- Modify: `server/src/lib.rs`
- Modify: `server/src/storage/sqlite.rs`
- Modify: `server/src/storage/postgres.rs`
- Modify: `server/src/storage/backup/mod.rs`

- [ ] **Step 1: Fix `manual_let_else` in `server/src/storage/sqlite.rs:216`**

  Read lines 210–235 to see the full match. Change the pattern-matching `let (...) = match row { Some(tuple) => tuple, None => return Err(...) };` to `let...else`:

  ```rust
  let Some((
      user_id,
      username,
      display_name,
      bio,
      created_at,
      _last_authenticated_at,
      hash,
      email,
      email_verified,
      is_operator,
  )) = row else {
      return Err(UserAuthError::InvalidCredentials);
  };
  ```

- [ ] **Step 2: Fix `manual_let_else` in `server/src/storage/sqlite.rs:787`**

  Read lines 785–810. Change:
  ```rust
  let user_id = if let Some((user_id,)) = claimed {
      user_id
  } else {
      // ... complex block that returns early ...
  };
  ```
  To:
  ```rust
  let Some((user_id,)) = claimed else {
      // ... same complex block ...
  };
  ```

- [ ] **Step 3: Fix `manual_let_else` in `server/src/storage/postgres.rs:162`**

  Same as sqlite step 1 — read lines 160–180 and apply the same `let Some((... )) = row else { return Err(UserAuthError::InvalidCredentials); };` pattern.

- [ ] **Step 4: Fix `manual_let_else` in `server/src/storage/postgres.rs:710`**

  Same as sqlite step 2 — read lines 708–730 and apply the `let Some((user_id,)) = claimed else { ... };` pattern.

- [ ] **Step 5: Fix `manual_let_else` in `server/src/storage/backup/mod.rs:545`**

  Read lines 543–550. Change:
  ```rust
  let parent = match destination_path.parent() {
      Some(parent) => parent,
      None => return Ok(None),
  };
  ```
  To:
  ```rust
  let Some(parent) = destination_path.parent() else {
      return Ok(None);
  };
  ```

- [ ] **Step 6: Fix `single_match_else` in `server/src/commands.rs:153`**

  Read lines 150–165. Change:
  ```rust
  let password = match password {
      Some(p) => p,
      None => {
          let p1 = rpassword::prompt_password("Password: ")?;
          let p2 = rpassword::prompt_password("Confirm password: ")?;
          if p1 != p2 {
              return Err(anyhow::anyhow!("passwords do not match"));
          }
          p1.parse::<Password>().map_err(|e| anyhow::anyhow!("{e}"))?
      }
  };
  ```
  To:
  ```rust
  let password = if let Some(p) = password {
      p
  } else {
      let p1 = rpassword::prompt_password("Password: ")?;
      let p2 = rpassword::prompt_password("Confirm password: ")?;
      if p1 != p2 {
          return Err(anyhow::anyhow!("passwords do not match"));
      }
      p1.parse::<Password>().map_err(|e| anyhow::anyhow!("{e}"))?
  };
  ```

- [ ] **Step 7: Fix `single_match_else` in `server/src/lib.rs` (2 instances)**

  Read lines 144–162. Change the two nested `match` expressions to `if let`:

  Line 146:
  ```rust
  Some(value) => match value.parse::<usize>() {
      Ok(value) => value,
      Err(_) => {
          invalid_keys.push(BACKUP_RETENTION_COUNT_KEY);
          default_backup_retention_count()
      }
  },
  ```
  To:
  ```rust
  Some(value) => if let Ok(value) = value.parse::<usize>() {
      value
  } else {
      invalid_keys.push(BACKUP_RETENTION_COUNT_KEY);
      default_backup_retention_count()
  },
  ```

  Line 156:
  ```rust
  Some(value) => match parse_backup_mode(&value) {
      Some(mode) => mode,
      None => {
          invalid_keys.push(BACKUP_MODE_KEY);
          default_backup_mode()
      }
  },
  ```
  To:
  ```rust
  Some(value) => if let Some(mode) = parse_backup_mode(&value) {
      mode
  } else {
      invalid_keys.push(BACKUP_MODE_KEY);
      default_backup_mode()
  },
  ```

- [ ] **Step 8: Run tests**

  Run: `cargo nextest run -p jaunder 2>&1`
  Expected: All tests pass.

- [ ] **Step 9: Commit**

  ```bash
  git add server/src/commands.rs server/src/lib.rs server/src/storage/sqlite.rs server/src/storage/postgres.rs server/src/storage/backup/mod.rs
  git commit -m "fix: server crate structural refactors (let..else, if let)"
  ```

---

## Task 11: `server` crate — documentation

**Files:**
- Modify: `server/src/auth.rs`
- Modify: `server/src/commands.rs`
- Modify: `server/src/lib.rs`
- Modify: `server/src/main.rs`
- Modify: `server/src/mailer.rs`
- Modify: `server/src/storage/mod.rs`
- Modify: `server/src/storage/backup/mod.rs`

- [ ] **Step 1: Add `# Errors` to `hash_token` in `server/src/auth.rs:22`**

  ```rust
  /// # Errors
  ///
  /// Returns `Err` if bcrypt hashing fails.
  pub fn hash_token(raw_token: &str) -> Result<String, String> {
  ```

- [ ] **Step 2: Add `# Errors` to functions in `server/src/commands.rs`**

  Read the file to see what each returns and add appropriate sections to: `cmd_init` (line 22), `cmd_create_pg_db` (line 83), `cmd_user_create` (line 142), `cmd_user_invite` (line 175), `cmd_smtp_test` (line 190), `cmd_backup` (line 225), `cmd_restore` (line 247), `cmd_serve` (line 335).

  Each should describe the `anyhow::Result` error conditions. Example for `cmd_init`:
  ```rust
  /// # Errors
  ///
  /// Returns `Err` if the database cannot be opened or migrations fail.
  pub async fn cmd_init(storage: &StorageArgs, skip_if_exists: bool) -> anyhow::Result<()> {
  ```

- [ ] **Step 3: Add `# Errors` to `start_backup_worker` in `server/src/lib.rs:180`**

  ```rust
  /// # Errors
  ///
  /// Returns `Err` if the backup scheduler cannot be started.
  pub async fn start_backup_worker(
  ```

- [ ] **Step 4: Add both `# Errors` and `# Panics` to `run` in `server/src/main.rs:10`**

  Read lines 10–30. The function panics at line 23 via `.expect("serve subcommand present")`:

  ```rust
  /// # Panics
  ///
  /// Panics if `serve` subcommand is selected via implicit mode but the command is `None`.
  ///
  /// # Errors
  ///
  /// Returns `Err` if any subcommand fails.
  pub async fn run(cli: Cli) -> anyhow::Result<()> {
  ```

- [ ] **Step 5: Add `# Errors` to `Mailer::from_config` in `server/src/mailer.rs:39`**

  ```rust
  /// # Errors
  ///
  /// Returns `Err(BuildMailerError)` if the SMTP transport cannot be built from the config.
  pub fn from_config(config: &SmtpConfig) -> Result<Self, BuildMailerError> {
  ```

- [ ] **Step 6: Add `# Errors` to storage functions in `server/src/storage/mod.rs`**

  Add to `init_storage` (line 348), `open_database` (line 427), `open_existing_database` (line 438):

  ```rust
  /// # Errors
  ///
  /// Returns `Err` if the storage directory cannot be created.
  pub fn init_storage(path: &Path) -> io::Result<()> {
  ```

  ```rust
  /// # Errors
  ///
  /// Returns `Err` if the database connection pool cannot be established.
  pub async fn open_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
  ```

  ```rust
  /// # Errors
  ///
  /// Returns `Err` if the database connection pool cannot be established.
  pub async fn open_existing_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
  ```

- [ ] **Step 7: Add `# Errors` to backup functions in `server/src/storage/backup/mod.rs`**

  Add to `export_backup` (line 103), `restore_backup` (line 112), `mirror_media_directory` (line 456):

  ```rust
  /// # Errors
  ///
  /// Returns `Err(BackupError)` if the backup export fails.
  pub async fn export_backup(
  ```

  ```rust
  /// # Errors
  ///
  /// Returns `Err(BackupError)` if the backup restore fails.
  pub async fn restore_backup(
  ```

  ```rust
  /// # Errors
  ///
  /// Returns `Err(BackupError)` if copying or removing media files fails.
  pub fn mirror_media_directory(
  ```

- [ ] **Step 8: Run clippy for server crate**

  Run: `cargo clippy -p jaunder 2>&1`
  Expected: Zero warnings except for `needless_pass_by_value` and `cast_possible_truncation` (handled in next task).

- [ ] **Step 9: Commit**

  ```bash
  git add server/src/auth.rs server/src/commands.rs server/src/lib.rs server/src/main.rs server/src/mailer.rs server/src/storage/mod.rs server/src/storage/backup/mod.rs
  git commit -m "docs: add # Errors and # Panics sections to server crate public API"
  ```

---

## Task 12: `server` crate — remaining fixes

**Files:**
- Modify: `server/src/observability.rs`
- Modify: `server/src/storage/mod.rs`

- [ ] **Step 1: Fix `needless_pass_by_value` in `server/src/storage/mod.rs:96`**

  Read the function at line 93–100. Change:
  ```rust
  username: String,
  ```
  To:
  ```rust
  username: &str,
  ```
  Then verify all call sites pass `&username` or a `&str`.

- [ ] **Step 2: Fix `cast_possible_truncation` in `server/src/observability.rs:115`**

  Read the function. The cast `elapsed.as_millis() as u64` is intentional — durations exceeding `u64::MAX` milliseconds (~585 million years) will never occur in practice. Add allows:

  ```rust
  #[allow(clippy::cast_possible_truncation)]
  Some((elapsed.as_millis() as u64, threshold.as_millis() as u64))
  ```

- [ ] **Step 3: Run clippy — final check**

  Run: `cargo clippy 2>&1`
  Expected: Zero warnings across the entire workspace.

- [ ] **Step 4: Run full test suite**

  Run: `cargo nextest run 2>&1`
  Expected: All tests pass.

- [ ] **Step 5: Commit**

  ```bash
  git add server/src/observability.rs server/src/storage/mod.rs
  git commit -m "fix: server crate needless_pass_by_value and intentional cast allows"
  ```

---

## Verification

After all tasks are complete:

```bash
cargo clippy 2>&1
# Expected: no warnings

cargo nextest run 2>&1
# Expected: all tests pass

scripts/verify
# Expected: full verification passes
```
