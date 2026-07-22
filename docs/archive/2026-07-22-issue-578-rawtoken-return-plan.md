# Plan — #578: `login` / `register` return `RawToken`

Spec:
[2026-07-22-issue-578-rawtoken-return.md](../specs/2026-07-22-issue-578-rawtoken-return.md)
· Issue [#578](https://github.com/jaunder-org/jaunder/issues/578)

## Review header

**Goal.** Return `WebResult<RawToken>` from `login` and `register` (was
`String`), and update the two client type-annotations that name the return type.

**Scope.** In: the two `#[server]` return types + bodies + ungated imports; the
two client `Result<String, WebError>` annotations. Out: inbound password args,
`create_session`, `set_session_cookie`, cookie attributes, the generator.

**One task** (the whole change is a cohesive boundary retype; no ordering to
sequence).

**Key risks / decisions.** `raw_token` is already a `RawToken`, so
`Ok(raw_token)` needs no wrap. `RawToken` serde borrow-serializes identically to
a `String`, so the wire is unchanged and `extract_token`-based integration
tests + auth e2e still pass. Security-adjacent → full `code-review` at the
deliverable boundary.

**For agentic workers:** execute with **jaunder-iterate** (small enough to do
inline).

## Global constraints

- No `Co-Authored-By` trailer. `cargo xtask check` clean before commit (hook
  enforces).
- Closes with `cargo xtask validate --no-e2e`.

## Task 1 — type the token return

**Files**

- `web/src/auth/api.rs`:
  - Add ungated `use common::token::RawToken;` (with the other ungated wire-type
    imports — `Username`, `ProfferedPassword`).
  - `login(…) -> WebResult<RawToken>` (was `WebResult<String>`).
  - Body: `Ok(raw_token.to_string())` → `Ok(raw_token)`.
  - Doc comment: "Returns the raw session token …" → note it returns the typed
    `RawToken` (still sets the `session` cookie).
- `web/src/registration/api.rs`: the same four edits for `register`.
- `web/src/auth/component.rs` (LoginPage, ~line 75):
  `.map(|r: Result<String, WebError>| …)` → `Result<RawToken, WebError>` (the
  `Ok(_)` arm unchanged); add `use common::token::RawToken;`.
- `web/src/registration/component.rs` (RegisterPage, ~line 141):
  `.and_then(|r: Result<String, WebError>| r.err())` →
  `Result<RawToken, WebError>`; add the import.

**Test**

- No new unit test (the retype is behavior-preserving; `RawToken`'s
  serde/redaction are covered by its own `common::token` tests). If the gate
  surfaces a web test asserting the return as `String`, update it to `RawToken`.
- `cargo check -p web` and `-p web --features server` compile (the type must
  resolve on both client and server builds).

**Run / final gate**

- `cargo xtask validate --no-e2e` — green. Confirm
  `rg -n "raw_token.to_string\(\)" web/src` returns nothing and both fns show
  `WebResult<RawToken>`.

**Commit:**
`refactor(web): login/register return typed RawToken, not String (#578)`

## Self-review

- The cold spec review already verified every edit site, the completeness of the
  client-caller set (exactly the two annotated sites; unannotated effects
  infer), that `raw_token` is already a `RawToken`, and that the wire format is
  unchanged — so a separate plan-review subagent would re-tread the same ground;
  the deliverable-boundary `code-review` (full, per security-adjacency) is the
  substantive check.
- Every spec acceptance criterion maps to Task 1: typed returns + no `to_string`
  → the api.rs edits; client consumers compile → the component.rs edits;
  `validate --no-e2e` → the gate.
