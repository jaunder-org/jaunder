# Issue #533 — stop returning the raw session token in login/register response bodies

## Status

Spec — awaiting approval.

## Background

`login` and `register` set the HttpOnly `session` cookie **and** return the raw
session token as the `Ok` payload:

- `web/src/auth/api.rs:55` — `login(...) -> WebResult<LoginResponse>`, where
  `LoginResponse { token: RawToken, is_operator: bool }`.
- `web/src/registration/api.rs:54` — `register(...) -> WebResult<RawToken>`.

Returning the token to page-visible JS defeats the point of the HttpOnly cookie:
an XSS present at login or registration time can exfiltrate a long-lived session
token instead of nothing. No in-repo browser code reads the token — `LoginPage`
matches `Ok(_)` and discards it (`web/src/auth/component.rs:81`), and
`RegisterPage` only inspects the `Err` arm
(`web/src/registration/component.rs:146`).

This is pre-existing behavior surfaced by the 2026-07-18 cold review of PR #508.

## Correction to the issue's proposed fix

The issue says to change **both** endpoints to `WebResult<()>`. That is correct
for `register` but **wrong for `login`**.

`LoginResponse` also carries `is_operator`, and it is load-bearing: the client
reads it at `web/src/auth/component.rs:33` to seed the session marker, which is
exactly what #591 added so operator chrome is flash-free on first login.
Collapsing `login` to `WebResult<()>` would regress #591.

So: `login` keeps its response struct and loses only the `token` field;
`register`, whose client hardcodes `is_operator: false` and discards the payload
entirely, becomes `WebResult<()>`.

## The invariant

> Session establishment for the web client is **cookie-only**. Neither `login`
> nor `register` returns session-token material in its response body; the
> HttpOnly `session` cookie is the sole channel by which the browser obtains a
> session.

Out of scope for the invariant, deliberately: `create_app_password`
(`web/src/sessions/api.rs:62`) returns `WebResult<AppPassword>`, and
`AppPassword` carries a `token: RawToken` field (`:54`). Handing that secret
back exactly once is the entire purpose of that endpoint, and it is not a
browser session. This spec does not touch it. (Note the shape: it is a _field
inside a returned struct_, not a bare token return — which is why the rejected
structural gate below would need to inspect field types, not just signatures.)

## Design

### Server

| Endpoint   | Before                                                  | After                                              |
| ---------- | ------------------------------------------------------- | -------------------------------------------------- |
| `login`    | `WebResult<LoginResponse>` with `token` + `is_operator` | `WebResult<LoginResponse>` with `is_operator` only |
| `register` | `WebResult<RawToken>`                                   | `WebResult<()>`                                    |

Both continue to call `set_session_cookie(&raw_token)` and
`leptos_axum::redirect("/")` exactly as now. The minted `raw_token` becomes a
server-local value in both functions rather than a returned one.

`LoginResponse` keeps its struct shape rather than collapsing to
`WebResult<bool>`: the field is named at the call site, the client's existing
`resp.is_operator` read is unchanged, and a future additive field does not churn
the wire type again.

One cross-reference: ADR-0097 (line 96) records that **#782 tracks auditing
`web::auth::LoginResponse`'s role-suffix name**. This spec deliberately does not
rename it — shrinking it to one field makes it a more obvious candidate for that
audit, but renaming here would collide with #782's scope. Leave the name to
#782.

### Import gating

Both API files carry comments explaining that `RawToken` is imported **ungated**
(across client and server builds) precisely _because_ it is a wire type:

- `web/src/auth/api.rs:9` — "`RawToken` is ungated for the same reason — it is
  …"
- `web/src/registration/api.rs:14` — "`RawToken` the wire _return_ type of
  `register`, so the `#[server]`-generated …"

Once the token leaves the wire, that justification no longer holds. Each import
must be re-examined: if `RawToken` becomes server-only in that file, gate it
accordingly, and **update or delete the stale comment**. A comment that explains
a decision that no longer exists is worse than none.

### Client

`web/src/registration/component.rs:146` annotates the action value as
`Result<RawToken, WebError>`; it becomes `Result<(), WebError>`. The `RawToken`
import in that file goes with it if nothing else uses it. `LoginPage` needs no
change — it already matches `Ok(_)`.

### Tests

`server/tests/web/web_auth.rs` is where the real work is. Today two helpers pull
the token out of the response body, across **eight** call sites — the full
inventory, because a partial list is how two of them get missed:

| Helper                                     | Call sites                     | What the site does with the token                                   |
| ------------------------------------------ | ------------------------------ | ------------------------------------------------------------------- |
| `extract_token(&body) -> RawToken`         | `:66`, `:139`                  | asserts it is non-empty                                             |
| `extract_login(&body) -> (RawToken, bool)` | `:304`, `:340`                 | asserts it is non-empty                                             |
| `extract_login(&body).0`                   | `:395`, `:428`, `:539`, `:577` | `sessions.authenticate(&raw_token)`, then asserts on `record.label` |

The two helpers are **not** treated the same way:

- **`extract_token` is deleted.** It returns _only_ a token, and `register`'s
  success body becomes a bare `null` — there is nothing left for it to extract.
  Trimming it would leave a dead function that the lint and coverage gates would
  reject. Its `use common::token::RawToken;` at `web_auth.rs:5` goes with it if
  nothing else in the file needs it.
- **`extract_login` degenerates to a bool parse** and is renamed to say so (e.g.
  `is_operator_from_body(&body) -> bool`); a name promising a login _tuple_
  would misdescribe it. Its doc comment at `:30-31`, which currently narrates
  both wire shapes, is rewritten.

The four `authenticate` sites re-derive the session from the cookie (below). The
four non-empty assertions (`:66`, `:139`, `:304`, `:340`) assert on the cookie
instead.

Those tests re-derive the session **from the `Set-Cookie` header**, which the
test harness already returns (`server/tests/helpers/mod.rs:470` captures it, and
several tests at lines 69, 306, 929 already assert on it). This is the right
substitution because it exercises the delivery path a browser actually uses: a
test that instead queried storage by `user_id` would still pass if the cookie
stopped being set at all, which is precisely the property under test.

A new helper in `server/tests/helpers/mod.rs` parses the token out of a
`Set-Cookie` value, sitting beside the existing
`session_cookie(&RawToken) -> String` as its inverse:

```rust
/// The session token carried by a `Set-Cookie` header — the inverse of
/// [`session_cookie`]. Tests read the session the *cookie* establishes, since the
/// response body no longer carries a token (#533).
pub fn token_from_set_cookie(set_cookie: &str) -> RawToken;
```

Tests that only assert a token was minted and non-empty assert on the cookie
instead. Two of them are **renamed**, because a name promising "returns token"
would outlive the behavior:
`register_open_creates_user_sets_cookie_returns_token` and
`login_correct_password_sets_cookie_and_returns_token`.

Two more need editing but do **not** advertise a token in their names, so a
name-based sweep misses them — they are called out here for that reason:

- `register_invite_only_valid_code_creates_user_marks_invite_used` (`:139`).
- `login_returns_is_operator_flag` (`:340`) — its `is_operator` assertion
  survives unchanged, but it currently also destructures and asserts the token,
  which must go.

### Regression guard: tests, not tooling

Two assertions of the required shape already exist at `web_auth.rs:468` and
`:502` (`assert!(!body.contains("\"token\""))`). This spec extends that approach
to the success paths rather than adding an xtask gate.

A structural "no `#[server]` fn returns `RawToken`" check was considered and
rejected for this cycle: the app-password endpoint legitimately returns one
(inside a struct field, so the check would have to walk field types too), the
gate would need an allowlist, and the allowlist is the part that rots. If that
invariant is wanted later it should come with a distinct secret type for the app
password, which is a wider change than this issue authorizes.

### Where the invariant lives

Rejecting tooling leaves a gap: the rule would exist only in the tests for these
two endpoints, and in this spec — which `jaunder-ship` archives. The next person
adding a `#[server]` fn on the auth path has nothing discoverable to read.

So this cycle also records the invariant as a **short ADR** (via `jaunder-adr`'s
draft-out-of-git flow: a numberless draft in `docs/adr/drafts/`, numbered at
ship by `cargo xtask adr promote`). It states the cookie-only rule, names the
app-password endpoint as the deliberate exception, and records why a machine
gate was declined — so a future reader inherits the reasoning rather than
re-deriving it.

This is documentation, not tooling; it does not reopen the rejected-gate
decision.

## Acceptance criteria

1. **AC1 — `login` returns no token.** `LoginResponse` has no `token` field. A
   test recovers the real token from the `Set-Cookie` header and asserts the
   **response body does not contain that token string**. The weaker in-repo
   precedent (`!body.contains("\"token\"")`, `web_auth.rs:468`) checks only the
   field _name_ and is not sufficient on its own here — the strong form costs
   nothing once `token_from_set_cookie` exists.

2. **AC2 — `login` still carries `is_operator`.** `LoginResponse.is_operator` is
   unchanged and still populated from the authenticated `UserRecord`, and
   `web/src/auth/component.rs` still seeds the marker from it — #591 is not
   regressed. `login_returns_is_operator_flag` **is edited** (it currently also
   destructures and asserts the token) and its `is_operator` assertion still
   passes. The edit is required, not a violation.

3. **AC3 — `register` returns `()`.** Its signature is `WebResult<()>`. A test
   asserts the success body does not contain the token recovered from
   `Set-Cookie`. Note the body becomes a bare `null`, which makes a
   `!body.contains("\"token\"")` assertion vacuous — so the token-value form is
   the one that carries weight.

4. **AC4 — the cookie is still the session.** For both endpoints, a test takes
   the `session` cookie from the `Set-Cookie` header, authenticates with it, and
   gets the expected session — proving establishment still works through the
   only remaining channel. Session-property assertions (e.g. the derived label)
   are made on the session reached this way.

5. **AC5 — no stale justification or stale doc left behind.** The `RawToken`
   imports in `web/src/auth/api.rs`, `web/src/registration/api.rs`, and
   `web/src/registration/component.rs` are correct for their new use, and
   **every** comment asserting the old contract is updated or removed. The full
   list, since "any comment" is too vague to check:
   - `web/src/auth/api.rs:9` — the ungated-import justification.
   - `web/src/auth/api.rs:28-31` — `LoginResponse`'s doc ("the raw session
     token…").
   - `web/src/auth/api.rs:38-39` — `login`'s doc, which **intra-doc-links
     `[`RawToken`]`**.
   - `web/src/registration/api.rs:14` — the ungated-import justification.
   - `web/src/registration/api.rs:47-48` — `register`'s doc ("Returns the
     freshly minted session [`RawToken`]").
   - `server/tests/web/web_auth.rs:30-31` — `extract_login`'s doc.
   - `web/src/auth/api.rs:108-110` — the inline comment about what
     `LoginResponse` carries.

   **Build hazard to watch:** if `RawToken` becomes
   `#[cfg(feature = "server")]`-gated in `auth/api.rs`, a surviving intra-doc
   link to it breaks the client build (and the `doc-links` gate step). Rewriting
   those docs is what prevents that, not an afterthought to it.

6. **AC6 — test names match behavior.** No test is left named `…returns_token`
   for an endpoint that no longer returns one. Note two tests needing edits are
   **not** catchable this way —
   `register_invite_only_valid_code_creates_user_marks_invite_used` and
   `login_returns_is_operator_flag` — so this criterion is checked against the
   eight-site inventory above, not by grepping names.

7. **AC7 — the app-password endpoint is untouched.** `web/src/sessions/api.rs`
   still returns its `RawToken`; the diff does not modify it.

8. **AC8 — the gate is green.** `cargo xtask validate` passes with **e2e**, and
   the coverage policy passes.

   To be precise about why: no e2e test reads either payload — they drive the
   login and registration forms through the UI — so nothing in the e2e suite
   asserts this behavior directly. e2e runs here because the change alters a
   server-fn return type on the auth path that those UI flows depend on, so the
   flows are what confirm nothing downstream broke. It is regression cover, not
   a conformance assertion.

9. **AC9 — the invariant is recorded where it can be found.** The "web session
   establishment is cookie-only" rule is written down somewhere durable, not
   left only in a spec that ship archives — see "Where the invariant lives"
   below.

## Out of scope

- Any change to the app-password token flow (`web/src/sessions/api.rs`).
- An xtask structural gate for token-returning server fns.
- Any bearer-token API for out-of-tree clients. If one is ever wanted it is a
  deliberate design, not a leak preserved by inertia.
