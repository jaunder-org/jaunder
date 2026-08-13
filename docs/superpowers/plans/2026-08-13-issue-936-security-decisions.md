# Issue #936 Security Decisions Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating an individual task through `jaunder-dispatch` when useful). Tick
> each task's checkboxes in real time.

**Spec:** `docs/superpowers/specs/2026-08-13-issue-936-security-decisions.md`
(the problem and decisions live there; this document is the implementation
path).

## Review

**Goal:** Record four security decisions and make explicit `Authorization`
authoritative over ambient browser sessions, retiring any simultaneous session
cookie only after successful header authentication.

**Scope:** In: host credential parsing, shared `AuthUser`/response integration,
optional-auth propagation, four ADRs, the architecture projection, and the
Username glossary term. Out: new authentication schemes, token type-state,
Argon2 parameter changes, session expiry, and username migrations.

**Tasks:**

0. Commit the approved spec and plan.
1. Make credential resolution typed and Authorization-first.
2. Retire ambient cookies after successful explicit authentication.
3. Propagate explicit-auth failures through optional-auth routes.
4. Author and project the four security ADRs; verify unchanged KDF/token guards.

**Key risks/decisions:** Any `Authorization` header suppresses cookie fallback;
unsupported and invalid headers reject. Cookie expiry is requested only after
token authentication and Basic username verification succeed. A shared request
marker plus outer Axum response middleware must append, never replace,
`Set-Cookie`. ADR drafts remain numberless and ignored until ship-time
`cargo xtask adr promote` assigns collision-free numbers.

**Architecture:** `host::auth` owns pure transport resolution and returns typed
provenance. `web::auth::AuthUser` owns authenticated identity and marks a
request-scoped, shared retirement flag only after successful explicit auth.
`server::create_router` installs the flag and appends the expiry cookie after
the handler completes. Optional viewer extraction distinguishes absent
credentials from rejected explicit credentials.

**Tech stack:** Rust 2024, Axum middleware/extensions, Leptos server functions,
`rstest` dual-backend integration tests, Markdown ADRs, `cargo nextest`, and
`cargo xtask`.

## Global Constraints

- Preserve SQLite/PostgreSQL behavior parity; every database-backed integration
  scenario uses `#[apply(backends)]`.
- Any `Authorization` header is authoritative. Unsupported, malformed, invalid,
  or Basic-username-mismatched credentials reject without cookie fallback.
- Expire `session=` only after successful Bearer/Basic authentication; apply to
  all `AuthUser` consumers and append the response header.
- With no `Authorization` header, existing cookie behavior remains unchanged.
- Keep `RawToken` non-sqlx and redacting; do not introduce token type-state.
- Keep Username ASCII `[a-z0-9_-]+` and lowercase-canonical at `FromStr`.
- Do not add lint suppressions. No `Co-Authored-By` trailers.
- For every implementation task: first run its targeted command, then
  `devtool run -- cargo xtask check --no-test` during the fix loop. Before each
  commit, follow `jaunder-commit`: run `devtool run -- cargo xtask check`, stage
  the checked tree, commit, and include the newly checked plan state.
- Do not promote ADR drafts during iteration. `jaunder-ship` promotes them only
  after the final rebase and before push.

---

### Task 0: Commit approved cycle documents

**Files:**

- Add: `docs/superpowers/specs/2026-08-13-issue-936-security-decisions.md`
- Add: `docs/superpowers/plans/2026-08-13-issue-936-security-decisions.md`

**Interfaces:**

- Consumes: User-approved issue #936 specification.
- Produces: Tracked lifecycle artifacts keyed by the load-bearing `issue-936`
  token; later tasks update this plan's checkboxes.

- [x] **Step 1: Verify the approved documents are formatted**

Run:

```bash
devtool run -- prettier -c docs/superpowers/specs/2026-08-13-issue-936-security-decisions.md docs/superpowers/plans/2026-08-13-issue-936-security-decisions.md
```

Expected: PASS; both paths report unchanged.

- [x] **Step 2: Mark Task 0 complete and commit the planning artifacts**

Run the required full pre-commit gate, stage the spec and plan, then commit:

```bash
devtool run -- cargo xtask check
git add docs/superpowers/specs/2026-08-13-issue-936-security-decisions.md docs/superpowers/plans/2026-08-13-issue-936-security-decisions.md
git commit -m "docs: approve security decision design (#936)"
```

Expected: gate PASS; commit contains only the approved spec and plan, with Task
0 checked.

---

### Task 1: Typed Authorization-first credential resolution

**Files:**

- Modify: `host/src/auth.rs:23-76,98-168`
- Modify: `web/src/auth/server.rs:24-85,166-184,305-409`

**Interfaces:**

- Produces in `host::auth`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialTransport {
    Cookie,
    Bearer,
    Basic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialResolutionError {
    Missing,
    InvalidAuthorization,
}

#[derive(Debug)]
pub struct Credential {
    pub token: RawToken,
    pub expected_username: Option<Username>,
    pub transport: CredentialTransport,
    pub session_cookie_present: bool,
}

pub fn resolve_credential(
    headers: &http::HeaderMap,
) -> Result<Credential, CredentialResolutionError>;
```

- Updates `web::auth::AuthRejection` with a distinct invalid-Authorization
  variant and carries `CredentialTransport` on post-resolution failures (missing
  session storage and session authentication). Missing and invalid credentials
  both project to unauthorized, while Task 3 can distinguish an explicit-header
  failure from a cookie-only miss.
- Task 2 consumes `CredentialTransport` and `session_cookie_present` after
  authentication succeeds.

- [x] **Step 1: Replace the host resolver tests with the full precedence
      contract**

Keep the existing cookie/Bearer/Basic happy-path and cookie-header construction
tests. Add or reshape resolver tests to pin every branch:

```rust
#[test]
fn authorization_precedes_cookie_and_reports_cookie_presence() {
    let mut headers = headers_with(http::header::COOKIE, "session=cookie-token");
    headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_static("Bearer bearer-token"),
    );

    let credential = resolve_credential(&headers).unwrap();
    assert_eq!(credential.token, "bearer-token");
    assert_eq!(credential.transport, CredentialTransport::Bearer);
    assert!(credential.session_cookie_present);
}

#[test]
fn basic_reports_transport_and_canonical_expected_username() {
    let headers = headers_with(
        http::header::AUTHORIZATION,
        "Basic QWxpY2U6dG9rMTIz", // Alice:tok123
    );
    let credential = resolve_credential(&headers).unwrap();
    assert_eq!(credential.transport, CredentialTransport::Basic);
    assert_eq!(credential.expected_username.as_deref(), Some("alice"));
}

#[test]
fn cookie_is_used_only_without_authorization() {
    let headers = headers_with(http::header::COOKIE, "session=cookie-token");
    let credential = resolve_credential(&headers).unwrap();
    assert_eq!(credential.transport, CredentialTransport::Cookie);
    assert!(credential.session_cookie_present);
}

#[test]
fn any_bad_authorization_rejects_instead_of_falling_back_to_cookie() {
    for value in [
        "Negotiate xyz",
        "Bearer has space",
        "Basic !!!notbase64!!!",
    ] {
        let mut headers = headers_with(http::header::COOKIE, "session=cookie-token");
        headers.insert(http::header::AUTHORIZATION, value.parse().unwrap());
        assert!(matches!(
            resolve_credential(&headers),
            Err(CredentialResolutionError::InvalidAuthorization)
        ));
    }
}

#[test]
fn missing_or_invalid_cookie_without_authorization_remains_missing() {
    assert!(matches!(
        resolve_credential(&http::HeaderMap::new()),
        Err(CredentialResolutionError::Missing)
    ));
    let headers = headers_with(http::header::COOKIE, "session=");
    assert!(matches!(
        resolve_credential(&headers),
        Err(CredentialResolutionError::Missing)
    ));
}
```

- [x] **Step 2: Run the host tests and verify the old API fails**

```bash
devtool run -- cargo nextest run -p host resolve_credential
```

Expected: FAIL because the typed result, transport, and error variants do not
exist and the resolver still prefers cookies.

- [x] **Step 3: Implement the pure resolver and migrate `AuthUser`**

Resolve `Authorization` by header presence before inspecting cookies. Bearer and
Basic parsing failures, invalid header bytes, and every unsupported scheme
return `InvalidAuthorization`; only absence of the header permits cookie
resolution. Detect `session=` presence independently of token validity so a
later successful explicit credential can request retirement.

In `web/src/auth/server.rs`, map `CredentialResolutionError::Missing` and
`InvalidAuthorization` to distinct `AuthRejection` variants, preserve the
existing session lookup and Basic username check, and extend the response/error
projection tests so both variants produce the repository's unauthorized public
shape.

- [x] **Step 4: Run targeted tests, then the fast gate**

```bash
devtool run -- cargo nextest run -p host resolve_credential
devtool run -- cargo nextest run -p web --features server auth_rejection
devtool run -- cargo xtask check --no-test
```

Expected: all PASS. Existing no-header cookie tests remain green; unsupported
Authorization now reaches unauthorized rather than `MissingToken` or cookie
authentication.

- [x] **Step 5: Commit the typed resolution cutover**

After the required full gate, stage `host/src/auth.rs`,
`web/src/auth/server.rs`, and the checked plan:

```bash
git commit -m "fix(auth): prefer explicit authorization (#936)"
```

Expected: one compiling clean-cutover commit; no old `Option<Credential>` caller
remains.

---

### Task 2: Shared successful-auth cookie retirement

**Files:**

- Modify: `web/src/auth/server.rs:24-85`
- Modify: `web/src/auth/mod.rs:51-55`
- Modify: `server/src/lib.rs:29-124`
- Modify: `server/tests/helpers/mod.rs:415-633`
- Modify: `server/tests/web/web_sessions.rs`
- Modify: `server/tests/web/web_auth.rs:750-820`
- Modify: `server/tests/atompub/atompub_service.rs`

**Interfaces:**

- Produces in `web::auth`:

```rust
#[derive(Clone, Default)]
pub struct SessionCookieRetirement(Arc<AtomicBool>);

impl SessionCookieRetirement {
    pub fn request(&self);
    pub fn requested(&self) -> bool;
}
```

The marker is re-exported from `web::auth`. `AuthUser` requests retirement only
when `transport` is Bearer/Basic, `session_cookie_present` is true, session
lookup succeeded, and Basic username verification (when applicable) succeeded.

- Produces a router middleware in `server/src/lib.rs` that inserts one marker,
  runs `Next`, and appends `host::auth::clear_session_cookie_header(secure)`
  when `requested()` is true.
- Produces test-only helpers accepting cookie and Authorization independently
  and returning all response `Set-Cookie` values via `HeaderMap::get_all`.
- Task 3 reuses those helpers and the same marker on optional-auth routes.

- [x] **Step 1: Add failing dual-credential integration tests**

In `server/tests/helpers/mod.rs`, add a response carrier and mixed-auth helpers
without changing existing wrapper signatures:

```rust
pub struct TestHttpResponse {
    pub status: StatusCode,
    pub set_cookies: Vec<String>,
    pub body: String,
}

pub async fn post_form_with_credentials(
    state: &Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
    authorization: Option<&str>,
    secure_cookies: bool,
) -> TestHttpResponse;

pub async fn post_json_with_credentials(
    state: &Arc<storage::AppState>,
    uri: &str,
    body: serde_json::Value,
    cookie: Option<&str>,
    authorization: Option<&str>,
    secure_cookies: bool,
) -> TestHttpResponse;
```

Use `web::sessions::List` for the Leptos protected-route contract:

```rust
#[apply(backends)]
#[tokio::test]
async fn bearer_identity_wins_and_expires_simultaneous_cookie(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _ } = backend.setup().await;
    let cookie_user = create_user_and_session(&state).await;
    let bearer_user = create_user_and_session(&state).await;
    let response = post_form_with_credentials(
        &state,
        <web::sessions::List as ServerFn>::PATH,
        "",
        Some(&cookie_user.cookie()),
        Some(&format!("Bearer {}", bearer_user.token)),
        true,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    let sessions: Vec<web::sessions::Info> = serde_json::from_str(&response.body).unwrap();
    let current = sessions.iter().find(|session| session.is_current).unwrap();
    assert_eq!(current.token_hash, host::token::hash(&bearer_user.token));
    assert!(response.set_cookies.iter().any(|value| {
        value == "session=; HttpOnly; SameSite=Lax; Path=/; Secure; Max-Age=0"
    }));
}
```

Add neighboring cases proving: the same token in cookie and Bearer still expires
the cookie; syntactically valid but unknown Bearer, malformed Bearer, and
unsupported scheme reject without an expiry cookie; and Bearer-only success
emits no expiry cookie.

Use the existing logout server function to prove append semantics: with both
credentials present, the handler's own clear cookie and the middleware's clear
cookie must both remain in `get_all(SET_COOKIE)` (two values), rather than one
replacing the other.

For raw Axum/AtomPub coverage, send Bob's Basic credentials plus Alice's cookie
to `/atompub/service`; assert the returned service document names Bob's
namespace and the insecure test router appends the non-`Secure` expiry cookie.
Add a Basic-username mismatch plus valid cookie case asserting `401` and no
expiry cookie.

- [x] **Step 2: Run the integration filters and verify failure**

```bash
devtool run -- cargo nextest run -p jaunder --test integration bearer_identity_wins
devtool run -- cargo nextest run -p jaunder --test integration explicit_basic_identity
devtool run -- cargo nextest run -p jaunder --test integration explicit_auth_set_cookie
```

Expected: FAIL because no marker/middleware exists, the cookie is not retired,
and the test helper cannot yet observe all `Set-Cookie` values.

- [x] **Step 3: Implement the marker and outer response middleware**

Insert one `SessionCookieRetirement` into request extensions before `Next`.
After the response returns, append the generated expiry cookie only when the
shared marker was requested. Use Axum `HeaderMap::append`; never `insert`.
Install the middleware once around the complete router in `create_router`, with
the existing `secure_cookies` argument as its state/configuration.

At the successful end of `AuthUser::from_request_parts`, after
`verify_basic_username`, request retirement for Bearer/Basic plus simultaneous
`session=`. Failed lookup and mismatch paths return before touching the marker.
Keep the extractor usable in contexts without the router marker by treating an
absent marker as no response work, not as authentication failure.

- [x] **Step 4: Run the complete auth integration slice and fast gate**

```bash
devtool run -- cargo nextest run -p jaunder --test integration web_sessions
devtool run -- cargo nextest run -p jaunder --test integration web_auth
devtool run -- cargo nextest run -p jaunder --test integration atompub_service
devtool run -- cargo xtask check --no-test
```

Expected: PASS for both backends. Header identity wins, success retires the
cookie with the configured `Secure` attribute, failures never retire it, and
pre-existing `Set-Cookie` values remain.

- [x] **Step 5: Commit shared cookie retirement**

After the full gate, stage the auth/router/test-helper/tests and checked plan:

```bash
git commit -m "fix(auth): retire ambient session cookies (#936)"
```

Expected: one route-independent middleware commit covering Leptos and raw Axum
handlers.

---

### Task 3: Optional-auth rejection and identity propagation

**Files:**

- Modify: `web/src/viewer.rs:19-41`
- Modify: `web/src/posts/api.rs` (all `viewer_identity().await` callers)
- Modify: `web/src/timeline/api.rs:37-127`
- Modify: `server/tests/web/web_posts.rs:735-765,2488-2611`

**Interfaces:**

- Changes:

```rust
pub async fn viewer_identity() -> InternalResult<ViewerIdentity>;
```

- `AuthRejection::MissingToken` and cookie-transport
  `SessionAuthError::{InvalidToken, SessionNotFound}` map to
  `Ok(ViewerIdentity::Anonymous)`. Invalid Authorization and every Bearer/Basic
  post-resolution failure map through the existing `auth_rejection_error` into
  `Err(InternalError)`.
- Every posts/timeline caller consumes the fallible result with `.await?`; no
  caller may restore catch-all anonymous fallback.

- [ ] **Step 1: Extend the viewer-sensitive timeline test with explicit auth**

Extend `local_timeline_enforces_visibility_for_viewer`, which already creates
Public, Subscribers, Named, and Private posts. Add a subscriber Bearer session
and an unrelated valid cookie, then call `ListLocalTimeline` through
`post_json_with_credentials`:

```rust
let subscriber_session = create_session_for(&state, subscriber).await;
let stranger_session = create_session_for(&state, stranger).await;
let response = post_json_with_credentials(
    &state,
    <web::timeline::ListLocalTimeline as ServerFn>::PATH,
    serde_json::json!({ "cursor": null, "limit": 50 }),
    Some(&stranger_session.cookie()),
    Some(&format!("Bearer {}", subscriber_session.token)),
    true,
)
.await;
assert_eq!(response.status, StatusCode::OK);
let page: TimelinePage = serde_json::from_str(&response.body).unwrap();
assert_eq!(
    slugs(&page),
    [
        public.slug.to_string(),
        subscribers.slug.to_string(),
        named.slug.to_string(),
    ]
    .into_iter()
    .collect()
);
assert!(response.set_cookies.iter().any(|value| value.contains("Max-Age=0")));
```

Add a second request to the same optional-auth endpoint with a valid subscriber
cookie plus `Authorization: Bearer unknown-token`; assert the server function
rejects (non-OK unauthorized body), returns no timeline payload, and emits no
expiry cookie. This distinguishes “credential absent” from “explicit credential
failed.”

- [ ] **Step 2: Run the optional-auth test and verify failure**

```bash
devtool run -- cargo nextest run -p jaunder --test integration local_timeline_enforces_visibility_for_viewer
```

Expected: FAIL because `viewer_identity` currently converts every `AuthUser`
rejection into `Anonymous`.

- [ ] **Step 3: Make viewer extraction fallible and migrate every caller**

Expose the existing `auth_rejection_error` within the `web` crate. In
`viewer_identity`, preserve anonymous fallback for `AuthRejection::MissingToken`
and cookie-transport `InvalidToken`/`SessionNotFound`; return the local viewer
on success; propagate invalid Authorization and all Bearer/Basic failures.
Update all LSP-reported call sites in `web/src/posts/api.rs` and
`web/src/timeline/api.rs` to use `.await?`; preserve their existing not-found
masking and authenticated-route behavior.

Before editing the exported function, re-run LSP references on
`web/src/viewer.rs::viewer_identity`; the expected set is the current 13
posts/timeline call sites plus the declaration. Migrate the complete returned
set.

- [ ] **Step 4: Run affected tests and fast gate**

```bash
devtool run -- cargo nextest run -p jaunder --test integration local_timeline_enforces_visibility_for_viewer
devtool run -- cargo nextest run -p jaunder --test integration web_posts
devtool run -- cargo nextest run -p web
devtool run -- cargo xtask check --no-test
```

Expected: PASS. Absent credentials remain anonymous; valid explicit credentials
select the authenticated viewer and retire the cookie; present failed explicit
credentials reject.

- [ ] **Step 5: Commit optional-auth propagation**

After the full gate, stage the viewer/callsites/test and checked plan:

```bash
git commit -m "fix(auth): reject invalid optional credentials (#936)"
```

Expected: no infallible `viewer_identity().await` call remains.

---

### Task 4: Four ADRs, architecture/domain projections, and invariant proof

**Files:**

- Create, ignored until ship:
  `docs/adr/drafts/test-only-cheap-kdf-fails-closed.md`
- Create, ignored until ship:
  `docs/adr/drafts/hash-bearer-tokens-before-persistence.md`
- Create, ignored until ship:
  `docs/adr/drafts/explicit-authorization-replaces-session-cookie.md`
- Create, ignored until ship: `docs/adr/drafts/lowercase-canonical-usernames.md`
- Modify: `docs/ARCHITECTURE.md:621-743,2416-2440`
- Modify: `CONTEXT.md:9-27,74-84`

**Interfaces:**

- Each draft uses `# ADR-DRAFT: <Title>`, `- Status: proposed`, date
  `2026-08-13`, and issue #936.
- Architecture citations use descriptive link text and exact draft-path targets
  `adr/drafts/<slug>.md`; ship-time promotion rewrites targets and moves files.
- `CONTEXT.md` produces the canonical term:

```markdown
**Username**: A case-insensitive local account identifier accepted as ASCII
`[a-z0-9_-]+`. Input is normalized to lowercase; that canonical form is stored,
compared, serialized, displayed, and used in URLs. _Avoid_: preserving case as a
second username identity or pre-normalizing outside the Username boundary.
```

- [ ] **Step 1: Author the four numberless ADR drafts and project them online**

Copy `docs/adr/template.md` four times, then write one decision per draft:

1. **Test-only cheap KDF fails closed** — record production dependency-edge
   isolation, the `!debug_assertions` compile error, startup exit before CLI,
   why compile/startup are complementary, and PHC-derived verification.
2. **Hash bearer-equivalent tokens before persistence** — record SHA-256
   `TokenHash` persistence, `RawToken`'s missing sqlx bridge and redacting
   debug, explicit hash-before-store conversion, the declined #554 type-state
   design, and the honest limit that call sites still choose the conversion.
3. **Explicit Authorization replaces ambient session state** — record any-header
   authority, rejection without fallback, Bearer/Basic success semantics,
   post-success cookie retirement across all routes, append semantics, and
   optional-auth absent-vs-invalid handling.
4. **Lowercase-canonical usernames** — record accepted ASCII grammar, mixed-case
   ingress, single `FromStr` normalization, lowercase persistence and wire/URL
   form, direct equality, and the rejected Unicode/case-preserving alternatives.

In `docs/ARCHITECTURE.md`, replace the old cookie-first paragraph with the
implemented Authorization-first behavior and cite draft 3. Add draft citations
to the existing token-storage, cheap-KDF, and validating-newtype/Username prose.
Delete the four #936 bullets from **Un-ADR'd reality** and change its issue-list
intro to only the still-open #937/#938 sets. Add **Username** to `CONTEXT.md`
and the User→Username relationship; do not put Rust implementation detail in the
glossary.

- [ ] **Step 2: Verify the unchanged cheap-KDF and token safeguards**

Run existing behavioral tests:

```bash
devtool run -- cargo nextest run -p jaunder --test integration jaunder_binary_fail_closes_under_cheap_kdf_build
devtool run -- cargo nextest run -p common raw_token_debug_redacts_body
devtool run -- cargo nextest run -p macros str_newtype_no_sqlx_omits_the_bridge
```

Expected: PASS.

Then prove both compile-time sides:

```bash
devtool run -- cargo check -p jaunder --release
devtool run -- cargo check -p common --features cheap-kdf --release
```

Expected: the normal release check PASSes; the cheap-KDF release check FAILs
specifically with `cheap-kdf must not be enabled in a release/optimized build`.
The second non-zero exit is the expected proof, not a gate failure to suppress.

- [ ] **Step 3: Check documentation and links locally**

```bash
devtool run -- prettier -c CONTEXT.md docs/ARCHITECTURE.md docs/adr/drafts/test-only-cheap-kdf-fails-closed.md docs/adr/drafts/hash-bearer-tokens-before-persistence.md docs/adr/drafts/explicit-authorization-replaces-session-cookie.md docs/adr/drafts/lowercase-canonical-usernames.md
devtool run -- cargo xtask check --no-test
```

Expected: PASS while the draft files exist in the local ignored pen. Do not push
this intermediate state: tracked architecture links to ignored drafts are valid
locally but would be dead in a clean clone until promotion.

- [ ] **Step 4: Commit the tracked documentation projection**

After the required full gate, stage only `CONTEXT.md`, `docs/ARCHITECTURE.md`,
and the checked plan; drafts intentionally remain ignored and uncommitted:

```bash
git commit -m "docs(adr): record security invariants (#936)"
```

Expected: tracked docs cite all four local draft paths; **Un-ADR'd reality** has
no #936 entry. The four drafts remain under `docs/adr/drafts/` for
`jaunder-ship`.

- [ ] **Step 5: Preserve ship-time promotion prerequisites**

Confirm all four ignored drafts exist in this checkout and no duplicate copies
exist in another checkout used for this issue. Do not run promotion yet.
`jaunder-ship` must, after rebasing onto current `main`, run:

```bash
devtool run -- cargo xtask adr promote
```

Expected at ship: four numbered accepted ADRs, rewritten architecture citations,
generated `docs/README.md` rows, and staged promotion output ready for the ship
commit.
