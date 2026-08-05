# `login` takes `Option<SessionLabel>` — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-08-04-issue-685-login-session-label.md`](../specs/2026-08-04-issue-685-login-session-label.md)
— the "what/why". This plan is the "how"; it does not restate the spec's
analysis.

**Goal:** Retype `web::auth::login`'s `label` wire arg from `Option<String>` to
`Option<SessionLabel>`, and let the newtype own the User-Agent branch's bounding
and defaulting.

**Architecture:** Two independently-reviewable changes to one function. Task 2
swaps the wire-arg type (routing client input through `SessionLabel`'s
validating serde bridge) and leaves the User-Agent branch alone; Task 3 then
deletes the hand-rolled 200-char cap and `"Unknown device"` literal from that
branch, leaving `SessionLabel::from_lossy` as the single owner of both. Each
task ends green.

**Tech Stack:** Rust, Leptos `#[server]` fns, `cargo nextest`, `rstest`
(`#[apply(backends)]` dual-backend template), `cargo xtask check`.

## Review header

**Scope — in:**

- `web/src/auth/api.rs` — the `login` signature, its `SessionLabel` import, the
  label-derivation block, and its doc comment.
- `server/tests/web/web_auth.rs` — one test updated, two added.

**Scope — out:** the login form UI (no label field is added),
`create_app_password` and the app-password form, `SessionLabel` itself,
`MAX_SESSION_LABEL_CHARS`, and realigning ADR-0065's stale secret-exception
wording (filed as a follow-up in Task 1).

**Tasks:**

1. File the separable ADR-0065 stale-wording concern as its own issue.
2. Retype `label` to `Option<SessionLabel>`; ungate the import; add the
   ADR-0065-rationale doc comment; add the whitespace-only and over-long
   rejection tests.
3. Let `from_lossy` own the User-Agent branch (drop the 200-cap and the
   literal); update the long-UA test to the 255 bound.

**Key risks / decisions:**

- **An empty `label=` is _not_ rejected** — the `Option<T>` form layer decodes
  it to `None` before `SessionLabel`'s deserializer runs, so it still falls back
  to the User-Agent. `login_with_empty_label_creates_session_without_label` must
  therefore pass **unchanged**; a plan that "fixes" it is wrong. (Spec §D2, with
  the falsified-premise note.)
- **The import must be ungated** — a wire-arg type is named in the `#[server]`
  signature on the client build too, so leaving `SessionLabel` inside
  `#[cfg(feature = "server")]` breaks the wasm build. `cargo xtask check` builds
  both; a host-only `cargo check` would not catch it.
- **Task 3 changes an observable bound** (a 250-char UA yields 250, not 200).

## Global Constraints

Copied from the spec and `CONTRIBUTING.md`; every task's requirements include
these.

- **Backend parity** — storage-touching tests use the dual-backend template
  (`#[apply(backends)]` + `#[case] backend: Backend`). A bare `#[tokio::test]`
  fails the `test-backend-pattern` guard.
- **`label` stays in `#[macros::server(skip(password, label))]`** — the #511
  tracing gate is unchanged (`Option<SessionLabel>` is non-recordable).
- **No custom deserializer or pre-parse shim** for the label — the
  `SessionLabel` serde bridge is the sole chokepoint (ADR-0065: never a
  re-implemented rule).
- **`SessionLabel::DEFAULT` is private to `common`** — `api.rs` must not name
  it.
- **A `Some(label)` is used directly** — never round-tripped through
  `from_lossy` or `String`.
- **Rejection convention** — a typed-wire-arg decode failure surfaces as
  `StatusCode::INTERNAL_SERVER_ERROR` (the existing session-fn convention).
- **Per-commit gate** — run `cargo xtask check` before each commit so the
  pre-commit hook passes clean (**`jaunder-commit`**). **No `Co-Authored-By`
  trailer.**
- **Crate name is `jaunder`, not `server`** — `server/Cargo.toml` declares
  `name = "jaunder"` with an `integration` test target, so it's
  `cargo nextest run -p jaunder`.
- **Bare `cargo nextest` can't run the postgres cases** — no local PG daemon, so
  `case_2_postgres` fails `ConnectionRefused`. Filter to sqlite while iterating
  (`-E 'test(web_auth) and test(sqlite)'`); `cargo xtask check` provisions
  PostgreSQL and covers both backends.

---

### Task 1: File the separable ADR-0065 concern

The cold spec review found that ADR-0065's secret exception says a secret's arg
"stays `String`", while the code passes a typed `ProfferedPassword`
(`web/src/auth/api.rs:43`). That is a real doc/code drift, but it is not this
issue — file it rather than folding it in.

**Files:** none (tracker-only).

**Interfaces:**

- Consumes: nothing.
- Produces: nothing later tasks depend on.

- [x] **Step 1: File the issue** using **`jaunder-issues`** (its type/label/
      milestone conventions govern; do not hand-roll them).

Title:
`ADR-0065's secret exception says the arg "stays String", but the code passes ProfferedPassword`

Body must state: ADR-0065 lines 76–79 carve out secrets from the typed-wire-arg
rule on the grounds that a secret's arg "stays `String`"; `login` in fact takes
a typed `ProfferedPassword` (an ADR-0063 inbound-secret newtype), so the ADR's
stated mechanism is stale even though its conclusion (secrets are special)
holds. Surfaced by the #685 spec review. Suggest realigning the ADR text to
describe what the code does — the exception is about _client-side pre-validation
and tracing_, not about the arg's type.

- [x] **Step 2: Record the issue number.** Filed as
      [#822](https://github.com/jaunder-org/jaunder/issues/822) — type `Task`,
      label `type-safety`, milestone "Code quality ratchet", priority P3, no
      blockers.

There is no commit for this task — the tracker is the deliverable. **Stage
explicit paths; never `git add -A`** — what lands must be what the gate checked.
(Observed in practice: the repo's pre-commit hook auto-stages, so the spec and
plan rode along into Task 2's commit `71cc8aef` rather than waiting for ship-time
archiving. The gate ran on the full tree, so the invariant holds.)

---

### Task 2: Retype `label` to `Option<SessionLabel>`

**Files:**

- Modify: `web/src/auth/api.rs:6-26` (imports), `:38-45` (doc comment +
  signature), `:68-91` (label derivation)
- Test: `server/tests/web/web_auth.rs`

**Interfaces:**

- Consumes: `common::session_label::SessionLabel` (existing; validating
  `FromStr`, serde bridge, `from_lossy`).
- Produces:
  `pub async fn login(username: Username, password: ProfferedPassword, label: Option<SessionLabel>) -> WebResult<LoginResponse>`
  — the signature Task 3 edits the body of.

- [x] **Step 1: Write the failing tests**

Add both to `server/tests/web/web_auth.rs`, after
`login_with_empty_label_creates_session_without_label` (which is **not**
touched).

```rust
#[apply(backends)]
#[tokio::test]
async fn login_rejects_whitespace_only_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    // A whitespace-only label is rejected at the typed-wire-arg decode
    // (SessionLabel's FromStr trims, then rejects empty) — it no longer falls
    // through to the User-Agent branch. Surfaces as 500, the session-fn convention.
    let (status, _, body) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=alice&password=password123&label=%20%20",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    // Decode fails before the handler body runs, so no session is minted.
    assert!(!body.contains("\"token\""), "token minted: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn login_rejects_overlong_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    // Past MAX_SESSION_LABEL_CHARS (255) the label is rejected at decode rather
    // than silently truncated, matching create_app_password_rejects_overlong_label.
    let overlong = "a".repeat(256);
    let (status, _, body) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        format!("username=alice&password=password123&label={overlong}"),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.contains("\"token\""), "token minted: {body}");
}
```

The `!body.contains("\"token\"")` assertion is deliberately at parity with the
precedent tests (`web_sessions.rs:190-226`), which assert status only. A
stronger "the user has zero sessions" check isn't worth diverging from the house
pattern: a decode failure never reaches the handler body, so no session can be
minted.

- [x] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd <worktree> -- cargo nextest run -p jaunder login_rejects_`

Expected: FAIL — both return `200 OK` today (whitespace trims to empty → UA
branch; over-long is truncated by `from_lossy`), so the `INTERNAL_SERVER_ERROR`
assertion fails.

- [x] **Step 3: Implement against the tests**

Three edits to `web/src/auth/api.rs`:

1. **Ungate the import.** Remove `common::session_label::SessionLabel,` from the
   `#[cfg(feature = "server")]` block (`:17-26`) and add it to the ungated group
   (`:11-13`), then extend the comment at `:7-10` so it covers the new member —
   it currently explains why `Username`/`ProfferedPassword`/`RawToken` are
   ungated; `SessionLabel` is ungated for the same reason (named in the
   `#[server]` signature on both builds).

2. **Signature + doc comment** (`:38-45`). Keep
   `#[macros::server(skip(password, label))]` exactly as-is. Change `label` to
   `Option<SessionLabel>` and append to the doc comment:

```rust
/// `label` is a typed wire arg (ADR-0065): the `SessionLabel` serde bridge trims
/// it and rejects whitespace-only or over-long values at decode. It has **no client-side
/// `Field<SessionLabel>` on the login form** — that form collects only username
/// and password, so a browser omits `label` entirely and always takes the
/// User-Agent branch below. ADR-0065's client-pre-validation requirement is
/// therefore vacuous here, not violated. (The app-password form, which *does*
/// collect a label, has its `Field::<SessionLabel>` at
/// `web/src/sessions/component.rs:77`.) An omitted *or empty* `label` decodes to
/// `None`: the `Option` form layer absorbs a present-but-empty field before
/// `SessionLabel`'s deserializer runs.
```

3. **Label derivation** (`:68-91`). Replace the `derived_label`
   if/else-plus-`from_lossy` block with an `if let` that uses a `Some` directly
   and confines `from_lossy` to the UA branch. The User-Agent branch keeps its
   200-cap and literal **for now** — Task 3 removes them, so this task's diff
   stays about the type.

   **Use `if let`, not `match`.** A `match` on one destructured pattern with a
   block arm trips `clippy::single_match_else`, which is `-D warnings` here — the
   gate rejects it, and suppressing a lint needs user approval:

```rust
    // An explicit client-supplied label arrives already validated (typed wire arg),
    // so it is used as-is; otherwise derive a device name from the User-Agent.
    let session_label = if let Some(label) = label {
        label
    } else {
        let ua = leptos_axum::extract::<axum::http::HeaderMap>()
            .await
            .ok()
            .and_then(|headers| {
                headers
                    .get("user-agent")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Unknown device".to_string());
        let ua = if ua.len() > 200 {
            ua.chars().take(200).collect::<String>()
        } else {
            ua
        };
        SessionLabel::from_lossy(&ua)
    };
```

- [x] **Step 4: Run the tests, verify they pass** — 26 sqlite `web_auth` tests
      green (postgres cases need the Nix gate, run at Step 5).

Run: `devtool run --cwd <worktree> -- cargo nextest run -p jaunder web_auth`

Expected: PASS — the two new tests now 500, and
`login_with_label_creates_session_with_label`,
`login_with_empty_label_creates_session_without_label`, and
`login_truncates_long_user_agent` all still pass unchanged.

- [x] **Step 5: Commit** — `71cc8aef`. The gate caught
      `clippy::single_match_else` on the first run; fixed by switching the
      `match` to `if let` (see Step 3.3), then green.

Run `cargo xtask check` first (**`jaunder-commit`**) — it builds the wasm/CSR
target too, which is what proves the ungated import (Step 3.1) is correct.

```bash
git add web/src/auth/api.rs server/tests/web/web_auth.rs
git commit -m "refactor(web): type login's label as Option<SessionLabel> (#685)"
```

---

### Task 3: Let `from_lossy` own the User-Agent branch

**Files:**

- Modify: `web/src/auth/api.rs` (the `None =>` arm from Task 2)
- Test: `server/tests/web/web_auth.rs:432-470`

**Interfaces:**

- Consumes: the `match label { Some(..) => .., None => .. }` block Task 2
  produced.
- Produces: nothing later tasks depend on (final task).

- [ ] **Step 1: Update the test to the new bound**

Replace `login_truncates_long_user_agent` (`:432-470`) — its stale `M2.9.12`
comment and 200-char assertions included — with:

```rust
// A long User-Agent is bounded by MAX_SESSION_LABEL_CHARS (255), the newtype's
// own cap, rather than a second hand-rolled 200-char cap in `login` (#685).
#[apply(backends)]
#[tokio::test]
async fn login_bounds_long_user_agent_at_session_label_cap(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    // 250 < 255, so this UA survives intact — under the old 200-char cap it did not.
    let long_ua = "a".repeat(250);
    let (status, _, body) = post_form_with_ua(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        &long_ua,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = extract_login(&body).0;
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.label, "a".repeat(250).as_str());
}

// Past the cap, the UA is truncated (not rejected): it is an internally derived
// value, so it goes through the lossy door (ADR-0063 §2), unlike a submitted label.
#[apply(backends)]
#[tokio::test]
async fn login_truncates_user_agent_past_session_label_cap(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    let long_ua = "a".repeat(300);
    let (status, _, body) = post_form_with_ua(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        &long_ua,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = extract_login(&body).0;
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.label.chars().count(), MAX_SESSION_LABEL_CHARS);
}
```

Add the import for the cap at the top of `server/tests/web/web_auth.rs`:

```rust
use common::session_label::MAX_SESSION_LABEL_CHARS;
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p jaunder user_agent`

Expected: FAIL — **both** new tests fail against the hand-rolled 200-char cap:
`login_bounds_long_user_agent_at_session_label_cap` gets a 200-char label, not
250, and `login_truncates_user_agent_past_session_label_cap` gets 200, not
`MAX_SESSION_LABEL_CHARS` (255). Two failures here is the expected state, not a
sign something is wrong.

- [ ] **Step 3: Implement against the tests**

In `web/src/auth/api.rs`, replace the `None =>` arm's body with the version that
hands the raw User-Agent straight to `from_lossy` — no `.take(200)`, no
`"Unknown device"` literal. An absent header becomes an empty string, which
`from_lossy` turns into its own default:

```rust
        None => {
            // The User-Agent is an internally derived hint, not submitted input, so
            // it goes through the lossy door (ADR-0063 §2): `from_lossy` trims,
            // bounds it at MAX_SESSION_LABEL_CHARS, and supplies the "Unknown
            // device" default when there is no usable header. Both the cap and the
            // default live in `SessionLabel` — never duplicated here.
            let ua = leptos_axum::extract::<axum::http::HeaderMap>()
                .await
                .ok()
                .and_then(|headers| {
                    headers
                        .get("user-agent")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            SessionLabel::from_lossy(&ua)
        }
```

Also drop "capped at 200 chars with an `"Unknown device"` default" from the
comment above the `match` if Task 2 left any trace of it.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p jaunder web_auth`

Expected: PASS — both UA tests, plus the unchanged
`login_with_empty_label_creates_session_without_label` (no UA header is sent, so
`unwrap_or_default()` yields `""` → `from_lossy` → `"Unknown device"`).

- [ ] **Step 5: Commit**

Run `cargo xtask check` first (**`jaunder-commit`**).

```bash
git add web/src/auth/api.rs server/tests/web/web_auth.rs
git commit -m "refactor(web): let SessionLabel own login's UA bound and default (#685)"
```

---

## Verification

- [ ] `devtool run --cwd <worktree> -- cargo xtask validate` is green — static +
      clippy + coverage + all four `{sqlite,postgres}×{chromium,firefox}` e2e
      combos. No e2e change is expected: the suite never sends a label and its
      seeded-auth path (ADR-0098) bypasses `login` entirely.
- [ ] `rg 'Unknown device' web/src/auth/api.rs` returns nothing (AC8).
- [ ] `rg 'take\(200\)|> 200' web/src/auth/api.rs` returns nothing (AC3). Match
      the truncation shape, not a bare `200` — a future issue number containing
      those digits would make a bare-number grep a false positive.
