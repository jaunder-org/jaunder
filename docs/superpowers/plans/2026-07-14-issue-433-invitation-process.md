# Plan — #433: complete the invitation process

Spec:
[`docs/superpowers/specs/2026-07-14-issue-433-invitation-process.md`](../specs/2026-07-14-issue-433-invitation-process.md).
Read it for the _what/why_; this plan is the _how_.

## Review header

**Goal.** Wire both halves of the invite round trip, each mirroring an existing
password-reset exemplar: **deliver** (web `create_invite` emails the invite
link) and **accept** (`RegisterPage` consumes `?invite_code=` from the URL).

**Scope — in:** `EmailKind::Invite`; web `create_invite` gains a required
recipient

- composes the absolute link + sends email; the operator create-invite form
  gains a recipient field; `RegisterPage` reads the code from the URL (plain, no
  `with_untracked`), submits it hidden, and handles the no-code case; e2e for
  the link-register flow.

**Scope — out:** CLI email (URL only); recipient persistence (fire-and-forget);
the request-an-invitation flow (#444); `Email`-newtype typing (#397).

**Tasks:**

1. Add `EmailKind::Invite` (+ metric test).
2. Web `create_invite` emails the invite link; operator form gains a recipient
   field; update the web `create_invite` tests.
3. `RegisterPage` consumes the invite link (+ e2e for link-register and the
   no-code message).

**Key risks / decisions:**

- **`create_invite` signature change** (`+recipient_email`, now required) — its
  existing web tests (`server/tests/web/web_account.rs`) must add the recipient,
  set `site.base_url`, and provide a mailer; the operator form must submit the
  new field (compiles without it, but 400s at runtime — land them together).
- **Order to avoid orphans:** require `site.base_url` _before_ creating the
  invite, so a missing base URL doesn't leave an undelivered invite. A send
  failure _after_ create is accepted (fire-and-forget) and surfaced.
- **Part 3 is client/wasm** (`#[component]`, coverage-exempt) → its test is
  **e2e**; the existing `auth.spec.ts` invite path (manual field entry) must
  move to URL-driven, or it breaks.
- **No mailer/base-url configured** → emailing errors (NoopMailSender →
  `NotConfigured`; base-url unset → validation error). Expected.

**For agentic workers:** execute with `jaunder-iterate`, delegating a task to a
subagent via `jaunder-dispatch` when useful. Tick checkboxes in real time.

## Global constraints

- No `Co-Authored-By` trailer. Run `cargo xtask check` clean before each commit
  (`jaunder-commit`); serialize edit → gate → commit.
- `validate` runs **with e2e** at ship (this touches the register/e2e surface).
- Import discipline; comments state intent, not mechanics.

---

## Task 1 — `EmailKind::Invite`

**File:** `host/src/metrics.rs`.

- Extend the enum:
  `enum_attr!(EmailKind { Verification => "verification", PasswordReset => "password_reset", Invite => "invite" });`
  (line 34).
- **Test:** in the existing metrics test module, extend the `email_send_result`
  coverage (see `metrics.rs:257-258`) with
  `email_send_result(EmailKind::Invite, &Ok::<(), ()>(()));` so the new
  variant's label path is exercised.

**Run:** `cargo nextest run -p host metrics` → PASS. `cargo xtask check` →
commit (`feat(host): add EmailKind::Invite metric variant (#433)`).

## Task 2 — Web `create_invite` emails the invite link

**Files:** `web/src/invites/mod.rs`, `web/src/pages/invites.rs`,
`server/tests/web/web_account.rs`.

### `web/src/invites/mod.rs`

Mirror `request_password_reset` (`web/src/password_reset/mod.rs:46-66`). Add
`common::mailer::{EmailMessage, MailSender}` and `storage::SiteConfigStorage` to
the server-gated `use` block.

```rust
#[server(endpoint = "/create_invite")]
pub async fn create_invite(
    expires_in_hours: Option<u64>,
    recipient_email: String,
) -> WebResult<()> {
    boundary!("create_invite", {
        let _auth = require_auth().await?;
        let invites = expect_context::<Arc<dyn InviteStorage>>();
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let mailer = expect_context::<Arc<dyn MailSender>>();

        // Require the base URL up front — a missing one must not leave an
        // undelivered invite behind (no orphan on the error path).
        let base_url = site_config
            .get_identity()
            .await?
            .base_url
            .ok_or_else(|| InternalError::validation(
                "set the site base URL before emailing invites",
            ))?;

        let hours = expires_in_hours.unwrap_or(168);
        let duration = i64::try_from(hours)
            .ok()
            .and_then(chrono::Duration::try_hours)
            .ok_or_else(|| InternalError::validation("expires_in_hours too large"))?;
        let expires_at = Utc::now() + duration;

        let code = invites.create_invite(expires_at).await.map_err(InternalError::storage)?;
        host::metrics::invite(host::metrics::InviteEvent::Created);

        // Deliberate egress of the secret via `AsRef` (InviteCode has no Display/serde).
        let link = format!("{base_url}/register?invite_code={}", code.as_ref());
        let message = EmailMessage {
            from: None,
            to: vec![recipient_email],
            subject: "You've been invited to Jaunder".to_string(),
            body_text: format!(
                "You've been invited to create an account. Click the link below to register:\n\n{link}\n\nThis invitation expires in {hours} hours."
            ),
        };
        let send_result = mailer.send_email(&message).await;
        host::metrics::email_send_result(host::metrics::EmailKind::Invite, &send_result);
        send_result?; // MailError → WebError via `?`, as in request_password_reset
        Ok(())
    })
}
```

(The `InviteCode` is now _used_ to build the link — no longer discarded — but
still never returned to the client. `recipient_email` is a `String`; a malformed
address is rejected by the mailer (lettre) on send and surfaced.)

### `web/src/pages/invites.rs`

The create-invite `<ActionForm action=create_action>` (a
`ServerAction::<CreateInvite>`) gains a **required recipient email** input
beside `expires_in_hours`:

```rust
<input class="j-form-input" type="email" name="recipient_email" required=true />
```

On the action's success value, show "Invitation sent." (replace/augment the
current version-bump refetch; the list still refetches on
`create_action.version()`).

### `server/tests/web/web_account.rs`

Update the three `create_invite_*` tests
(`create_invite_appears_in_list_invites`,
`create_invite_unauthorized_returns_error`,
`create_invite_large_hours_returns_error`) and add new ones. The harness must,
for the success path: set `site.base_url`
(`state.site_config.set("site.base_url", "https://example.com")`), install a
`CapturingMailSender` in context (mirror the password-reset web tests), and POST
`expires_in_hours=24&recipient_email=invitee@example.com`. Assert:

- one email captured, `to == ["invitee@example.com"]`, body contains
  `https://example.com/register?invite_code=`;
- **base-url unset** → error (no email sent);
- **send failure** (a failing mailer) → the call errors;
- unauthorized (no cookie) → error (unchanged).

**Run:** `cargo nextest run -p server create_invite` → PASS (after updates).
`cargo xtask check` → commit
(`feat(web): email the invite link on create_invite (#433)`).

## Task 3 — `RegisterPage` consumes the invite link

**Files:** `web/src/pages/auth.rs`, `end2end/tests/auth.spec.ts` (+
`selectors.ts`/`helpers.ts` as needed).

### `web/src/pages/auth.rs`

Follow `ResetPasswordPage` in shape; read the param **plainly** (per spec — CSR,
no hydration race):

```rust
use leptos_router::hooks::use_query_map;
let invite_code = use_query_map().read().get("invite_code").unwrap_or_default();
```

In the `is_invite_only` branch, replace the manual
`<input type="text" name="invite_code" required>` with:

- **`!invite_code.is_empty()`** → a
  `<input type="hidden" name="invite_code" value=invite_code />` plus a
  read-only line, e.g.
  `<p class="j-form-note">"Registering with your invitation."</p>`.
- **empty** → render a guidance block instead of the form:
  `<p>"You need an invitation link to register."</p>` (the #444 request-form
  lands here later).

`register`'s wire arg is unchanged (`Option<ProfferedInviteCode>`); the hidden
field feeds it. The submit-disable stays `!username.is_valid()`.

### `end2end/tests/auth.spec.ts`

The existing invite path (typing into the manual field) must move to URL-driven,
or it breaks. Extend so, in `invite_only` mode:

- visiting `/register?invite_code=<code>` (mint a code via the operator/CLI seam
  the suite already uses), filling username+password, and submitting **registers
  successfully** (no manual code entry);
- visiting `/register` with **no** code shows the "need an invitation link" text
  and no register submit.

**Run:** `cargo nextest run -p web` (host build compiles). The behavior is
proven by e2e — `cargo xtask e2e sqlite chromium` (or the full matrix at ship).
`cargo xtask check` → commit
(`feat(web): register consumes the invite_code URL link (#433)`).

---

## Self-review checklist

- [ ] Clicking `/register?invite_code=<code>` registers end-to-end; no manual
      entry.
- [ ] Operator create-invite emails the absolute link to the recipient; base-url
      unset and send-failure both error.
- [ ] No code reaches a client (create_invite still returns `()`; the hidden
      field carries only what the URL supplied).
- [ ] `cargo xtask validate` clean (e2e included).
- [ ] No recipient persistence; CLI unchanged; #444/#397 left as follow-ons.
