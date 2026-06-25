# Plan: Reorganizing Leptos Server Functions into the Server Submodule Pattern

> **Status: COMPLETE** — shipped. The pattern this plan delivered is codified in
> [ADR-0013: Server Submodule Pattern](../adr/0013-server-submodule-pattern.md)
> (accepted 2026-05-23). Archived under issue #39.

This document outlines our plan to restructure the files in `web/src/` to eliminate, centralize, and minimize the abundant use of `#[cfg(feature = "ssr")]` annotations.

---

## Architectural Goal

Currently, the web-layer modules mix three distinct concerns in a single flat file:
1. **The Seam (The External Interface)**: `#[server]` functions and shared serialization structures (DTOs like `ProfileData`) compiled on both the WASM client and the SSR server.
2. **The Server Implementation**: DB queries, Axum context extractions, and cryptography.
3. **The Web Dependencies**: Server-only imports (such as `axum`, `storage`, `chrono`, etc.) that cannot be compiled under WASM.

By restructuring each feature into a directory module, we declare `#[cfg(feature = "ssr")] mod server;` at the module level. This completely compiles the `server.rs` file conditionally, ensuring **no inline `#[cfg(feature = "ssr")]` annotations are needed inside the actual implementation.**

```text
web/src/feature/
├── mod.rs     # The Seam (Shared DTOs + `#[server]` functions with real bodies)
└── server.rs  # Module-private helpers, transactions, and tests (only when warranted)
```

---

## Key Insight: `#[server]` Bodies Are Already Cfg-Gated

Leptos's `#[server]` proc-macro emits two expansions: on WASM, the body is replaced with HTTP-call boilerplate; on SSR, the body is kept verbatim. **The body therefore only ever compiles in SSR mode.** This has two consequences that shape the plan:

1. **No thin wrappers in `mod.rs`.** The `#[server]` function *is* the implementation. Putting a one-line `server::get_profile().await` body adds an extra named function for no benefit; it just doubles the stack frame and the name.
2. **No per-import cfg guards inside `#[server]` bodies.** A single `#[cfg(feature = "ssr")] use server::*;` at the top of `mod.rs` brings everything server-only into scope, and the bodies read as ordinary Rust.

### Canonical shape

```rust
// profile/mod.rs

#[cfg(feature = "ssr")]
mod server;
#[cfg(feature = "ssr")]
use server::*;

#[derive(Serialize, Deserialize)]
pub struct ProfileData { ... }   // shared DTO, no cfg needed

#[server]
pub async fn get_profile() -> WebResult<ProfileData> {
    boundary!("get_profile", {
        let user = require_auth().await?;
        let row = load_profile(user.user_id).await?;
        Ok(ProfileData::from(row))
    })
}
```

`server.rs` exists when there's genuine module-private logic worth naming (e.g. `perform_post_creation` in `posts`, `verify_token` in `password_reset`). For small modules (`profile`, `tags`, possibly `email`), it may not be needed — a `#[cfg(feature = "ssr")] use` block at the top of `mod.rs` can cover the upstream imports directly.

---

## Fate of `web_server_fn!`

The existing `web_server_fn!` macro in `web/src/lib.rs` does three things:
1. Cfg-gate the body to SSR — **redundant** with `#[server]`.
2. Generate the WASM stub — **redundant** with `#[server]`.
3. Wrap the body in `server_boundary(name, ...)` for tracing/error context — **still valuable**.

We replace it with a slim `boundary!` macro that does only (3):

```rust
#[macro_export]
macro_rules! boundary {
    ($name:expr, $body:block) => {
        $crate::error::server_boundary($name, async move $body).await
    };
}
```

`web_server_fn!` and `web_ssr!` are deleted from `lib.rs` as part of this refactor. Any remaining call sites migrate to `boundary!`.

---

## Re-export Surface

Several `auth` items are consumed by every other server module: `require_auth`, `AuthUser`, `set_session_cookie`, `clear_session_cookie`, `CookieSettings`, `AuthRejection`. When `auth.rs` becomes `auth/`, these move into `auth/server.rs` but **must remain reachable via `crate::auth::*`** so other modules' `use server::*;` (which in turn pulls `use crate::auth::require_auth;` etc.) continue to compile unchanged.

`auth/mod.rs` will therefore contain:

```rust
#[cfg(feature = "ssr")]
mod server;
#[cfg(feature = "ssr")]
pub use server::{
    require_auth, AuthUser, AuthRejection, CookieSettings,
    set_session_cookie, clear_session_cookie,
};
```

This is the one piece of plumbing that absolutely must be in place before any other module that consumes `crate::auth::*` is touched. Because `auth` is the last module refactored (Phase C), Phases A and B continue to import from the flat `auth.rs` file, which already exports these symbols — no transitional asymmetry to manage.

---

## Other Loose Ends

* **`web/src/storage.rs`** is a deprecated empty shim (a 3-line comment pointing callers to the `common` crate). Delete it as part of this refactor and remove the `pub mod storage;` line from `lib.rs`. There are no remaining references to it in the tree.
* **`#[server]` attribute arguments** (e.g. `#[server(name, prefix, endpoint)]`) on existing functions must be preserved verbatim during the move. Audit each function's attribute before rewriting the body; the public HTTP path is part of the wire contract and must not silently change.
* **Coverage baseline**: the existing baseline in `.coverage-manifest.json` stays as-is. Any per-file entries for files that move (e.g. `web/src/profile.rs` → `web/src/profile/mod.rs` / `web/src/profile/server.rs`) need their paths updated in the same commit as the file move; coverage *values* should not change materially since the code is the same. If `scripts/check-coverage` reports drift after a phase, investigate before adjusting.

---

## Reorganization Plan

For each module below, `server.rs` is created only when the module has genuine private helpers, transaction logic, or tests worth isolating. Modules marked **(mod.rs only)** keep the implementation inline in `#[server]` bodies, with `#[cfg(feature = "ssr")] use ...;` at the top of `mod.rs`.

### 1. `profile.rs` → `profile/` (mod.rs only)
* **Current**: 61 lines, 4 `#[cfg(ssr)]` annotations.
* **Target**: `profile/mod.rs` holds `ProfileData`, `#[cfg(feature = "ssr")] use { ... };` for `require_auth`, `UserStorage`, `InternalError`, and the two `#[server]` functions (`get_profile`, `update_profile`) with full bodies. No `server.rs` needed.

### 2. `email.rs` → `email/` (mod.rs only)
* **Current**: 69 lines, 5 `#[cfg(ssr)]` annotations.
* **Target**: `email/mod.rs` with `request_email_verification` and `verify_email` bodies inline. Upstream imports (`MailSender`, `EmailVerificationStorage`, `UserStorage`) under a single `#[cfg(feature = "ssr")] use ...;` block.

### 3. `invites.rs` → `invites/` (mod.rs only)
* **Current**: 64 lines, 5 `#[cfg(ssr)]` annotations.
* **Target**: `invites/mod.rs` holds `InviteInfo` and the two `#[server]` functions (`create_invite`, `list_invites`). Storage imports under one cfg-gated `use`.

### 4. `password_reset.rs` → `password_reset/`
* **Current**: 143 lines, heavy `#[cfg(ssr)]` density.
* **Target**:
    * `password_reset/mod.rs`: the two `#[server]` endpoints with bodies that orchestrate the flow.
    * `password_reset/server.rs`: private helpers for token hashing, mailer setup, and any internal tests.

### 5. `sessions.rs` → `sessions/` (mod.rs only)
* **Current**: 73 lines, several `#[cfg(ssr)]` annotations.
* **Target**: `sessions/mod.rs` holds `SessionInfo` and the two `#[server]` endpoints inline.

### 6. `backup.rs` → `backup/`
* **Current**: 390 lines mixing pure validation helpers with server interactions.
* **Target**:
    * `backup/mod.rs`: `BackupSettings` DTO; **shared pure validators** (`backup_schedule_valid`, `backup_mode_valid`, etc.) — these are *not* cfg-gated since they have no platform-specific code and may be used by client view code in the future; the four `#[server]` endpoints (`backup_warning_visible`, `current_user_is_operator`, `get_backup_settings`, `update_backup_settings`).
    * `backup/server.rs`: `require_operator`, site-config integration, backup-directory mapping, and storage-integrated tests.

### 7. `media.rs` → `media/`
* **Current**: 190 lines, several `#[cfg(ssr)]` imports.
* **Target**:
    * `media/mod.rs`: `MediaItem`, `MediaUsageData`, `DeleteMediaResult` DTOs; the three `#[server]` endpoints (`list_my_media`, `media_usage`, `delete_media`) with bodies inline.
    * `media/server.rs`: disk/storage interactions that aren't trivially inlinable (e.g. media-deletion bookkeeping shared across endpoints), plus tests.

### 8. `tags.rs` → `tags/` (mod.rs only)
* **Current**: reduced significantly by Phase 3.
* **Target**: `tags/mod.rs` holds `DEFAULT_TAG_LIMIT`, `MAX_TAG_LIMIT`, `TagSummary`, and the `list_tags` `#[server]` endpoint inline.

### 9. `auth.rs` → `auth/`
* **Current**: 571 lines, very high conditional-compilation density.
* **Target**:
    * `auth/mod.rs`: the five `#[server]` endpoints (`get_registration_policy`, `current_user`, `register`, `login`, `logout`) with full bodies, plus the **`pub use server::{...};` block** described in [Re-export Surface](#re-export-surface).
    * `auth/server.rs`: `AuthUser`, `CookieSettings`, `AuthRejection`, the `FromRequestParts` impl, cookie helpers, and authentication-business-rule helpers.

### 10. `posts.rs` → `posts/`
* **Current**: 1,200+ lines, 13 server functions, complex transaction logic.
* **Target**:
    * `posts/mod.rs`: all shared DTOs (`CreatePostResult`, `UpdatePostResult`, `DraftSummary`, `PublishPostResult`, `TimelinePostSummary`, `TimelinePage`, `PostResponse`, `PostCursor`); the 13 `#[server]` endpoints with bodies that orchestrate via helpers from `server.rs`.
    * `posts/server.rs`: `perform_post_creation`, `perform_post_update`, and other helpers genuinely shared across endpoints; storage-row mappers; the `sqlite_storage_tests` module.
    * Within `posts/server.rs`, group helpers thematically (creation, update, timeline pagination) with `//` section banners — but do *not* split into further submodules unless the file exceeds ~600 lines after the move. Premature splitting hides where logic lives.

---

## Verification & Rollout Plan

We perform this refactor **one module at a time**, starting with the smallest, to allow gradual verification of cargo builds:

1. **Preparation**:
    * Add the `boundary!` macro to `web/src/lib.rs` alongside the existing `web_server_fn!` so both coexist during the migration.
    * Delete `web/src/storage.rs` and the `pub mod storage;` declaration; update `.coverage-manifest.json` if it has an entry for this file. Run `scripts/verify`.
2. **Phase A (low-friction features)**: refactor `profile`, `email`, `invites`, `tags`. Run `scripts/verify` after each.
3. **Phase B (intermediate features)**: refactor `sessions`, `media`, `password_reset`, `backup`. Run `scripts/verify` after each.
4. **Phase C (high-complexity features)**: refactor `auth` (re-export block goes in first), then `posts`. Run the full Nix-VM e2e suite via `scripts/verify` after each.
5. **Cleanup**: delete `web_server_fn!` and `web_ssr!` from `lib.rs` once no call sites remain. Run `scripts/verify` one final time.

Per `CLAUDE.md`: every commit runs `scripts/verify`; each refactor is its own commit; user review is requested before committing.
