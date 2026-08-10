# Issue #533 — Session Token Out of Response Bodies: Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating an individual task to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-08-09-issue-533-session-token-body.md` —
read it for the _what_ and _why_; this plan is the _how_ and does not restate
it.

**Goal:** Make web session establishment cookie-only — `login` stops returning
the raw session token (keeping `is_operator`), `register` returns `()`.

**Architecture:** Drop the `token` field from `LoginResponse` and collapse
`register`'s return to `()`. Tests that needed the token to reach the session
recover it from the `Set-Cookie` header instead, via a new helper that is the
inverse of the existing `session_cookie`.

**Tech Stack:** Rust, Leptos server fns (`#[macros::server]`), axum, rstest /
rstest_reuse, cargo-nextest, `cargo xtask` gate.

## Review header

**Scope — in:**

- `web/src/auth/api.rs`, `web/src/registration/api.rs` — return types + docs.
- `web/src/registration/component.rs` — the `Result<RawToken, _>` annotation.
- `server/tests/helpers/mod.rs` — `token_from_set_cookie`.
- `server/tests/web/web_auth.rs` — eight token call sites, two helpers, two
  renames.
- A short ADR recording the cookie-only invariant.

**Scope — out:** `web/src/sessions/api.rs` (app-password token — the deliberate
exception); any `LoginResponse` rename (#782's); any structural xtask gate.

**Separable concerns:** none found.

**Tasks:**

1. Add `token_from_set_cookie` and the two failing body assertions (red). **No
   commit of its own** — see Task 1 Step 4.
2. Change both endpoints, the client annotation, and all eight test sites
   (green); commit Tasks 1 + 2 together.
3. Write the cookie-only ADR draft (untracked; `adr promote` commits it at ship)
   and run the full `validate` with e2e.

**Key risks / decisions:**

- Tasks 1 and 2 cannot be split further: changing a server-fn signature breaks
  compilation of every test in the crate, so the signature change and the test
  updates must land in one commit. Task 1 is therefore a genuine assertion-level
  red, not a compile error.
- If `RawToken` becomes server-gated in `web/src/auth/api.rs`, a surviving
  intra-doc `[`RawToken`]` link breaks the client build and the `doc-links` gate
  step. Rewriting the docs (Task 2 Step 3) is what prevents that.
- `register`'s success body becomes bare `null`, so
  `!body.contains("\"token\"")` is vacuous there — the assertions must test the
  token _value_.

## Global Constraints

- Dual-backend tests use `#[apply(backends)]`; a bare `#[tokio::test]` that
  should be dual-backend fails the `test-backend-pattern` guard.
- ADR drafts are numberless in `docs/adr/drafts/`; `cargo xtask adr promote`
  numbers them at ship (`jaunder-adr`). Do not hand-number.
- No `Co-Authored-By` trailer on commits.
- Run `cargo xtask check` before committing; the pre-commit hook runs it too.

---

### Task 1: The cookie-side helper and the failing assertions

**Files:**

- Modify: `server/tests/helpers/mod.rs` — add `token_from_set_cookie` beside
  `session_cookie` (~line 119).
- Modify: `server/tests/web/web_auth.rs` — add body assertions to
  `register_open_creates_user_sets_cookie_returns_token` (~line 65) and
  `login_correct_password_sets_cookie_and_returns_token` (~line 304).

**Interfaces:**

- Consumes: `post_form_with_secure_flag` (returns
  `(StatusCode, Option<String>, String)` — status, `Set-Cookie`, body);
  `RawToken` (`common::token`).
- Produces: `helpers::token_from_set_cookie(set_cookie: &str) -> RawToken`.

---

- [x] **Step 1: Write the helper**

Add to `server/tests/helpers/mod.rs`, directly after `session_cookie`:

```rust
/// The session token carried by a `Set-Cookie` header — the inverse of
/// [`session_cookie`]. Tests read the session the *cookie* establishes, because the
/// login/register response bodies no longer carry a token (#533).
///
/// # Panics
///
/// If the header is not a `session=<token>` cookie, or the token does not parse.
#[must_use]
pub fn token_from_set_cookie(set_cookie: &str) -> RawToken {
    let value = set_cookie
        .strip_prefix("session=")
        .expect("Set-Cookie is not a session cookie");
    // The cookie is `session=<token>; HttpOnly; SameSite=Lax; Path=/[; Secure]`
    // (`host::auth::session_cookie_header`), so the token runs to the first `;`.
    let token = value.split(';').next().unwrap_or(value);
    token.parse().expect("valid token in session cookie")
}
```

- [x] **Step 2: Write the failing assertions**

In `register_open_creates_user_sets_cookie_returns_token`, after the existing
`let cookie = set_cookie.expect(...)` / `starts_with("session=")` lines, add:

```rust
    // #533: the cookie is the only channel. The body must not carry the token —
    // asserted against the real token value, not just the field name, because
    // `register`'s body is a bare `null` and a name check would be vacuous.
    let cookie_token = token_from_set_cookie(&cookie);
    assert!(
        !body.contains(&*cookie_token),
        "register body leaked the session token: {body}"
    );
```

In `login_correct_password_sets_cookie_and_returns_token`, after its
`let cookie = set_cookie.expect(...)` line, add the same assertion with a
`login` message.

**Call it unqualified.** `web_auth.rs:12` imports helper _names_
(`use crate::helpers::{create_user_and_session, post_form_with_bearer, …}`) and
the file never uses a `helpers::`-qualified path — a
`helpers::token_from_set_cookie(…)` call would not resolve. Add
`token_from_set_cookie` to that import list and call it bare.

`&*cookie_token`, not `.as_ref()`: `RawToken` derefs to `str`, but
`str::contains` is generic over `Pattern`, so `.as_ref()` leaves the target type
unpinned and can need a type annotation.

- [x] **Step 3: Run the tests, verify they fail**

```bash
devtool run -- devtool pg run -- cargo nextest run -p jaunder register_open_creates_user_sets_cookie_returns_token login_correct_password_sets_cookie_and_returns_token
```

Expected: **FAIL**, all four cases (two tests × two backends), on the new
assertion — `register body leaked the session token: "<token>"` and the login
equivalent. The token is currently in both bodies, which is the bug.

(`RawToken` implements `FromStr` and derefs to `str` via the `StrNewtype`
trailer, and `server/tests/helpers/mod.rs:12` already imports it — the helper
compiles as written.)

- [x] **Step 4: Do NOT commit — carry into Task 2**

**Deliberate exception to one-clean-commit-per-task.** Changing a server-fn
signature breaks compilation of every test in the crate, so the signature change
and the test updates are physically inseparable; and the gate cannot pass with a
failing test. So Task 1 has no commit of its own and its changes land in Task
2's commit.

If this task is dispatched to a subagent, the brief must say so explicitly —
otherwise an iterate driver enforcing per-task commits will either fail the gate
on the red or reach for `--no-verify`. Neither is wanted. Tick this step to mark
the red observed and go straight to Task 2.

---

### Task 2: Stop returning the token

**Files:**

- Modify: `web/src/auth/api.rs` — `LoginResponse` (line 32-36), `login`
  (51-115), docs (9, 28-31, 38-39, 108-110).
- Modify: `web/src/registration/api.rs` — `register` (49-132), docs (14, 47-48).
- Modify: `web/src/registration/component.rs` — line 146, and the `RawToken`
  import (line 13).
- Modify: `server/tests/web/web_auth.rs` — helpers (17-43) and eight call sites.

**Interfaces:**

- Produces: `LoginResponse { is_operator: bool }` (no `token`);
  `register(...) -> WebResult<()>`.
- Consumes: `helpers::token_from_set_cookie` from Task 1.

---

- [x] **Step 1: Shrink `LoginResponse` and `login`**

In `web/src/auth/api.rs`, delete the `token` field:

```rust
pub struct LoginResponse {
    pub is_operator: bool,
}
```

and the construction at the end of `login`:

```rust
    set_session_cookie(&raw_token);
    leptos_axum::redirect("/");
    // The session travels only in the HttpOnly cookie set above (#533); the body
    // carries just the marker seed, so an XSS at login time cannot read a token
    // that was never sent to JS. `is_operator` comes from the authenticated
    // `UserRecord` — no extra query.
    Ok(LoginResponse {
        is_operator: record.is_operator,
    })
```

The rest of `login` is unchanged — it still mints `raw_token` and sets the
cookie.

- [x] **Step 2: Collapse `register`**

In `web/src/registration/api.rs`, change the signature to `-> WebResult<()>` and
the tail to:

```rust
    set_session_cookie(&raw_token);
    leptos_axum::redirect("/");
    // Session establishment is cookie-only (#533) — nothing to return.
    Ok(())
```

- [x] **Step 3: Rewrite every stale doc and fix the imports**

This is the step AC5 checks, and the intra-doc links are a build hazard, not a
nicety. AC5 lists **seven** stale docs; two of them are handled elsewhere in
this task — `web_auth.rs:30-31` by Step 5 (the helper is replaced wholesale) and
`auth/api.rs:108-110` by Step 1 (the comment is rewritten with the
construction). The remaining five are this step's job:

- `web/src/auth/api.rs:9` — the ungated-`RawToken` justification.
- `web/src/auth/api.rs:28-31` — `LoginResponse`'s doc.
- `web/src/auth/api.rs:38-39` — `login`'s doc, which links ``[`RawToken`]``.
- `web/src/registration/api.rs:14` — the ungated-`RawToken` justification.
- `web/src/registration/api.rs:47-48` — `register`'s doc, which links
  ``[`RawToken`]``.

Each should now say the token is set as an HttpOnly cookie and deliberately not
returned (#533). **Remove the ``[`RawToken`]`` intra-doc links** from any doc in
a file where the import becomes server-gated or is deleted — a link to a
`cfg`-gated-away item fails the client build and the `doc-links` gate step.

Then fix the imports: `web/src/auth/api.rs:13` and
`web/src/registration/api.rs:17`. `raw_token`'s type is inferred from
`create_session`, so the explicit import is likely now unused in both — delete
it if so; clippy will say. In `web/src/registration/component.rs`, drop the
`RawToken` import (line 13) once Step 4 removes its last use.

- [x] **Step 4: Fix the client annotation**

`web/src/registration/component.rs:146` — the action-value annotation becomes:

```rust
                        .and_then(|r: Result<(), WebError>| r.err())
```

`LoginPage` needs no change: it already matches `Ok(_)`.

- [x] **Step 5: Rework the test helpers**

In `server/tests/web/web_auth.rs`:

- **Delete `extract_token` entirely** (lines 17-28). `register`'s body is now
  bare `null`; there is nothing to extract, and a trimmed-but-unused helper
  fails the lint and coverage gates.
- **Replace `extract_login`** (lines 30-43) with a bool-only reader whose name
  matches what it does:

```rust
/// The `is_operator` flag from a login response body — all that `login` returns now
/// that the session token travels only in the HttpOnly cookie (#533).
fn is_operator_from_body(body: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Resp {
        is_operator: bool,
    }
    serde_json::from_str::<Resp>(body.trim())
        .expect("valid login JSON body")
        .is_operator
}
```

- Drop `use common::token::RawToken;` (line 5) if nothing else in the file needs
  it.

- [x] **Step 6: Update all eight call sites**

Work the inventory from the spec; none may be skipped.

**Non-empty-token assertions → assert on the cookie** (`:66`, `:139`, `:304`,
`:340`): delete the `extract_token`/`extract_login` token line and its
`assert!(!token.is_empty())`.

- `:66`, `:139`, `:304` already capture `set_cookie` and assert
  `starts_with("session=")`, which becomes the meaningful assertion. `:66` and
  `:139` additionally gain a real session check — see Step 6b.
- `:340` (`login_returns_is_operator_flag`) keeps its `is_operator` assertion —
  rewrite as `let is_operator = is_operator_from_body(&body);`. **Leave its
  cookie slot destructured as `_`**: this test has no cookie assertion, and
  binding `set_cookie` without using it is an unused-variable warning that fails
  the lint gate. Do not invent a new assertion here to justify a binding.

- [x] **Step 6b: Give `register` a real session check (AC4)**

All four `authenticate` sites are `login` tests. AC4 says "for **both**
endpoints, a test takes the `session` cookie, authenticates, and gets the
expected session" — so without this, `register` could stop minting a working
session and the suite would stay green, which is exactly the property AC4 names.

In `register_open_creates_user_and_sets_session_cookie` (the renamed `:48`
test), after the cookie assertion, add:

```rust
    // #533/AC4: the cookie is the only channel, so prove it actually establishes a
    // session for the new user — a `starts_with("session=")` check alone would pass
    // against a cookie carrying a token that authenticates nothing.
    let raw_token = token_from_set_cookie(&cookie);
    let record = state
        .sessions
        .authenticate(&raw_token)
        .await
        .expect("the register cookie authenticates");
    assert_eq!(record.user_id, user.user_id);
```

That test already fetches the created user
(`state.users.get_user_by_username(...)` just below the cookie block), so order
the two so `user` is in scope; reuse it rather than re-querying. If its binding
is shaped differently, assert on whatever identifies the user in that test.

**`authenticate(&raw_token)` sites → authenticate with the cookie token**
(`:395`, `:428`, `:539`, `:577`). `:395` and `:428` call
`post_form_with_secure_flag`; **`:539` and `:577` call `post_form_with_ua` (six
args)** — a different call, but it returns the same
`(StatusCode, Option<String>, String)` triple, so the substitution below applies
to all four. Match on the destructuring, not on the function name.

Each currently reads:

```rust
    let (status, _, body) = post_form_with_secure_flag(/* … */).await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = extract_login(&body).0;
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.label, "my-device");
```

and becomes:

```rust
    let (status, set_cookie, _body) = post_form_with_secure_flag(/* … */).await;

    assert_eq!(status, StatusCode::OK);
    // #533: the token reaches us only via the cookie, which is the channel a
    // browser actually uses — so this also proves the cookie is still being set.
    let cookie = set_cookie.expect("Set-Cookie header should be present on login");
    let raw_token = helpers::token_from_set_cookie(&cookie);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.label, "my-device");
```

keeping each site's own label assertion (`"my-device"`, `"Unknown device"`, and
whatever `:539` / `:577` assert).

- [x] **Step 7: Rename the two misleading tests**

- `register_open_creates_user_sets_cookie_returns_token` →
  `register_open_creates_user_and_sets_session_cookie`
- `login_correct_password_sets_cookie_and_returns_token` →
  `login_correct_password_sets_session_cookie`

Update the `// M2.9.8:` / `// M2.9.12:` comments above them, which also say
"returns token".

- [x] **Step 8: Run the auth tests, verify they pass**

```bash
devtool run -- devtool pg run -- cargo nextest run -p jaunder web_auth
```

Expected: **PASS**, every case. The two Task 1 assertions now hold because the
bodies no longer carry the token.

- [x] **Step 9: Run the gate**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-533-session-token-body -- cargo xtask check
```

Expected: exit 0. Watch the `doc-links` step specifically — it is what catches a
surviving intra-doc link to a gated-away `RawToken`. Read
`.xtask/last-result.json` `.steps` on failure. Follow `jaunder-commit`.

- [x] **Step 10: Commit**

```bash
git add web/src/auth/api.rs web/src/registration/api.rs web/src/registration/component.rs server/tests/helpers/mod.rs server/tests/web/web_auth.rs
git commit -m "fix(web): stop returning the session token in login/register bodies (#533)"
```

---

### Task 3: Record the cookie-only invariant as an ADR

**Files:**

- Create: `docs/adr/drafts/session-establishment-is-cookie-only.md` (numberless
  — `cargo xtask adr promote` numbers it at ship).

**Interfaces:** none produced.

**The draft is never committed.** `.gitignore:48-49` ignores `docs/adr/drafts/*`
except `README.md`, so `git add` on the draft fails. `cargo xtask adr promote`
at ship is what numbers it and makes its first, correctly-numbered appearance in
history. Leave it untracked here.

- [x] **Step 1: Read the ADR conventions**

Read `docs/adr/drafts/README.md`. The binding constraints come from there,
**not** from the `adr-format` gate — `xtask/src/steps/adr_check.rs` only scans
numbered `docs/adr/NNNN-*.md`, so drafts are never checked by it. From the
README:

- Heading exactly `# ADR-DRAFT: <Title>` — `promote` swaps `DRAFT` for the
  number.
- Leave `- Status: proposed` alone.
- Write links to sibling ADRs **as if the draft already lived in `docs/adr/`**
  (use the path, never a bare `ADR-DRAFT` token). Getting this wrong fails
  `doc-links` after promotion, not now — which is why it must be right the first
  time.

- [x] **Step 2: Write the draft**

It must state:

- **Decision:** web session establishment is cookie-only — a `#[server]` fn on
  the auth path sets the HttpOnly `session` cookie and returns no session-token
  material.
- **Why:** returning the token to page-visible JS defeats the HttpOnly cookie;
  an XSS at login/registration time could exfiltrate a long-lived session token
  (#533).
- **The deliberate exception:** `create_app_password`
  (`web/src/sessions/api.rs`) returns `AppPassword { token: RawToken }`. Showing
  that secret once is the endpoint's entire purpose, and it is not a browser
  session.
- **Why no machine gate:** a "no `#[server]` fn returns `RawToken`" check would
  have to walk struct field types (the app-password case is a field, not a bare
  return) and would need an allowlist for it. The allowlist is the part that
  rots. If the invariant is ever worth enforcing structurally, it should come
  with a distinct secret type for the app password.
- **How it is enforced today:** assertions in `server/tests/web/web_auth.rs`
  that the login and register bodies do not contain the token recovered from
  `Set-Cookie`.

Reference #533 and #591 (which is why `LoginResponse` still exists, carrying
`is_operator`).

- [x] **Step 3: Confirm the tree is otherwise clean, and run the full gate
      (AC8)**

```bash
git status --porcelain
```

Expected: **empty** — the draft is gitignored, so it must not appear. If it
does, the ignore rule is not doing its job; stop and investigate rather than
committing it.

Then the criterion `check` cannot satisfy — AC8 wants `validate` **with e2e**,
because the e2e login/registration UI flows are the regression cover for a
server-fn signature change on the auth path:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-533-session-token-body -- cargo xtask validate
```

Expected: exit 0. This is the ~25-minute run; use Bash background mode.

**No commit for this task** — the deliverable is the untracked draft plus a
green full gate. `jaunder-ship` promotes and commits the ADR after the rebase.

---

## Self-review

**Spec coverage:**

| Spec AC | Task                                                                            |
| ------- | ------------------------------------------------------------------------------- |
| AC1     | T1 S2 (login body assertion), T2 S1 (field removed)                             |
| AC2     | T2 S1 (`is_operator` kept), T2 S6 (`:340` edited, assertion kept)               |
| AC3     | T1 S2 (register body assertion), T2 S2 (`WebResult<()>`)                        |
| AC4     | T2 S6 (four login sites), **T2 S6b** (the register site)                        |
| AC5     | T2 S3 (five docs), T2 S5 + T2 S1 (the other two)                                |
| AC6     | T2 S7 (two renames), T2 S6 (the two name-invisible edits)                       |
| AC7     | Scope fence — no task touches `web/src/sessions/api.rs`                         |
| AC8     | **T3 S3** (`cargo xtask validate`, with e2e); T2 S9 is the iterate-time `check` |
| AC9     | Task 3 (the ADR draft, left untracked; promoted at ship)                        |

**Placeholders:** none — every step carries real Rust or a real command. Two
steps carry a conditional ("delete the import if unused; clippy will say"),
which is a verifiable instruction, not a hole.

**Type consistency:** `token_from_set_cookie(&str) -> RawToken` is spelled the
same in Task 1's Interfaces block, its implementation, and both Task 2 call-site
templates. `is_operator_from_body(&str) -> bool` replaces `extract_login`
consistently at `:340`. `LoginResponse` keeps its name (deliberately — #782 owns
any rename).
