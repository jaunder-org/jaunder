# Spec — #433: complete the invitation process (email delivery + link handling)

- Issue: [#433](https://github.com/jaunder-org/jaunder/issues/433)
- Depends on: #400 (done) — the `InviteCode` types + the CLI invite URL.
- Related: #397 (Email newtype, unmerged); #439 (closed, folded here); #444 (the
  no-code "request an invitation" flow, split out).
- Date: 2026-07-14

## Problem

#400 made the invite code a type and had the CLI print an invite URL
(`{base_url}/register?invite_code=<code>`), but neither half of the round trip
is wired:

- **Accept:** `RegisterPage` ignores `?invite_code=` — it has only a
  manually-typed field, so the delivered link is dead on the page side.
- **Deliver:** there is no way to _send_ an invite; the web `create_invite`
  can't even reveal the code (never sent server→client, #400), so a web-created
  invite is currently unreachable.

Both halves have a working exemplar in the password-reset flow (a link emailed
with a token; a page that reads the token from the URL).

## Part 2 — Accept the link (`web/src/pages/auth.rs`)

Follow `ResetPasswordPage` (`web/src/pages/password_reset.rs:61-86`) in shape —
read the code from the URL and submit it via a hidden field — but read the param
**plainly** (like `email.rs:64`), not via `with_untracked`:

```rust
let invite_code = use_query_map().read().get("invite_code").unwrap_or_default();
```

The app is **CSR** (`web/Cargo.toml` `csr`; `mount_csr` → `mount_to_body`, not
`hydrate_body`), so the `<Router>` has the parsed `window.location` before route
components render — the "empty query map during hydration" race that
`password_reset.rs` guards against with `with_untracked` is an SSR-era artifact
and doesn't apply here. (That stale guard/comment in `password_reset.rs` is a
trivial cleanup, noted below — not this issue.)

In **invite-only** mode:

- **Code present in the URL** → a
  `<input type="hidden" name="invite_code" value=…>` plus a read-only
  "Registering with your invitation" confirmation. **No editable text box.**
- **No code in the URL** (someone opened `/register` directly) → show a "You
  need an invitation link to register" message _instead of_ the form. (A public
  "request an invitation" form for this branch is the follow-on #444.)

Remove the manual `<input type="text" name="invite_code" required>` added in
#400. The `register` server fn is unchanged (`Option<ProfferedInviteCode>`); the
hidden field feeds it.

## Part 1 — Deliver by email (`web/src/invites/mod.rs`, operator UI)

Mirror `request_password_reset` (`web/src/password_reset/mod.rs:46-66`).

- **`create_invite` gains a recipient** and sends the link:
  `create_invite(expires_in_hours: Option<u64>, recipient_email: String)`. It
  builds the invite (as today), composes the **absolute** link
  `{base_url}/register?invite_code=<code>` from the generated `InviteCode`
  (`.as_ref()`), and emails it via `expect_context::<Arc<dyn MailSender>>()`
  (`EmailMessage` + `send_email`, with the `email_send_*` metrics like
  password-reset). Still returns `WebResult<()>` — the code is used to build the
  link, never returned to the client.
- **Operator UI** (`web/src/pages/invites.rs`): the create-invite form gains a
  required **recipient email** field; on success shows "Invitation emailed to
  &lt;addr&gt;."
- **Metrics:** add `EmailKind::Invite` (`host/src/metrics.rs`), emit
  `email_send_result(EmailKind::Invite, …)`.

## Decisions taken (please confirm at review)

1. **Email is a web-operator surface.** The web create-invite sends the email;
   the **CLI `jaunder user invite` is unchanged** — it keeps printing the URL
   for manual sharing. (CLI email is feasible via `build_mailer` but out of
   scope here.)
2. **Web create-invite requires a recipient email.** Since #400 removed the
   server→client code reveal, an emailed link is the _only_ way a web-created
   invite reaches anyone — so the recipient is required (create an invite _for_
   someone). An operator wanting a shareable link with no specific recipient
   uses the CLI.
3. **Fire-and-forget** — the recipient is **not** persisted (no `invites`
   column, no migration). The invite is still tracked by code/expiry/used.
4. **Absolute link needs `site.base_url`.** If it is unset, emailing errors
   ("configure the site base URL first") — a relative link is unusable in a mail
   client.
5. **Recipient is a `String`** wire arg, parsed to an
   `email_address::EmailAddress` in the server fn — validated **before** the
   invite is created (a malformed address errors with no orphan) rather than
   deferring to the mailer on send. Typing the wire arg itself as the #397
   `Email` newtype is a follow-up once #397 lands.
6. **No-code-in-URL** (Part 2) shows a "you need an invitation link" message,
   not a failing form.

## Tests

- **Part 2** is client/wasm behavior (`#[component]`, coverage-exempt) → an
  **e2e** test: visiting `/register?invite_code=<code>` in invite-only mode
  registers successfully; visiting `/register` with no code shows the guidance
  message. Extend the existing register e2e spec.
- **Part 1**: `web` server-fn test for `create_invite` — a `CapturingMailSender`
  asserts one email is sent to the recipient containing the absolute
  `…/register?invite_code=…` link; base-url-unset returns an error; a send
  failure propagates. Mirror the password-reset web tests.
- `EmailKind::Invite` metric unit coverage (host/metrics test).

## Acceptance

- Clicking an invite link (`/register?invite_code=<code>`) registers end-to-end;
  no manual code entry.
- The operator UI emails an invite link to a supplied recipient.
- `cargo xtask validate` clean (e2e included — this touches the web/register e2e
  surface).

## Non-goals

- CLI email delivery (web is the email surface; CLI prints the URL).
- Persisting the recipient on the invite record (fire-and-forget).
- Typing the recipient as an `Email` newtype (#397 follow-up).
- The public "request an invitation" flow for the no-code case (**#444**).
- Fixing password-reset's relative email link, and removing its now-stale
  `with_untracked` hydration-race guard/comment (latent CSR-era cleanups,
  separate).
