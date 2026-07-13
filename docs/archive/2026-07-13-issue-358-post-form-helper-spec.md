# Spec — issue #358: consolidate the duplicated `post_form` test helpers

- **Issue:** [#358](https://github.com/jaunder-org/jaunder/issues/358) (Task,
  test-infra)
- **Follow-up filed:** [#426](https://github.com/jaunder-org/jaunder/issues/426)
  — static guard so the registrar list stops rotting (blocked-by #358)
- **Kind:** pure test-infra refactor — no product-code or test-behavior change.

## Problem

`post_form` is copy-pasted across the web integration tests: 13 definitions in
11 files (`audiences`, `web_account`, `web_auth` ×3, `web_backup`, `web_email`,
`web_media`, `web_password_reset`, `web_posts`, `web_sessions`, `web_site`,
`web_subscriptions`), even though `server/tests/helpers/mod.rs` already holds
the shared web-test helpers every file imports. The copies have **drifted** —
they are not byte-identical — so consolidation must preserve each caller's
behavior, not just delete text.

### Drift the consolidation must absorb

| Axis                       | Divergence                                                                                                                                                      |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Return type                | 9 files → `(StatusCode, String)`; `web_account` + `web_auth` → `(StatusCode, Option<String>, String)` (captures `Set-Cookie`)                                   |
| `secure_cookies`           | hardcoded `true` everywhere **except `web_auth`**, where it is a parameter (it tests insecure-cookie behavior)                                                  |
| mailer                     | `noop_mailer()` everywhere **except `web_email` + `web_password_reset`**, which inject a `CapturingMailSender`                                                  |
| auth header                | cookie everywhere except `web_auth`'s `_with_bearer` (Authorization: Bearer) and `_with_ua` (adds User-Agent)                                                   |
| server-fn registration     | `web_media`'s copy registers 3 fns (`ListMyMedia`, `MediaUsage`, `DeleteMedia`) inline because they are absent from the shared `ensure_server_fns_registered()` |
| panic style / body binding | `.unwrap()` vs `.expect("…")`, inline vs intermediate `body_str` — cosmetic                                                                                     |

Two findings that shrink the work:

- **`web_account`'s 3-tuple is dead weight** — all 17 of its call sites discard
  the middle element (`(status, _, body)`). It collapses onto the plain 2-tuple.
  So **only `web_auth`** genuinely needs `Set-Cookie` capture + the
  `secure_cookies` flag.
- **`web_media`'s 3 fns** are simply missing from the shared registrar's
  explicit list; adding them there matches how every other domain is handled.

## Decision — shape A: one private core + thin typed wrappers

Chosen over a single kitchen-sink parameterized fn (would make all ~180 call
sites noisier) and over a builder (would rewrite all ~180 call sites and their
destructuring). Shape A matches the issue's "thin wrappers or params" framing,
touches the fewest call sites, and still collapses all router-building into
**one** implementation. (See #426 for the separate concern of _keeping the
registrar in sync_ going forward — out of scope here.)

### `server/tests/helpers/mod.rs`

One private core owns all router-building + request-driving logic; the public
surface is thin wrappers that delegate. Signatures (names provisional, shape
fixed):

```rust
// The single implementation. Every wrapper flows through here.
async fn post_form_inner(
    state: Arc<storage::AppState>,
    mailer: Arc<dyn common::mailer::MailSender>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
    bearer: Option<&str>,
    user_agent: Option<&str>,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String);

// Canonical case: noop mailer, secure cookies, cookie auth, Set-Cookie dropped.
// Callers: audiences, web_account, web_backup, web_media, web_posts,
//          web_sessions, web_site, web_subscriptions.
pub async fn post_form(
    state: Arc<storage::AppState>, uri: &str, body: impl Into<String>, cookie: Option<&str>,
) -> (StatusCode, String);

// Injects a mailer. Callers: web_email, web_password_reset.
pub async fn post_form_with_mailer(
    state: Arc<storage::AppState>, mailer: Arc<dyn common::mailer::MailSender>,
    uri: &str, body: impl Into<String>, cookie: Option<&str>,
) -> (StatusCode, String);

// Exposes the two knobs web_auth needs: the secure_cookies flag and the returned
// Set-Cookie. Caller: web_auth (its plain post_form sites, renamed).
pub async fn post_form_with_secure_flag(
    state: Arc<storage::AppState>, uri: &str, body: impl Into<String>,
    cookie: Option<&str>, secure_cookies: bool,
) -> (StatusCode, Option<String>, String);

// Adds a User-Agent header. Caller: web_auth.
pub async fn post_form_with_ua(
    state: Arc<storage::AppState>, uri: &str, body: impl Into<String>,
    cookie: Option<&str>, user_agent: &str, secure_cookies: bool,
) -> (StatusCode, Option<String>, String);

// Bearer-token auth instead of a cookie. Caller: web_auth.
pub async fn post_form_with_bearer(
    state: Arc<storage::AppState>, uri: &str, body: impl Into<String>, bearer: &str,
) -> (StatusCode, Option<String>, String);
```

The 3 media server fns are added to `ensure_server_fns_registered()` so
`web_media` uses the plain shared `post_form` with no special-casing.

### Per-file conversion

| File                                                                        | Wrapper it converts to       | Call-site change                                                                    |
| --------------------------------------------------------------------------- | ---------------------------- | ----------------------------------------------------------------------------------- |
| audiences, web_backup, web_posts, web_sessions, web_site, web_subscriptions | `post_form`                  | delete local def + import helper; call sites unchanged                              |
| web_media                                                                   | `post_form`                  | as above **+** its 3 fns move to the shared registrar; drop the inline registration |
| web_account                                                                 | `post_form`                  | drop the dead `Set-Cookie` slot: `(status, _, body)` → `(status, body)`             |
| web_email, web_password_reset                                               | `post_form_with_mailer`      | pass the existing `mailer` arg through the helper                                   |
| web_auth (base)                                                             | `post_form_with_secure_flag` | rename `post_form(` → `post_form_with_secure_flag(`; return type unchanged          |
| web_auth `_with_ua`                                                         | `post_form_with_ua`          | delete local def; call sites unchanged                                              |
| web_auth `_with_bearer`                                                     | `post_form_with_bearer`      | delete local def; call sites unchanged                                              |
| web_tags                                                                    | —                            | already has no copy; untouched                                                      |

## Acceptance criteria

1. **No local copies remain.** `rg 'fn post_form' server/tests/web/*.rs` returns
   zero matches; every definition lives in `server/tests/helpers/mod.rs`.
2. **Single implementation.** `server/tests/helpers/mod.rs` has exactly one
   function that builds the router and drives the request (`post_form_inner`);
   the public wrappers contain no duplicated router/request-building — each only
   sets arguments, delegates to the core, and (for the two 2-tuple wrappers)
   drops the captured `Set-Cookie` element.
3. **Behavior preserved.** Every capability the copies had is reachable through
   the new surface: mailer injection (`post_form_with_mailer`), `secure_cookies`
   toggle and `Set-Cookie` capture (`post_form_with_secure_flag` / `_with_ua`),
   User-Agent (`post_form_with_ua`), bearer auth (`post_form_with_bearer`).
4. **Media fns homed in the registrar.** `ListMyMedia`, `MediaUsage`,
   `DeleteMedia` appear in `ensure_server_fns_registered()`; `web_media.rs`
   contains no inline `register_explicit` for them.
5. **Dead cookie slot removed.** `web_account.rs` no longer destructures an
   ignored `Set-Cookie` element from a `post_form` result.
6. **Green gate, no assertion changes.** `cargo xtask check` passes (host
   integration tests over sqlite + postgres + coverage). No test's assertions
   change — only the helper it calls, the call name, and destructuring shape.
   Test count is unchanged.

## Out of scope

- The **second** `ensure_server_fns_registered()` in `server/src/lib.rs`'s
  `#[cfg(test)]` module — a different crate-internal test module, not part of
  the `server/tests/web` helper surface. Captured in #426.
- Building the **static guard** that prevents future registrar drift — #426.
- Any change to `#[server]` fns, routes, or production code.
