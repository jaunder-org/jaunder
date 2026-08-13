# Request-Aggregate Server-Function Inputs Implementation Plan

> **For agentic workers:** Execute task-by-task with `jaunder-iterate`; use
> `jaunder-dispatch` for a single task when useful. Tick checkboxes in real
> time.

**Goal:** Replace all current cohesive multi-field `ActionForm` submissions with
typed request aggregates assembled from parsed client fields.

**Architecture:** Aggregates live in their vertical's `api.rs` and become the
sole caller-supplied server-function parameter. Native `<form>` handlers retain
browser semantics while dispatching generated `ServerAction` inputs directly.
Login is a hard tracer gate before any other migration.

**Tech Stack:** Rust 2024, Leptos 0.8, serde/serde_qs, axum, rstest dual-backend
fixtures, Playwright, cargo xtask.

## Review

**Scope:** Seven aggregates and eight rendered UI paths from the
[approved spec](../specs/2026-08-12-issue-417-request-aggregate-types.md), plus
their tests and decision docs. Excludes single-value forms, endpoint/response/
storage behavior changes, progressive enhancement, and a cohesion gate.

**Tasks:** Login tracer; registration; invite creation; password-reset
confirmation; audience rename/membership; media ordinary/force deletion; final
audit/docs/full gate.

**Risks:** Nested PostUrl spelling; typed-secret boundaries; pending duplicate
dispatch; optional-value loss; scoped invalidation; ignored ADR draft must be
promoted before push.

## Global Constraints

- Exact names/fields/types come from the spec Scope table. Never flatten an
  ADR-0063 domain type to a primitive.
- Aggregates contain caller values only—never auth, cookies, headers, or DI.
- Every native form calls `prevent_default`, refuses invalid/pending input,
  dispatches parsed values once, retains Enter submission, and disables submit
  while pending.
- Preserve endpoints, responses, copy, confirmation prompts, redirects, error
  rendering, storage semantics, and invalidation scope.
- Integration tests use `#[apply(backends)]` + `backend.setup().await`; never
  `sqlite::memory:` or a hand-rolled pool.
- Every browser contract is run RED before its component implementation, then
  GREEN afterward. Existing tests count as RED only when the signature/wire
  change makes them fail for the intended reason.
- **Every task's commit sequence is mandatory:** stage exact task paths with
  `git add`; inspect `git diff --cached`; ensure no unrelated unstaged changes
  because the gate checks the whole working tree; run
  `devtool run -- cargo xtask check`; if formatting changes anything, re-stage,
  inspect, and rerun; commit the unchanged checked tree. Never use a commit
  pathspec or `Co-Authored-By`.

---

### Task 1: Login tracer bullet

**Files:** `web/src/auth/api.rs`, `web/src/auth/component.rs`,
`web/src/auth/mod.rs`, `server/tests/web/web_auth.rs`,
`end2end/tests/auth.spec.ts`, `xtask/src/steps/proffered_secret_check.rs`,
`docs/ARCHITECTURE.md`, approved spec, this plan. Keep
`docs/adr/drafts/request-aggregate-server-function-inputs.md` ignored.

**Produces:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: Username,
    pub password: ProfferedPassword,
    pub label: Option<SessionLabel>,
}
#[macros::server(skip_all)]
pub async fn login(request: LoginRequest) -> WebResult<LoginResponse>;
```

- [x] Add backend tests `login_nested_request_maps_distinct_fields`,
      `login_nested_request_without_label_uses_user_agent`,
      `login_nested_request_rejects_invalid_username_before_handler`, and
      `login_nested_request_rejects_short_password_before_handler`. Post
      `request[username]`, `request[password]`, optional `request[label]`; use
      distinct sentinels. Assert exact saved label or UA fallback. Invalid cases
      assert HTTP 500 + `server_function` and unchanged session count.
- [x] Run
      `devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_auth`;
      expect RED on the old signature, not setup failure.
- [x] Add `LoginRequest`, destructure once at handler entry, preserve all body
      branches, update Rust callers, rerun the integration command; expect GREEN
      on both backends.
- [x] Add Playwright tests `login submits with Enter`,
      `login invalid fields do not dispatch`, and
      `login pending state prevents duplicate dispatch`. The invalid test counts
      `/api/auth/login` requests (zero) and observes touched username/password
      errors. The pending test delays the route, submits once, attempts Enter
      while pending, observes disabled submit, releases the response, and
      asserts exactly one request. Retain existing success, wrong-password,
      marker, and pushState tests.
- [x] Run `devtool run -- cargo xtask e2e-local auth.spec.ts`; expect RED
      because `ActionForm` does not meet the new interaction contract.
- [x] Replace login `ActionForm` with `<form on:submit>`, use
      `Field<ProfferedPassword>`, gate `parsed()`/pending, dispatch
      `Login { request: LoginRequest { ..., label: None } }`, and read
      successful username from `input.request.username`.
- [x] Rerun auth Playwright; expect GREEN. Execute the mandatory stage/check
      sequence and commit with this exact evidence-bearing message:

```bash
git commit -m 'feat(web): prove request-aggregate login' -m $'Proof:\n- devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_auth\n- devtool run -- cargo xtask e2e-local auth.spec.ts\n- devtool run -- cargo xtask check'
```

Do not begin Task 2 unless all three commands passed and this tracer is
committed.

### Task 2: Registration request

**Files:** `web/src/registration/api.rs`, `web/src/registration/component.rs`,
`web/src/registration/mod.rs`, `server/tests/web/web_auth.rs`,
`end2end/tests/auth.spec.ts`, `end2end/tests/invite.spec.ts`.

**Produces:**

```rust
pub struct RegistrationRequest {
    pub username: Username,
    pub password: ProfferedPassword,
    pub invite_code: Option<ProfferedInviteCode>,
}
pub async fn register(request: RegistrationRequest) -> WebResult<()>;
```

Derive `Debug, Clone, Serialize, Deserialize`.

- [x] Adapt backend registration posts to nested keys. Add
      `register_nested_request_maps_open_fields` (distinct username/password,
      `invite_code: None`) and `register_nested_request_maps_invite_code` (exact
      `Some` code is redeemed). Assert created username, password
      authentication, session cookie, and invite redemption—not only status.
- [x] Run
      `devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_auth`;
      expect RED.
- [x] Implement the aggregate/handler and update Rust callers; rerun; expect
      GREEN on both backends.
- [x] Add Playwright tests `register pending state prevents duplicate dispatch`,
      `register invalid fields do not dispatch`, and
      `register server failure renders error`; delay/count
      `/api/registration/register`, assert disabled + exactly one request after
      a second Enter, and zero requests plus touched errors for invalid fields.
      Existing `register with open policy succeeds` and invite-link registration
      remain the success/redirect/session/hidden-code oracles. The failure test
      intercepts the endpoint, asserts the visible error, and asserts no
      navigation/session establishment.
- [x] Run `devtool run -- cargo xtask e2e-local auth.spec.ts` and
      `devtool run -- cargo xtask e2e-local invite.spec.ts`; expect RED.
- [x] Replace registration `ActionForm`, use `Field<ProfferedPassword>`, parse
      the query invite code into `Option<ProfferedInviteCode>` before dispatch,
      and preserve policy guidance/error/redirect/session behavior. Rerun both
      specs; expect GREEN.
- [x] Mandatory stage/check sequence; commit
      `feat(web): aggregate registration requests`.

### Task 3: Invite-creation request

**Files:** `web/src/invites/api.rs`, `web/src/invites/component.rs`,
`web/src/invites/mod.rs`, `server/tests/web/web_account.rs`,
`end2end/tests/invite.spec.ts`.

**Produces:**

```rust
pub struct CreateInviteRequest {
    pub expires_in_hours: Option<InviteTtlHours>,
    pub recipient_email: Email,
}
pub async fn create(request: CreateInviteRequest) -> WebResult<()>;
```

Derive `Debug, Clone, Serialize, Deserialize`.

- [x] Adapt invite posts to nested keys. Add/retain named cases proving exact
      recipient plus `expires_in_hours: Some(37)` in the emailed/stored invite,
      and both omitted and empty hours map to `None`/the 168-hour default. Keep
      malformed email/expiry decode rejection and assert no invite/email side
      effect.
- [x] Run
      `devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_account`;
      expect RED.
- [x] Implement aggregate/handler/callers; rerun; expect GREEN on both backends.
- [x] Add Playwright `invite creation pending prevents duplicate dispatch`:
      delay/count `/api/invites/create`, assert disabled, second Enter produces
      no second request, release, observe success message and exactly one
      `/api/invites/list` refresh. Existing main-flow test remains the
      email/list success oracle; extend it to exercise a non-default expiry
      value.
- [x] Add Playwright `invite creation server failure renders error`: intercept
      `/api/invites/create`, assert the visible error, no success message, and
      zero `/api/invites/list` refreshes.
- [x] Add Playwright `invite creation invalid fields do not dispatch`: submit
      invalid email and out-of-range expiry values, assert both touched field
      errors, and count zero `/api/invites/create` requests.
- [x] Run `devtool run -- cargo xtask e2e-local invite.spec.ts`; expect RED.
- [x] Replace create `ActionForm` with typed manual dispatch; preserve email and
      optional-expiry validation, outcome rendering, and list invalidation.
      Rerun; expect GREEN.
- [x] Mandatory stage/check sequence; commit
      `feat(web): aggregate invite creation requests`.

### Task 4: Password-reset confirmation request

**Files:** `web/src/password_reset/api.rs`,
`web/src/password_reset/component.rs`, `web/src/password_reset/mod.rs`,
`server/tests/web/web_password_reset.rs`,
`end2end/tests/password_reset.spec.ts`.

**Produces:**

```rust
pub struct ConfirmPasswordResetRequest {
    pub token: RawToken,
    pub new_password: ProfferedPassword,
}
pub async fn confirm(request: ConfirmPasswordResetRequest) -> WebResult<()>;
```

Derive `Debug, Clone, Serialize, Deserialize`.

- [ ] Adapt confirmation posts to nested keys. Add
      `confirm_nested_request_maps_token_and_password`; use distinct sentinels
      and assert the token is consumed, new password authenticates, and old
      sessions are revoked. Retain malformed token and short password HTTP 500 +
      `server_function` cases with unchanged password/session state.
- [ ] Run
      `devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_password_reset`;
      expect RED, then implement aggregate/handler and rerun; expect GREEN.
- [ ] Add Playwright tests `reset confirmation invalid input does not dispatch`
      and `reset confirmation pending prevents duplicate dispatch`; assert
      parsed query-token mapping, zero request on invalid token/password,
      touched error, delayed response disabled state, one request after second
      Enter, error rendering, and success redirect. Keep the existing end-to-end
      new-password login assertion.
- [ ] Run `devtool run -- cargo xtask e2e-local password_reset.spec.ts`; expect
      RED.
- [ ] Replace confirm `ActionForm`, parse query token to `RawToken`, use
      `Field<ProfferedPassword>`, preserve error/redirect behavior; rerun;
      expect GREEN.
- [ ] Mandatory stage/check sequence; commit
      `feat(web): aggregate password reset requests`.

### Task 5: Audience rename and membership requests

**Files:** `web/src/audiences/api.rs`, `web/src/audiences/component.rs`,
`web/src/audiences/mod.rs`, `server/tests/web/audiences.rs`,
`end2end/tests/audiences.spec.ts`.

**Produces:**

```rust
pub struct RenameAudienceRequest {
    pub audience_id: AudienceId,
    pub name: AudienceName,
}
pub struct AudienceMembershipRequest {
    pub audience_id: AudienceId,
    pub subscription_id: SubscriptionId,
}
pub async fn rename(request: RenameAudienceRequest) -> WebResult<()>;
pub async fn add_subscriber(request: AudienceMembershipRequest) -> WebResult<()>;
pub async fn remove_subscriber(request: AudienceMembershipRequest) -> WebResult<()>;
```

Both derive `Debug, Clone, Serialize, Deserialize`.

- [ ] Adapt nested backend inputs. Add `rename_nested_request_maps_id_and_name`,
      `add_subscriber_nested_request_maps_both_ids`, and
      `remove_subscriber_nested_request_maps_both_ids`; use different IDs,
      assert exact renamed row/member set, and retain blank-name decode
      rejection.
- [ ] Run
      `devtool run -- devtool pg run -- cargo nextest run -p jaunder web::audiences`;
      expect RED. Implement API aggregates/destructuring/callers; rerun; expect
      GREEN.
- [ ] Add Playwright tests `audience rename pending and error preserve the row`,
      `audience add pending prevents duplicate dispatch`, and
      `audience remove pending prevents duplicate dispatch`. For each mutation,
      delay/count its endpoint, assert disabled + one request after second
      Enter, inject one failure to observe error rendering, then success. Rename
      success must produce exactly one `list_mine` refresh. Add/remove success
      must produce exactly one target audience `list_members` refresh, zero
      `list_mine` refresh, and zero unrelated-audience `list_members` refresh.
      Existing blank-name test remains validation/zero-dispatch proof.
- [ ] Run `devtool run -- cargo xtask e2e-local audiences.spec.ts`; expect RED.
      Replace rename/add/remove `ActionForm`s with typed native forms while
      leaving audience create/delete direct. Rerun; expect GREEN with scoped
      counters.
- [ ] Mandatory stage/check sequence; commit
      `feat(web): aggregate audience mutation requests`.

### Task 6: Media deletion request

**Files:** `web/src/media/api.rs`, `web/src/media/component.rs`,
`web/src/media/mod.rs`, `server/tests/web/web_media.rs`,
`end2end/tests/media.spec.ts`.

**Produces:**

```rust
pub struct DeleteMediaRequest {
    pub sha256: ContentHash,
    pub filename: Filename,
    pub source: MediaSource,
    pub force: Option<bool>,
}
pub async fn delete(request: DeleteMediaRequest) -> WebResult<DeleteResult>;
```

Derive `Debug, Clone, Serialize, Deserialize`.

- [ ] Adapt delete inputs to nested fields. Add
      `delete_nested_request_maps_identity_without_force` and
      `delete_nested_request_maps_identity_with_force`; distinct hash/filename/
      source sentinels prove mapping. Omitted force refuses referenced deletion;
      `Some(true)` deletes the same item. Retain malformed typed-field
      rejection.
- [ ] Run
      `devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_media`;
      expect RED. Implement the aggregate/handler/callers; rerun; expect GREEN.
- [ ] Split/extend Playwright coverage with
      `ordinary media delete confirms and removes unreferenced item`,
      `ordinary media delete confirms and refuses referenced item`, and
      `forced media delete confirms and cannot double dispatch`. Observe the
      ordinary success using `force: None`, exact ordinary confirmation, one
      list refresh, and row removal. Both ordinary tests delay/count the delete,
      assert disabled state, attempt a second click or Enter, and observe
      exactly one request; the forced test independently proves the same
      pending/dedup contract. Also observe distinct confirmation text, refusal
      rendering without list removal, forced success/error rendering, and
      exactly one list refresh/removal on success.
- [ ] Run `devtool run -- cargo xtask e2e-local media.spec.ts`; expect RED.
      Replace both delete `ActionForm`s with native dispatch (`force: None` and
      `Some(true)`), preserving target tracking and confirmations. Rerun; expect
      GREEN.
- [ ] Mandatory stage/check sequence; commit
      `feat(web): aggregate media deletion requests`.

### Task 7: Population audit, architecture truth, and full gate

**Files:** `docs/ARCHITECTURE.md`, comments/docs found by audit, this plan. ADR
draft remains ignored until `jaunder-ship` promotion.

**Consumes:** All six migration commits and the approved spec.

- [ ] Search every `ActionForm` and `#[macros::server]` signature. Verify the
      seven aggregate shapes exist; audience add/remove share one type; media's
      two renderings share one type; enumerated audience create/delete, email
      request, password-reset request, post publish/delete, and
      subscribe/unsubscribe remain direct. Investigate every other multi-arg
      server fn and record why it is independent or already aggregate-shaped.
- [ ] Update stale comments describing positional args/string harvesting. Change
      the issue-417 `Committed direction` paragraph in `docs/ARCHITECTURE.md` to
      present-tense current truth. Do not number the ADR.
- [ ] Run these exact commands; expect PASS:

```bash
devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_auth
devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_account
devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_password_reset
devtool run -- devtool pg run -- cargo nextest run -p jaunder web::audiences
devtool run -- devtool pg run -- cargo nextest run -p jaunder web::web_media
devtool run -- cargo xtask e2e-local auth.spec.ts
devtool run -- cargo xtask e2e-local invite.spec.ts
devtool run -- cargo xtask e2e-local password_reset.spec.ts
devtool run -- cargo xtask e2e-local audiences.spec.ts
devtool run -- cargo xtask e2e-local media.spec.ts
```

- [ ] Run `devtool run -- cargo xtask validate`; expect PASS across static,
      coverage, SQLite/PostgreSQL, and Chromium/Firefox.
- [ ] Mandatory stage/check sequence; commit docs/audit corrections as
      `docs: record request-aggregate boundary`.
- [ ] Hand off to `jaunder-ship`: final review/rebase, promote the draft ADR
      (rewriting/staging its architecture citation), gate, push/open/monitor PR,
      and halt before merge.
