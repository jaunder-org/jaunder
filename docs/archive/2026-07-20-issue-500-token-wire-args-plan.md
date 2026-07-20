# Type reset/verification `token` args as `RawToken` — Plan (issue #500)

> **For agentic workers:** Execute with **jaunder-iterate**. One task.

**Goal:** Type `confirm_password_reset` / `verify_email`'s `token` wire arg as
`RawToken` (dropping the in-body re-parse), closing #500's acceptance.

Spec: `docs/superpowers/specs/2026-07-20-issue-500-token-wire-args.md`
(AC1–AC5).

## Global Constraints

- No `Co-Authored-By` trailer. No new type (reuse `RawToken`). ADR-0065: assert
  non-OK, not message text. Verify web with
  `cargo check -p web --all-features --all-targets`.
- Per-commit gate `cargo xtask check`; final gate
  `cargo xtask validate --no-e2e`.

---

## Task 1: `token: RawToken` on both `#[server]` fns

**Files:**

- Modify: `web/src/password_reset/mod.rs` (`confirm_password_reset` :77-89).
- Modify: `web/src/email/mod.rs` (`verify_email` :60-66).
- Test: `server/tests/web/web_email.rs` +
  `server/tests/web/web_password_reset.rs` (a shape-invalid-token wire-rejection
  test on **each** changed fn — both use the
  `post_form_with_mailer(state, mailer, path, body, None)` harness with a
  `CapturingMailSender`).

**Interfaces:**

- Consumes: `RawToken` (full `StrNewtype` serde trailer; already imported in
  both files).
- Produces:
  `confirm_password_reset(token: RawToken, new_password: ProfferedPassword)`;
  `verify_email(token: RawToken)`.

- [ ] **Step 1: Add a wire-rejection characterization test on EACH changed fn.**
      A **shape-invalid** token (`!` is outside base64url `[A-Za-z0-9_-]`) must
      yield non-OK. Each passes on current code (in-body `RawToken::try_from`
      rejects it) and must stay green after the refactor (wire-decode rejects
      it) — guarding the preserved rejection across both changed fns (security
      surface, per the plan review).

  In `server/tests/web/web_email.rs` (mirror
  `verify_email_with_unknown_token_returns_error`):

```rust
#[apply(backends)]
#[tokio::test]
async fn verify_email_with_malformed_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    // `bad!token` is not valid base64url shape, so RawToken rejects it (in-body today,
    // at wire-decode once `token` is typed) — either way a non-OK response.
    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/verify_email",
        "token=bad!token",
        None,
    )
    .await;
    assert_ne!(status, StatusCode::OK, "a malformed verification token must be rejected");
}
```

In `server/tests/web/web_password_reset.rs` (mirror
`confirm_password_reset_with_invalid_token_returns_error`; `new_password` is
valid-length so the failure isolates to the token):

```rust
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_malformed_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/confirm_password_reset",
        "token=bad!token&new_password=newpassword456",
        None,
    )
    .await;
    assert_ne!(status, StatusCode::OK, "a malformed reset token must be rejected");
}
```

- [ ] **Step 2: Run both, verify they pass on current code.** Run:
      `cargo nextest run -p jaunder --test integration -E '(test(verify_email_with_malformed_token) or test(confirm_password_reset_with_malformed_token)) and test(sqlite)'`
      Expected: PASS (today's in-body `RawToken::try_from` already rejects the
      bad shape).

- [ ] **Step 3: Type `confirm_password_reset`'s token**
      (`web/src/password_reset/mod.rs`): change `token: String` →
      `token: RawToken`; delete the
      `let raw_token = RawToken::try_from(token).map_err(...)?;` block; pass the
      typed `token` to the storage call
      (`atomic.confirm_password_reset(&token, &password)`). Remove the
      `InternalError` import **iff** the compiler flags it as now-unused (it may
      still be used by `request_password_reset` in the same file — check).

- [ ] **Step 4: Type `verify_email`'s token** (`web/src/email/mod.rs`): change
      `token: String` → `token: RawToken`; delete the
      `RawToken::try_from(token).map_err(...)` block; pass `&token` to
      `use_email_verification`. (`InternalError` stays — still used by
      `InternalError::storage` at the tail.)

- [ ] **Step 5: Compile + run the reset/verify suites.** Run:
      `cargo check -p web --all-features --all-targets` Then:
      `cargo nextest run -p jaunder --test integration -E '(test(password_reset) or test(verify_email)) and test(sqlite)'`
      Expected: PASS — the new test and all existing reset/verify tests green
      (their invalid-token bodies are valid base64url shape → still fail at DB
      lookup → non-OK).

- [ ] **Step 6: Commit.** Run `cargo xtask check` first.
  ```bash
  git add web/src/password_reset/mod.rs web/src/email/mod.rs \
          server/tests/web/web_email.rs server/tests/web/web_password_reset.rs
  git commit -m "refactor(web): type reset/verify token wire args as RawToken (#500)"
  ```

---

## Final gate

- [ ] `cargo xtask validate --no-e2e` (AC5) → PASS, then hand off to
      **jaunder-ship**.

## Self-review

AC1→Step 3; AC2→Step 4; AC3→Steps 3+4 (no raw-String secrets left;
register/login untouched); AC4→Steps 1/2 + existing suites (Step 5); AC5→Final
gate. No new type; the `proffered-secret` gate already covers
`ProfferedPassword`.
