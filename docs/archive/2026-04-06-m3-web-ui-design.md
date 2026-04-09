# M3 Steps 10 & 11: Email Verification and Password Reset Web UI

## Scope

Implement the Leptos web UI and server functions for the email verification flow (Step 10) and the
password reset flow (Step 11). All storage traits and migrations are already in place from Steps 1–9.

---

## Step 10: Email Verification Flow

### New file: `web/src/email.rs`

**Server functions:**

- `request_email_verification(email: String) -> Result<(), ServerFnError>`
  - Requires auth (`require_auth().await?`).
  - Parses `email` as `email_address::EmailAddress`; returns error on invalid format.
  - Calls `state.email_verifications.create_email_verification(user_id, &email, expires_at)`.
  - Sends an `EmailMessage` to the given address via `state.mailer` with a link to
    `/verify-email?token=<raw_token>`.
  - Returns `Ok(())`.

- `verify_email(token: String) -> Result<(), ServerFnError>`
  - No auth required.
  - Calls `state.email_verifications.use_email_verification(&token)`.
  - On success: calls `state.users.set_email(user_id, Some(&email_address), true)`.
  - Maps `UseEmailVerificationError` variants to descriptive `ServerFnError` messages:
    - `NotFound` → "verification token not found"
    - `Expired` → "verification token has expired"
    - `AlreadyUsed` → "verification token has already been used"

**Components:**

- `EmailPage`
  - Auth-protected (redirects or shows error if unauthenticated).
  - Loads current email and verified status via `get_profile()` (see ProfileData extension below).
  - Form with an email input calling `request_email_verification`.
  - On success: shows "Check your email for a verification link."
  - On error: shows error message.

- `VerifyEmailPage`
  - Reads `token` query parameter via `leptos_router::hooks::use_query_map()`.
  - Calls `verify_email(token)` on mount via `Resource`.
  - Renders success message or appropriate error.

**Routes added to `App` in `web/src/lib.rs`:**

```
/profile/email  →  EmailPage
/verify-email   →  VerifyEmailPage
```

### ProfileData extension

`ProfileData` in `web/src/profile.rs` gains:
- `email: Option<String>`
- `email_verified: bool`

`get_profile` populates these from `UserRecord`.

### Integration tests (`server/tests/web_email.rs`)

1. `request_email_verification` creates a verification row and sends via `CapturingMailSender`
   with correct recipient and body containing the token URL.
2. `verify_email` with a valid token sets email as verified on the user.
3. `verify_email` with an expired token returns an error.
4. `verify_email` with an unknown token returns an error.

### E2E tests (`end2end/tests/email.spec.ts`)

1. User adds email on `/profile/email`; token extracted from capture file; visit
   `/verify-email?token=...`; confirm email shown as verified on `/profile/email`.
2. Visiting `/verify-email` with an invalid token shows an error.

---

## Step 11: Password Reset Flow

### New file: `web/src/password_reset.rs`

**Server functions:**

- `request_password_reset(username: String) -> Result<(), ServerFnError>`
  - No auth required.
  - Looks up user by username via `get_user_by_username`.
  - If user not found or has no verified email: returns error "No verified email on file for this
    account. Please contact the site operator."
  - Otherwise: creates a reset token via `state.password_resets.create_password_reset`; sends
    `EmailMessage` with link to `/reset-password?token=<raw_token>`.
  - Returns `Ok(())` regardless of whether the username existed (avoids enumeration).

- `confirm_password_reset(token: String, new_password: String) -> Result<(), ServerFnError>`
  - No auth required.
  - Parses `new_password` as `Password`; returns error on invalid format.
  - Calls `state.password_resets.use_password_reset(&token)`; maps errors to descriptive messages.
  - Calls `state.users.set_password(user_id, &password)`.
  - Revokes all sessions: `list_sessions(user_id)` then `revoke_session` for each.
  - Returns `Ok(())`.

**Components:**

- `ForgotPasswordPage`
  - Username form calling `request_password_reset`.
  - On success: neutral message "If a verified email is on file, a reset link has been sent."
  - On error (no verified email / contact operator): surfaces the error message directly.

- `ResetPasswordPage`
  - Reads `token` query parameter.
  - New-password form calling `confirm_password_reset`.
  - On success: redirects to `/login` via `leptos_router`.
  - On error: shows error message.

**Routes added to `App` in `web/src/lib.rs`:**

```
/forgot-password  →  ForgotPasswordPage
/reset-password   →  ResetPasswordPage
```

### Integration tests (`server/tests/web_password_reset.rs`)

1. `request_password_reset` for user with verified email sends reset email via `CapturingMailSender`
   containing the token URL.
2. `request_password_reset` for user without verified email returns an error.
3. `request_password_reset` for unknown username returns an error.
4. `confirm_password_reset` with valid token sets new password and revokes all existing sessions.
5. `confirm_password_reset` with expired token returns an error.
6. `confirm_password_reset` with already-used token returns an error.

### E2E tests (`end2end/tests/password_reset.spec.ts`)

1. User requests reset on `/forgot-password`; token extracted from capture file; visit
   `/reset-password?token=...`; submit new password; confirm login with new password succeeds and
   old password fails.
2. Visiting `/reset-password` with an invalid token shows an error.
3. Submitting `/forgot-password` for user with no verified email shows the "contact operator" error.

---

## Email Capture for E2E Tests

### `FileMailSender` (in `server/src/mailer.rs`)

A new `MailSender` implementation that appends each outgoing `EmailMessage` as a JSON line to a
file path. Implements `MailSender` trait. Used only when `JAUNDER_MAIL_CAPTURE_FILE` env var is set.

JSON line format per message:
```json
{"to": ["addr@example.com"], "from": null, "subject": "...", "body_text": "..."}
```

### Server startup selection

`open_database` (in `server/src/storage/mod.rs`) checks `JAUNDER_MAIL_CAPTURE_FILE` at startup:
- If set: use `FileMailSender` writing to that path.
- Else if SMTP configured: use `LettreMailSender`.
- Else: use `NoopMailSender`.

### Nix e2e harness

The nix flake e2e test setup sets `JAUNDER_MAIL_CAPTURE_FILE` to a temp path before starting the
server. Playwright tests read this file after form submissions and parse JSON lines to extract tokens.

A helper function `readLastEmail(filePath)` is added to the e2e test utilities.

---

## Verification checklist

After each step:
1. `cargo build` succeeds.
2. `cargo nextest run` passes.
3. `cargo clippy -- -D warnings` is clean.
4. `scripts/check-coverage` succeeds.
5. `nix flake check` passes.
