# Session-credential Newtypes (`RawToken`/`TokenHash`) + Auth Hardening — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate (delegating individual tasks to a subagent via jaunder-dispatch when useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace stringly-typed session/token credentials with `RawToken` and `TokenHash` newtypes so raw-vs-hash transposition is uncompilable, and fold in the four #344 auth-hardening nits that ride the same code.

**Architecture:** Types live in `common::token` (wasm-visible; both tokens cross the server-fn wire); the RNG/SHA-256 machinery lives in `host::token` (server-only); `storage` becomes a pure consumer. `hash(&RawToken) -> TokenHash` is the sole raw→hash bridge. One generic pair is shared across the session / password-reset / email-verification families; invite keeps its `InviteCode`, minted from a `RawToken`.

**Tech Stack:** Rust, `macros::StrNewtype` (ADR-0063), `sqlx` (sqlite + postgres), Leptos server fns, `cargo nextest`, `cargo xtask check`.

**Spec:** `docs/superpowers/specs/2026-07-15-issue-458-session-credential-newtypes.md` — this plan is "how"; the spec is "what/why." AC references (AC1–AC12) point at spec §3.

## Global Constraints

- **Backend parity:** every storage behavior test runs on **both** sqlite and postgres via the dual-backend template (`CONTRIBUTING.md`); a bare `#[tokio::test]` that should be dual-backend fails the `test-backend-pattern` guard.
- **Coverage policy** applies (ADR-0050); `macros` crate is coverage-measured, `xtask` is not.
- **No `Co-Authored-By` trailer.** One clean commit per task. Pre-commit hook runs the full `cargo xtask check`; run it green first (jaunder-commit).
- **Governing decision:** ADR-0063 (newtype convention + secret-bearing exception). No new ADR.
- **`hash` is the sole `RawToken -> TokenHash` bridge:** no `From`/`Into` in either direction, no cross-type `PartialEq`.
- Gate command in this worktree: `devtool run -- cargo xtask check` (worktree-aware, honest exit).

---

## Review header — task list

1. **`common::token`** — add `RawToken` + `TokenHash` newtypes + unit tests. _(AC1, AC2)_
2. **`host::token`** — add `generate` / `hash` / `generate_hashed` machinery + golden test. _(AC3)_
3. **Session bearer-token flow** — thread the types end-to-end through `host::auth` (incl. #344 items 2 & 3), the `SessionStorage` trait + both backends + `SessionRecord`, and the web session surface. _(AC4, AC7, AC10, AC11)_
4. **Password-reset family** — thread the types through `storage::password`, `AtomicOps::confirm_password_reset`, both backends, and `web::password_reset`. _(AC5 reset)_
5. **#344 item 4** — `From<ConfirmPasswordResetError> for InternalError`; `?`-lift in `web::password_reset`. _(AC12)_
6. **Email-verification family** — thread the types through `storage::email`. _(AC5 email)_
7. **Invite generation + cleanup** — mint `InviteCode` from a `RawToken`; remove the three stringly doors (`storage::auth::{generate_token, hash_token}`, `storage::helpers::generate_hashed_token`). _(AC6)_
8. **#344 item 1** — fix the stale username-comparison comment; assert case-insensitive Basic-auth match. _(AC9)_
9. **`compile_fail` doctests** — prove the transposition is uncompilable. _(AC8)_

**Key risks / decisions:**

- Task 3 is the one large task: the session token type is one atomic compilable unit (cookie → `Credential` → `authenticate` → `SessionRecord` → web). It cannot be half-threaded. Its steps are sequenced so the crate compiles at the end of the task, not between sub-edits.
- The three stringly doors are removed only in Task 7, after Tasks 3/4/6 migrate every caller — removing earlier breaks the un-migrated families.
- `generate_hashed()` is infallible (a freshly minted 43-char base64url token always decodes; documented `.expect`), removing a spurious error path from every create site.

---

## Task 1: `common::token` — `RawToken` + `TokenHash`

**Files:**

- Modify: `common/src/token.rs` (append the two types; `validate_shape` + `InvalidTokenShape` already exist)
- Test: `common/src/token.rs` `#[cfg(test)]` (in-file, matching the module's existing tests)

**Interfaces:**

- Consumes: `common::token::{validate_shape, InvalidTokenShape}` (existing).
- Produces:
  - `pub struct RawToken(String)` — `#[derive(Clone, StrNewtype)]` `#[str_newtype(secret, serde)]`; `impl FromStr for RawToken { type Err = InvalidTokenShape; }`. Surface: redacting `Debug` (`RawToken([redacted])`), `AsRef<str>`, `TryFrom<String>`, serde.
  - `pub struct TokenHash(String)` — `#[derive(Clone, PartialEq, Eq, Hash, StrNewtype)]`; `impl FromStr for TokenHash { type Err = InvalidTokenShape; }`. Surface: `Display`, `AsRef<str>`, `Deref`, `Borrow`, `TryFrom<String>`, `PartialEq<str>`/`<&str>`, serde, plus std `PartialEq<Self>`/`Eq`/`Hash`.

- [ ] **Step 1: Write the failing tests** (append to `common/src/token.rs` tests)

```rust
use std::str::FromStr;

#[test]
fn raw_token_parses_valid_and_rejects_empty_and_bad_charset() {
    assert!(RawToken::from_str("abcABC012-_").is_ok());
    assert!(RawToken::from_str("").is_err());
    assert!(RawToken::from_str("has space").is_err());
    assert!(RawToken::from_str("plus+code").is_err());
}

#[test]
fn raw_token_debug_redacts_body() {
    let raw = RawToken::from_str("SecretBody123").unwrap();
    let shown = format!("{raw:?}");
    assert!(shown.contains("[redacted]"));
    assert!(!shown.contains("SecretBody123"));
}

#[test]
fn token_hash_parses_and_self_equality_holds() {
    let a = TokenHash::from_str("abcABC012-_").unwrap();
    let b = TokenHash::from_str("abcABC012-_").unwrap();
    assert_eq!(a, b); // std PartialEq<Self>
    assert!(TokenHash::from_str("").is_err());
}

#[test]
fn token_hash_serde_roundtrips() {
    let h = TokenHash::from_str("abcABC012-_").unwrap();
    let json = serde_json::to_string(&h).unwrap();
    let back: TokenHash = serde_json::from_str(&json).unwrap();
    assert_eq!(h, back);
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p common token`
Expected: FAIL — `RawToken`/`TokenHash` not defined. (`serde_json` is already a `common` dev-dep; if not, add it.)

- [ ] **Step 3: Implement against the tests**

Append to `common/src/token.rs`, to the interface signatures above. Both `FromStr` bodies are `validate_shape(s)?; Ok(Self(s.to_owned()))` — every branch (empty, bad-charset, ok) is pinned by Step 1; redaction and serde come from the macro variant, exercised by the Debug and serde tests. Model the derive/attribute lines on `common/src/invite.rs:25` (`ProfferedInviteCode`).

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p common token`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add common/src/token.rs
git commit -m "feat(common): RawToken and TokenHash newtypes (#458)"
```

Run `devtool run -- cargo xtask check` green first (jaunder-commit).

---

## Task 2: `host::token` — generate / hash / generate_hashed

**Files:**

- Create: `host/src/token.rs`
- Modify: `host/src/lib.rs` (add `pub mod token;`)
- Test: `host/src/token.rs` `#[cfg(test)]`

**Interfaces:**

- Consumes: `common::token::{RawToken, TokenHash}` (Task 1); `rand`, `sha2`, `base64` (move the impls from `storage/src/auth.rs:78,92`).
- Produces:
  - `pub fn generate() -> RawToken` — 32 random bytes, base64url-no-pad, wrapped via `RawToken::try_from`.
  - `pub struct TokenHashError` (`#[derive(Debug)]` + `Display`) — base64 decode failure.
  - `pub fn hash(token: &RawToken) -> Result<TokenHash, TokenHashError>` — base64url-decode the raw token, SHA-256, re-encode, wrap `TokenHash`.
  - `pub fn generate_hashed() -> (RawToken, TokenHash)` — `generate()` then `hash(&raw).expect("a freshly generated token is valid base64url")`.

- [ ] **Step 1: Write the failing tests** (`host/src/token.rs`)

```rust
#[test]
fn generate_produces_distinct_parseable_tokens() {
    let a = generate();
    let b = generate();
    assert_ne!(a.as_ref(), b.as_ref());
    assert!(!a.as_ref().is_empty());
}

#[test]
fn hash_matches_legacy_vector() {
    // Golden vector: the pre-refactor `hash_token` output for this exact input,
    // so hashing is byte-identical across the refactor (no session invalidation).
    let raw = RawToken::try_from("dGVzdC10b2tlbg".to_string()).unwrap();
    let hash = hash(&raw).unwrap();
    assert_eq!(hash.as_ref(), "GAENWEUeIAFR9RjX-9nBGm7lJVQ2s7hVQ9Hq3nJmY0A"); // regenerate below
}

#[test]
fn generate_hashed_pair_is_consistent() {
    let (raw, token_hash) = generate_hashed();
    assert_eq!(hash(&raw).unwrap(), token_hash);
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p host token`
Expected: FAIL — module absent.

**Note on the golden vector:** compute the real expected hash before implementing by running the *current* `storage::auth::hash_token` on `"dGVzdC10b2tlbg"` (e.g. a scratch `#[test]` printing `hash_token("dGVzdC10b2tlbg")`), and paste the value into `hash_matches_legacy_vector`. This pins byte-identical behavior; do not invent the constant.

- [ ] **Step 3: Implement against the tests**

Create `host/src/token.rs` to the interface signatures, moving the body from `storage/src/auth.rs:78,92` (do **not** delete the storage originals yet — Task 7). Add `pub mod token;` to `host/src/lib.rs`. Confirm `rand`/`sha2`/`base64` are `host` deps (add if missing — they are server-only, no wasm concern for `host`).

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p host token`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add host/src/token.rs host/src/lib.rs host/Cargo.toml
git commit -m "feat(host): token generate/hash machinery over RawToken/TokenHash (#458)"
```

---

## Task 3: Session bearer-token flow, end-to-end

Thread the newtypes through the whole session-credential path so it compiles as a unit. Delivers AC4, AC7, AC10 (#344 item 2), AC11 (#344 item 3).

**Files:**

- Modify: `host/src/auth.rs` — `Credential.token: RawToken` (`:21`); `resolve_credential` per-source parse (`:35-67`); `session_cookie_header(token: &RawToken, secure: bool)` (`:72`).
- Modify: `storage/src/sessions.rs` — trait (`:66,76,79`), `SessionRecord.token_hash: TokenHash` (`:13`), `SessionStore` impl (`:145,169,188`), dialect decl `touch_and_load(pool, token_hash: &TokenHash, now)` (`:110`).
- Modify: `storage/src/sqlite/sessions.rs`, `storage/src/postgres/sessions.rs` — `touch_and_load(&TokenHash)` (bind `token_hash.as_ref()`).
- Modify: `storage/src/helpers.rs` — `SessionRow`/`session_record_from_row`/`build_session_record` (`:70,231-234`) produce `TokenHash`.
- Modify: `web/src/auth/server.rs` — `AuthUser.token_hash: TokenHash` (field at `:26`); `sessions.authenticate(&credential.token)` (`:69`) now typechecks (`&RawToken`); the `set_session_cookie(raw_token: &RawToken, …)` helper (`:162`) and its `session_cookie_header(raw_token, …)` call (`:169`); the `set_session_cookie("token")` unit test (`:236`) constructs a `RawToken`.
- Modify: `web/src/auth/mod.rs` — the `register`/`login` server fns call `create_session` (→ `RawToken`, `:138,206`), pass it to `set_session_cookie` (`:142,210`), and `revoke_session(&auth.token_hash)` (→ `&TokenHash`, `:223`). (Spec blast radius lists this file; the crate will not compile without it.)
- Modify: `web/src/sessions/mod.rs` — `SessionInfo.token_hash: TokenHash` (`:14`), `AppPassword.token: RawToken` (`:44`), `revoke_session(token_hash: TokenHash)` (`:72`), the `is_current`/ownership compares (`:32,80`).
- Modify: `web/src/pages/sessions.rs` — the hidden-form-field render of the hash (`:73`).
- Test: `host/src/auth.rs` `#[cfg(test)]`; `web/src/sessions/mod.rs` + `server/tests/web/web_sessions.rs` (existing, must stay green).

**Interfaces:**

- Consumes: `common::token::{RawToken, TokenHash}` (T1); `host::token::{generate_hashed, hash}` (T2).
- Produces:
  - `SessionStorage::create_session(&self, user_id: i64, label: &str) -> sqlx::Result<RawToken>`
  - `SessionStorage::authenticate(&self, raw_token: &RawToken) -> Result<SessionRecord, SessionAuthError>`
  - `SessionStorage::revoke_session(&self, token_hash: &TokenHash) -> sqlx::Result<()>`
  - `SessionRecord.token_hash: TokenHash`
  - `host::auth::Credential.token: RawToken`; `host::auth::session_cookie_header(&RawToken, bool)`

- [ ] **Step 1: Write the failing #344 behavior tests** (`host/src/auth.rs` tests)

```rust
#[test]
fn resolve_credential_empty_session_cookie_falls_through_to_header() {
    // #344 item 2: an empty `session=` cookie must NOT short-circuit; a valid
    // Authorization header on the same request must still authenticate.
    let mut headers = http::HeaderMap::new();
    headers.insert(http::header::COOKIE, "session=".parse().unwrap());
    headers.insert(
        http::header::AUTHORIZATION,
        "Bearer abcABC012-_".parse().unwrap(),
    );
    let credential = resolve_credential(&headers).expect("credential from header");
    assert_eq!(credential.token.as_ref(), "abcABC012-_");
}

#[test]
fn resolve_credential_rejects_unparseable_bearer() {
    // A Bearer value that is not a valid RawToken yields no credential from that source.
    let mut headers = http::HeaderMap::new();
    headers.insert(http::header::AUTHORIZATION, "Bearer has space".parse().unwrap());
    assert!(resolve_credential(&headers).is_none());
}
```

The existing `session_cookie_header_*_matches_current_string` tests (`:132-160`) stay — update their call sites to pass `RawToken::try_from("token".to_string()).unwrap()` so the emitted string is unchanged (AC11: charset now type-guaranteed, output identical). #344 item 3 needs no new assertion beyond the type change; add a one-line comment at the interpolation site recording the base64url-charset invariant.

- [ ] **Step 2: Run the new tests, verify they fail**

Run: `devtool run -- cargo nextest run -p host auth`
Expected: FAIL — `credential.token` is `String` (no `.as_ref()` to `&str` returning the typed value / empty-cookie still short-circuits).

- [ ] **Step 3: Implement the type threading**

Make every edit in the Files list. Order within the task: (a) `host::auth` first (`Credential.token: RawToken`; each of the cookie/Bearer/Basic arms in `resolve_credential` does `RawToken::from_str(value).ok()` and contributes a credential only on `Some` — an empty/invalid value is skipped; `session_cookie_header(&RawToken)`); (b) `storage` trait + `SessionStore` (`create_session` → `host::token::generate_hashed()`; `authenticate` → `host::token::hash(raw_token)`; `revoke_session(&TokenHash)`) + `SessionRecord` + backends + helpers; (c) `web` (`web/src/auth/server.rs` `AuthUser` + `set_session_cookie(&RawToken)` helper + its test, `web/src/auth/mod.rs` register/login/revoke call sites, `web/src/sessions/mod.rs` `SessionInfo`/`AppPassword`/`revoke_session` server fn, `web/src/pages/sessions.rs` render). The body of each edit is a mechanical `String`/`&str` → `RawToken`/`TokenHash` substitution pinned by "the crate compiles + existing session suite passes"; the only new *behavior* is the `resolve_credential` per-source parse pinned by Step 1.

- [ ] **Step 4: Run the tests, verify they pass**

Run each; all PASS:
- `devtool run -- cargo nextest run -p host auth`
- `devtool run -- cargo nextest run -p storage sessions`
- `devtool run -- cargo nextest run -p web sessions`
- `devtool run -- cargo nextest run -p server web_sessions`

- [ ] **Step 5: Commit**

```bash
git add host/src/auth.rs storage/src/sessions.rs storage/src/sqlite/sessions.rs storage/src/postgres/sessions.rs storage/src/helpers.rs web/src/auth/server.rs web/src/auth/mod.rs web/src/sessions/mod.rs web/src/pages/sessions.rs
git commit -m "refactor(session): thread RawToken/TokenHash through the session credential flow (#458, #344)"
```

---

## Task 4: Password-reset family typed

**Files:**

- Modify: `storage/src/atomic.rs` — `AtomicOps::confirm_password_reset(&self, raw_token: &RawToken, new_password: &Password)` (`:104-108`).
- Modify: `storage/src/password.rs` — `create_password_reset` uses `host::token::generate_hashed()` (`:85`); the reset lookup hashes via `host::token::hash` (`:104`).
- Modify: `storage/src/sqlite/mod.rs` (`:283`), `storage/src/postgres/mod.rs` (`:158`) — `confirm_password_reset` impls hash the `&RawToken` via `host::token::hash`.
- Modify: `web/src/password_reset/mod.rs` — `confirm_password_reset` parses the client token into `RawToken` and passes `&RawToken` (`:74`).
- Test: existing `server/tests` password-reset suite + `storage` password tests (dual-backend), stay green.

**Interfaces:**

- Consumes: `RawToken`, `TokenHash`, `host::token::{generate_hashed, hash}`.
- Produces: `AtomicOps::confirm_password_reset(&self, raw_token: &RawToken, …)`.

- [ ] **Step 1: Adjust the existing tests to the new types**

The password-reset behavior is unchanged; update the existing test call sites to construct/pass `RawToken` (e.g. `RawToken::try_from(raw).unwrap()`) instead of `&str`. No new behavior branch — the contract is "same behavior, typed surface," pinned by the existing suite.

- [ ] **Step 2: Run, verify they fail to compile**

Run: `devtool run -- cargo nextest run -p storage password`
Expected: FAIL — signature mismatch until Step 3.

- [ ] **Step 3: Implement the threading**

Mechanical `&str`/`String` → `RawToken`/`TokenHash` at the cited lines; `web::password_reset::confirm_password_reset` does `let raw = RawToken::try_from(token)?;` (map a parse error to `InternalError::validation("invalid reset token")`) then passes `&raw`.

- [ ] **Step 4: Run, verify PASS**

Run: `devtool run -- cargo nextest run -p storage password` and `devtool run -- cargo nextest run -p server` (password-reset integration) — PASS.

- [ ] **Step 5: Commit**

```bash
git add storage/src/atomic.rs storage/src/password.rs storage/src/sqlite/mod.rs storage/src/postgres/mod.rs web/src/password_reset/mod.rs
git commit -m "refactor(password-reset): thread RawToken/TokenHash through reset tokens (#458)"
```

---

## Task 5: #344 item 4 — password-reset error classification

**Files:**

- Modify: `storage/src/atomic.rs` — add `From<ConfirmPasswordResetError> for host::error::InternalError` beside the `RegisterWithInviteError` mirror (`:30-53`).
- Modify: `web/src/password_reset/mod.rs` — replace `.map_err(InternalError::storage)` with `?` (`:74-83`).
- Test: `web/src/password_reset/mod.rs` `#[cfg(test)]` (or the existing password-reset test module).

**Interfaces:**

- Consumes: `storage::atomic::ConfirmPasswordResetError` (variants `NotFound`/`Expired`/`AlreadyUsed`/`Internal`); `host::error::InternalError`.
- Produces: `impl From<ConfirmPasswordResetError> for host::error::InternalError`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn confirm_reset_error_maps_token_failures_to_validation() {
    use host::error::{ErrorKind, InternalError};
    use storage::atomic::ConfirmPasswordResetError;
    for e in [
        ConfirmPasswordResetError::NotFound,
        ConfirmPasswordResetError::Expired,
        ConfirmPasswordResetError::AlreadyUsed,
    ] {
        let mapped: InternalError = e.into();
        assert_eq!(mapped.kind(), ErrorKind::Validation); // was masked as Storage (a 5xx)
    }
}
```

Accessor is `InternalError::kind() -> host::error::ErrorKind` (`host/src/error.rs:262`), asserted exactly as the existing error tests (`error.rs:430`). The `Internal(sqlx::Error)` arm maps to `ErrorKind::Storage`; it needs no separate assertion here (a `sqlx::Error` is awkward to fabricate) — it is covered by the existing password-reset integration suite continuing to pass.

- [ ] **Step 2: Run, verify it fails**

Run: `devtool run -- cargo nextest run -p web password_reset`
Expected: FAIL — no `From` impl.

- [ ] **Step 3: Implement**

Add the `From` impl mirroring `:35-52`: `NotFound => InternalError::validation("token not found")`, `Expired => InternalError::validation("token has expired")`, `AlreadyUsed => InternalError::validation("token has already been used")`, `Internal(e) => InternalError::storage(e)`. Change `web::password_reset::confirm_password_reset` to `?`-lift.

- [ ] **Step 4: Run, verify PASS**

Run: `devtool run -- cargo nextest run -p web password_reset` — PASS.

- [ ] **Step 5: Commit**

```bash
git add storage/src/atomic.rs web/src/password_reset/mod.rs
git commit -m "fix(password-reset): classify expired/used/unknown reset tokens as validation, not storage (#344)"
```

---

## Task 6: Email-verification family typed

**Files:**

- Modify: `storage/src/email.rs` — `create_email_verification` uses `host::token::generate_hashed()` (`:95`); the verification lookup hashes via `host::token::hash` (`:135`).
- Test: existing `storage` email suite (dual-backend), stays green.

**Interfaces:**

- Consumes: `RawToken`, `TokenHash`, `host::token::{generate_hashed, hash}`.
- Produces: the email-verification create/use functions speaking `RawToken`/`TokenHash` (signatures follow the same String→newtype substitution as Task 4; cite exact lines from the file when editing).

- [ ] **Step 1: Adjust existing tests to the new types** — construct/pass `RawToken` where the email tests previously used `&str`; behavior unchanged.

- [ ] **Step 2: Run, verify compile-fail** — `devtool run -- cargo nextest run -p storage email` → FAIL.

- [ ] **Step 3: Implement the threading** — mechanical substitution at `:95,135` and the affected signatures; use `UseEmailVerificationError::NotFound` on hash failure exactly as today (`:135`).

- [ ] **Step 4: Run, verify PASS** — `devtool run -- cargo nextest run -p storage email` → PASS.

- [ ] **Step 5: Commit**

```bash
git add storage/src/email.rs
git commit -m "refactor(email): thread RawToken/TokenHash through verification tokens (#458)"
```

---

## Task 7: Invite generation + remove the stringly doors

**Files:**

- Modify: `storage/src/invites.rs` — mint via `host::token::generate()` and wrap into `InviteCode` (`:89`).
- Modify: `storage/src/auth.rs` — **remove** `generate_token` (`:78`) and `hash_token` (`:92`) and their now-orphaned unit tests (`:209-231`).
- Modify: `storage/src/helpers.rs` — **remove** `generate_hashed_token` (`:311`) and its test (`:853`).
- Modify: `common/src/token.rs` — the module doc (`:5`) points tokens at `storage::auth::generate_token`, which this task deletes; repoint it to `host::token::generate`.
- Test: existing invite suite stays green.

**Interfaces:**

- Consumes: `host::token::generate`; `host::invite::InviteCode` (existing).
- Produces: no new public surface; net removal of three functions.

- [ ] **Step 1: Confirm no remaining callers**

Run: `devtool run -- rg -n 'generate_token|hash_token|generate_hashed_token' storage host web common --glob '!*/tests/*'`, then read the parked output. Expected non-test hits: the definitions in `storage/src/auth.rs` + `storage/src/helpers.rs`, the `invites.rs:89` `generate_token` call, and the `common/src/token.rs:5` module-doc prose mention (updated in Step 2) — all family create/lookup call sites were migrated in Tasks 2–6. Any *other* call site means a family was missed; stop and migrate it before deleting.

- [ ] **Step 2: Migrate invite + delete the doors**

`invites.rs:89`: `let code = host::token::generate().as_ref().parse::<InviteCode>()?;` — `InviteCode` has no `TryFrom<RawToken>`, so reach it through its `FromStr` (`host/src/invite.rs:29`), the same `.as_ref().parse()` idiom used at `host/src/invite.rs:42`. Delete the three functions and their tests, and repoint the `common/src/token.rs:5` module-doc mention to `host::token::generate`.

- [ ] **Step 3: Run the full build + invite suite**

Run: `devtool run -- cargo nextest run -p storage invites` and `devtool run -- cargo xtask check --no-test` (confirms nothing else referenced the deleted doors).
Expected: PASS / clean.

- [ ] **Step 4: Commit**

```bash
git add storage/src/invites.rs storage/src/auth.rs storage/src/helpers.rs common/src/token.rs
git commit -m "refactor(storage): mint invite codes from host::token; remove stringly token doors (#458)"
```

---

## Task 8: #344 item 1 — username-comparison comment + case test

**Files:**

- Modify: `web/src/auth/server.rs` — correct/remove the stale comment at `:141` ("The pure comparison lives in `common::auth`"); the comparison lives inline in `verify_basic_username` (`:148`).
- Test: `web/src/auth/server.rs` `#[cfg(test)]`.

**Interfaces:**

- Consumes: `common::username::Username` (case-normalizing `FromStr`, `common/src/username.rs:26`); `verify_basic_username` (`web/src/auth/server.rs:148`).

- [ ] **Step 1: Write the failing/anchoring test**

```rust
#[test]
fn basic_username_match_is_case_insensitive() {
    // Username lowercases at construction, so a differently-cased Basic-auth
    // username still matches the token's username.
    let authenticated = Username::from_str("alice").unwrap();
    let expected = Username::from_str("Alice").unwrap(); // normalizes to "alice"
    assert!(verify_basic_username(&authenticated, Some(&expected)).is_ok());
}
```

- [ ] **Step 2: Run, verify it passes already** (behavior is correct today; this test *locks* it)

Run: `devtool run -- cargo nextest run -p web auth`
Expected: PASS — confirming the normalization invariant. (If it FAILS, the case assumption is wrong; stop and reassess before touching the comment.)

- [ ] **Step 3: Fix the stale comment** — replace the `:141` line with an accurate note: the comparison is inline here, and equality is case-insensitive because `Username::from_str` lowercases (link `common/src/username.rs`).

- [ ] **Step 4: Re-run** — `devtool run -- cargo nextest run -p web auth` → PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/auth/server.rs
git commit -m "docs(auth): correct stale username-comparison comment; lock case-insensitive match (#344)"
```

---

## Task 9: `compile_fail` doctests — prove the distinction bites

**Files:**

- Modify: `common/src/token.rs` — add `compile_fail` doctests to the `RawToken`/`TokenHash` rustdoc, modeled on `common/src/render.rs:66-71`.

**Interfaces:** none new.

- [ ] **Step 1: Add the doctests** (AC8)

````rust
/// Raw construction and cross-type conversion do not compile:
/// ```compile_fail
/// let _ = common::token::RawToken("abc".to_string()); // private field
/// ```
/// ```compile_fail
/// use std::str::FromStr;
/// let raw = common::token::RawToken::from_str("abc").unwrap();
/// let _h: common::token::TokenHash = raw.into(); // no RawToken -> TokenHash conversion
/// ```
/// ```compile_fail
/// use std::str::FromStr;
/// let hash = common::token::TokenHash::from_str("abc").unwrap();
/// let _r: common::token::RawToken = hash.into(); // no reverse conversion
/// ```
/// ```compile_fail
/// use std::str::FromStr;
/// let raw = common::token::RawToken::from_str("abc").unwrap();
/// let hash = common::token::TokenHash::from_str("abc").unwrap();
/// let _ = raw == hash; // no cross-type PartialEq -> revoke_session(raw) can't typecheck either
/// ```
````

- [ ] **Step 2: Verify the doctests compile-fail as intended**

Run: `devtool run -- cargo test -p common --doc token`
Expected: PASS (each `compile_fail` block is confirmed to NOT compile; a block that *does* compile fails the doctest). If any block unexpectedly compiles, the type surface is too permissive — fix the type, not the doctest.

**Note (enforcement boundary):** `cargo xtask check`/CI run tests via `nextest`/`llvm-cov`, which do **not** execute doctests (same fact `macros/src/lib.rs:204` documents). So this `--doc` run is a **local/documentation** guard, not a gated regression barrier. That is consistent with the spec's deliberate "compile_fail test, not a runtime gate" choice — and the *actual* enforcement is stronger and gated: any real raw-vs-hash transposition regression makes the workspace fail to compile under `cargo xtask check`, because the whole codebase now consumes the distinct types. The doctests document the guarantee; the gate's build enforces it. (Wiring `cargo test --doc` into xtask is a possible future hardening, out of scope here.)

- [ ] **Step 3: Commit**

```bash
git add common/src/token.rs
git commit -m "test(common): compile_fail doctests pin RawToken/TokenHash distinction (#458)"
```

---

## Self-review notes (author)

- **Spec coverage:** AC1/AC2→T1; AC3→T2; AC4/AC7/AC10/AC11→T3; AC5→T4+T6; AC12→T5; AC6→T7; AC9→T8; AC8→T9. All twelve mapped.
- **Removal ordering:** the three stringly doors are deleted only in T7, after every caller (T3 session, T4 reset, T6 email) is migrated; T7 Step 1 verifies no stragglers.
- **Separable concerns:** none surfaced beyond the already-filed #457 (IDs) and #459 (content hash); no first-task issue filing needed.
- **Golden vector (T2):** the one literal that must be computed from live code, with an explicit step to do so — not invented.
