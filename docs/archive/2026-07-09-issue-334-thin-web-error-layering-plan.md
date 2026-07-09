# Thin web shell — error carrier / wire split — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax; tick
> them in real time.

**Goal:** Move the server-side error carrier (`InternalError` +
`ErrorKind`/`ErrorClass` + boundary log/metric) out of `web` into the `host`
crate, decoupled from the `WebError` wire type, make all error lifts typed, and
push the pure/effectful `auth`/`viewer`/`posts` server-only logic down out of
`web`.

**Architecture:** Three-tier
`T1 (storage/common domain errors) → T2 (host::InternalError carrier) → T3 (web::WebError wire)`
conversion pipeline. `web` keeps only the wire types, the `kind→WebError`
projection, the leptos owner-pinning boundary, and `#[server]`/UI. See the spec:
**`docs/superpowers/specs/2026-07-08-issue-334-thin-web-error-layering.md`** —
this plan is "how"; the spec is "what/why". Task IDs below cite spec decisions
(D1–D7) and acceptance criteria (A1–A20).

**Tech Stack:** Rust, leptos server functions, `sqlx`, `anyhow`, `tracing`,
`cargo nextest`, `cargo xtask` gate.

## Global Constraints

- **Backend parity (CONTRIBUTING.md):** storage-touching tests use the
  dual-backend template; a bare `#[tokio::test]` that should be dual-backend
  fails the `test-backend-pattern` guard. Never put tests in ADR-0019
  per-backend dialect files.
- **Coverage policy (ADR-0050):** new host-coverable lines need coverage or a
  justified `// cov:ignore` / CRAP marker; `#[component]` exemption is
  syntactic. `web` UI is host-compiled but coverage-exempt.
- **Wire compatibility:** `WebError` JSON encoding is stable snake_case, pinned
  by an existing test; the `kind→WebError` projection must reproduce today's
  `into_public()` output **byte-for-byte** (A5/A7).
- **`host` floor invariant (ADR-0058, as clarified by this cycle):** `host`
  depends on **no workspace crate except `common`**; it may take external infra
  crates (`anyhow`, `tracing`, `sqlx`, `http`) but **not** `chrono`, and must
  **not** name a workspace-`storage` abstraction (`PostStorage`,
  `AudienceError`, …).
- **Commits:** the pre-commit hook runs `cargo xtask check`; run it first so it
  passes clean (**jaunder-commit**). **No `Co-Authored-By` trailer.** One clean
  commit per task.
- **Behavior-preserving:** every error-handling change keeps the wire projection
  identical; only operator-side (typed source) improves. The existing no-leak +
  JSON tests are the guard.

---

## Review header

**Scope — in:** the ADR; relocating the T2 carrier to `host::error` decoupled
from `WebError`; the `web` projection; the `boundary!` split (owner-pinning
stays in `web`, log+metric → `host`); uniform typed `From`/constructor error
handling (incl. `sqlx`/`chrono`/`TaggingError`/`PostFormat`); pushing down the
`auth`/`viewer`/`posts` server-only clusters per the spec's D5 inventory.

**Scope — out:** UI co-location (`pages/` → co-located modules) — that's
#303/#314; #314 rebases on top. No `#[component]`/`pages/` moves. Remaining
verticals' push-down (email/media/backup/…) is as-we-go future work, **not**
filed as new issues (covered by the #303 umbrella + this issue's recorded
principle).

**Tasks:**

1. ✅ ADR draft + commit planning docs & the ADR-0058 clarification.
2. ✅ Decouple `InternalError` from `WebError` in place (in `web`).
3. ✅ Relocate the carrier to `host::error`; split the boundary.
4. ✅ Uniform external/common error handling — `host` `From` impls +
   de-stringify.
5. ✅ Domain-error `From` impls in `storage`; delete `web` mappers.
6. ✅ Push down `viewer.rs`.
   - ➕ (pulled-forward DRY) `InternalError::validation_source(msg, source)`
     constructor; route chrono/email lifts + `validation_from!` macro + storage
     `From`s through it.
7. ✅ Push down `auth/server.rs`.
8. ✅ Push down `posts/server.rs`.
9. ✅ Acceptance sweep + full `validate` (green: all static + coverage clean +
   full e2e matrix).

**Key risks/decisions:**

- Tasks 2→3 are ordered so the carrier is _decoupled in place first_ (semantic,
  behavior-tested), then _relocated mechanically_ (compile-driven) — neither is
  a big-bang.
- `web` re-exports `host::error::*` so per-vertical call sites
  (`InternalError::storage(…)`, `?`) keep compiling across the move without
  touching every file in one task.
- `From<sqlx::Error>` lives in `host` (adds `sqlx` dep — sanctioned by the
  clarified ADR-0058); `chrono` gets **no** `From` (site-specific messages), so
  `host` needs no `chrono`.
- Every task ends green (`cargo xtask check`); Task 9 is the full `validate` +
  acceptance audit.

---

### Task 1: ADR draft + planning-doc commit

**Files:**

- Create: `docs/adr/0059-thin-web-shell-error-layering.md` (numberless draft;
  `cargo xtask adr promote` numbers it → ADR-0059 at ship)
- Already-modified (commit here): `docs/adr/0058-host-crate-layering.md` (the
  dependency-rule clarification),
  `docs/superpowers/specs/2026-07-08-issue-334-thin-web-error-layering.md`, this
  plan.

**Interfaces:** none (docs only).

- [x] **Step 1: Write the ADR draft** using **jaunder-adr**. Required lead
      content (spec "ADR plan" section, verbatim intent): the two
      non-collapsible boundaries — **T2↔T3** security boundary made structural
      (operator payload structurally absent from the wire type ⇒ no-leak by
      construction), **T1↔T2** typed discrete cause vs uniform carrier. Then:
      the thin-web-shell invariant; conversion-not-containment; carrier in
      `host` decoupled from `WebError`; the `host` floor invariant (extends
      ADR-0058, adds `storage`/`web` dependents); the `InternalError` name
      decision (D7 — resolve here: keep `InternalError`, now
      `host::error::InternalError`). Reference ADR-0017 (its "forthcoming
      structured carrier" thread) and ADR-0016.
- [x] **Step 2: Prettier-format the Markdown** (project pre-commit reformats
      prose):
      `devtool run -- prettier -w docs/adr/0059-thin-web-shell-error-layering.md docs/superpowers/specs/2026-07-08-issue-334-thin-web-error-layering.md docs/superpowers/plans/2026-07-09-issue-334-thin-web-error-layering.md docs/adr/0058-host-crate-layering.md`
- [x] **Step 3: Commit.**

The ADR draft under `docs/adr/drafts/` is **gitignored** (draft-out-of-git flow,
#219) — do **not** `git add` it; it enters history at ship via
`cargo xtask adr promote`. Commit only the tracked docs:

```bash
git add docs/adr/0058-host-crate-layering.md docs/superpowers/specs/2026-07-08-issue-334-thin-web-error-layering.md docs/superpowers/plans/2026-07-09-issue-334-thin-web-error-layering.md
git commit -m "docs(#334): thin-web-shell error-layering spec + plan; clarify ADR-0058 dep rule"
```

---

### Task 2: Decouple `InternalError` from `WebError` (in `web`, in place)

Semantic change, fully inside `web/src/error.rs`; behavior preserved (wire
output identical). This is where the decoupling is _proven_; Task 3 is then a
mechanical move.

**Files:**

- Modify: `web/src/error.rs` (struct `InternalError` ~155–161; constructors
  ~185–317; `From<WebError>`/`kind_class_for` ~326–354; `server_boundary`
  ~458–465; `log_boundary_failure` ~473–501; tests ~503+)
- Modify — **every remaining call site of the removed/changed API**
  (`.public()`, `.into_public()`, and `masked(WebError, …)`), **production _and_
  test**, or the tree won't compile at this commit. As of writing,
  `rg -n '\.public\(\)|\.into_public\(\)|::masked\(' web/src` covers:
  `web/src/auth/server.rs` (`classify_current_user`), `web/src/backup/mod.rs`
  (2), `web/src/backup/server.rs` (1), `web/src/posts/server.rs`
  (`private_post_not_found_error` **production** + ~11 test sites),
  `web/src/audiences/mod.rs` (2). **Re-run that `rg` at execution time — do not
  trust this list to be exhaustive.**
- Test: `web/src/error.rs` `#[cfg(test)]`

**Interfaces:**

- Produces (the new `web::error` surface consumed unchanged by every vertical):
  - `pub struct InternalError { kind: ErrorKind, class: ErrorClass, context: Vec<(&'static str, String)>, public_message: String, source: Option<anyhow::Error> }`
    — **no `public: WebError` field**.
  - Constructors unchanged in signature:
    `unauthorized/not_found/validation/conflict(impl Into<String>)`,
    `storage/server/external(impl Error + Send + Sync + 'static)`,
    `server_boxed(Box<dyn Error + Send + Sync>)`,
    `server_message(impl Into<String>)`, `with_context`.
  - Changed:
    `pub fn masked(kind: ErrorKind, class: ErrorClass, public_message: impl Into<String>, source: anyhow::Error) -> Self`
    (was `masked(WebError, impl Into<String>)`).
  - `pub fn kind(&self) -> ErrorKind`, `pub fn class(&self) -> ErrorClass`,
    `pub fn context(&self) -> &[(&'static str, String)]`,
    `pub fn public_message(&self) -> &str`,
    `pub fn operator_message(&self) -> String`. **Removed:**
    `public(&self) -> &WebError`, `into_public(self) -> WebError`.
  - `pub(crate) fn project(kind: ErrorKind, public_message: &str) -> WebError` —
    the total D2 map.
  - **Deleted:** `impl From<WebError> for InternalError`, `fn kind_class_for`.

- [ ] **Step 1: Write/adjust the failing tests** in `web/src/error.rs` tests.
      The pure-`WebError` JSON-encoding test is literally unchanged; the no-leak
      test and any test calling `into_public()`/`public()` need a **mechanical
      rewrite** to `project(e.kind(), e.public_message())` (behavior asserted
      must be identical). Add:

```
test "project is the total kind→WebError map":
    project(Auth, "ignored")        == WebError::Unauthorized
    project(NotFound, "x not found")== WebError::NotFound { message: "x not found" }
    project(Validation, "bad")      == WebError::Validation { message: "bad" }
    project(Conflict, "dupe")       == WebError::Conflict { message: "dupe" }
    project(Storage, "storage operation failed") == WebError::Storage { message: "storage operation failed" }
    project(Internal, "server operation failed") == WebError::Server  { message: "server operation failed" }
    project(External, "server operation failed") == WebError::Server  { message: "server operation failed" }
test "masking constructors set generic public_message + preserve source":
    let e = InternalError::storage(SourceError);
    e.kind() == Storage; project(e.kind(), e.public_message()) == WebError::Storage{ "storage operation failed" }
    e.operator_message() contains "source context"   # source preserved, not leaked to public
test "server_boundary Err path projects the same wire error as before":
    # drive server_boundary with a body returning Err(InternalError::not_found("Post")),
    # assert Err(WebError::NotFound{ message: <today's not_found("Post") string> })
```

- [ ] **Step 2: Run, verify FAIL.**
      `devtool run -- cargo nextest run -p web --features server error::` → FAIL
      (`project`/`public_message` undefined; `masked`/`public()` signature
      mismatch).

- [ ] **Step 3: Implement the decoupling.** Contract is pinned by Step 1 + the
      retained no-leak/JSON tests. Concretely:
  - Swap the struct field `public: WebError` → `public_message: String`.
  - Each masking constructor (`storage`/`server`/`server_message`/`external`)
    sets `public_message` to its current literal (`"storage operation failed"` /
    `"server operation failed"`) and the same `kind`/`class`;
    `unauthorized(msg)` →
    `Self { kind: Auth, class: Client, public_message: String::new(), source: Some(anyhow::Error::msg(msg.into())), context: vec![] }`.
  - `not_found`/`validation`/`conflict(message)` set `kind`/`class` = the
    `kind_class_for` values and `public_message = message.into()` directly
    (inline the deleted `From<WebError>` body); `source: None`.
  - New `masked(kind, class, public_message, source)` stores fields directly.
  - `project(kind, public_message)` = the Step-1 table (the inverse of the
    deleted `kind_class_for`, message-carrying).
  - `operator_message` = `render_operator_message` rewritten to
    `match source { Some(s)=>format!("{s:#}"), None => self.public_message.clone() }`.
  - Delete `From<WebError>` + `kind_class_for`.
  - `server_boundary` Err arm:
    `log_boundary_failure(...); common::metrics::error(...); Err(project(error.kind, &error.public_message))`.
  - `log_boundary_failure`: change the `error.public = ?error.public` field to
    `error.public = %error.public_message`.
  - `classify_current_user` (auth/server.rs) and the two `backup/mod.rs` sites:
    `matches!(error.public(), WebError::Unauthorized)` →
    `error.kind() == ErrorKind::Auth`.
  - **Migrate every other changed-API site** the `rg` from Files found:
    `.into_public()` → `project(e.kind(), e.public_message())`;
    `masked(WebError::X(msg), op)` →
    `masked(<kind>, <class>, <public_message>, anyhow::Error::msg(op))`
    preserving current output (notably `private_post_not_found_error` production
    — its `set_not_found_status` split is deferred to Task 8; here only migrate
    the `masked` call). Compiler-driven: fix until `cargo check` is clean.

- [ ] **Step 4: Run, verify PASS.**
      `devtool run -- cargo nextest run -p web --features server` → PASS (**the
      full web suite**, not just `error::` — this is what proves the changed-API
      migration compiles and every vertical's wire output is unchanged). Then
      `devtool run -- cargo xtask check --no-test`. Audit:
      `rg -n '\.public\(\)|\.into_public\(\)' web/src` → nothing;
      `rg -n '::masked\(' web/src` shows only the new 4-arg form.

- [ ] **Step 5: Commit.**

```bash
git add web/src/error.rs web/src/auth/server.rs web/src/backup/mod.rs
git commit -m "refactor(web): decouple InternalError from WebError; project kind→wire (#334)"
```

---

### Task 3: Relocate the carrier to `host::error`; split the boundary

Mechanical move; no semantic change. `web` keeps the wire type, projection, and
owner-pinning; `host` gains the carrier + log/metric.

**Files:**

- Create: `host/src/error.rs`
- Modify: `host/src/lib.rs` (`pub mod error;`), `host/Cargo.toml` (deps),
  `web/src/error.rs` (remove moved items; re-export), `web/Cargo.toml` (`host`
  dep, server-gated)
- Test: `host/src/error.rs` `#[cfg(test)]` (moved carrier unit tests);
  `web/src/error.rs` keeps boundary/projection/owner tests.

**Interfaces:**

- Consumes: Task 2's decoupled `InternalError`/`ErrorKind`/`ErrorClass`.
- Produces:
  - `host::error::{InternalError, ErrorKind, ErrorClass, InternalResult, BoxedError-internal}`
    with the Task-2 constructors/accessors, plus a new method
    `pub fn emit_boundary_failure(&self, server_fn: &'static str)` = the moved
    `log_boundary_failure` **and**
    `common::metrics::error(self.kind.as_metric_str(), self.class.as_metric_str())`
    (accesses private fields; same module).
  - `web::error` re-exports:
    `pub use host::error::{InternalError, ErrorKind, ErrorClass, InternalResult};`
    so all `InternalError::storage(…)`/`?` call sites are unchanged.
  - `web::error` retains `WebError`, `WebResult`, `pub(crate) fn project`,
    `server_boundary`, `server_resource`, `owner_ancestry_strong`,
    `scoped_fetcher_future`.

- [ ] **Step 1: Move the carrier + write host tests.** Cut `InternalError`,
      `ErrorKind`, `ErrorClass`, `BoxedError`, the constructors/accessors,
      `render_operator_message`, and (as `emit_boundary_failure`)
      `log_boundary_failure`+metric into `host/src/error.rs`. Move the
      _carrier-only_ unit tests (constructor kind/class, masked,
      operator_message, no-leak-of-source) to `host/src/error.rs` tests. Add
      `host/src/lib.rs`: `pub mod error;`.

- [ ] **Step 2: Wire deps + re-exports.** `host/Cargo.toml` `[dependencies]`:
      `anyhow.workspace = true`, `tracing.workspace = true`,
      `common = { path = "../common", features = ["metrics"] }`.
      `web/Cargo.toml`: add `host = { path = "../host", optional = true }` and
      include `"dep:host"` in the `server` feature. In `web/src/error.rs` add
      the re-exports and rewrite `server_boundary`'s Err arm to
      `error.emit_boundary_failure(server_fn); Err(project(error.kind(), error.public_message()))`.

- [ ] **Step 3: Run — host then web.**
      `devtool run -- cargo nextest run -p host error::` → PASS (carrier tests
      now host-side — the testability win, A12 for the carrier).
      `devtool run -- cargo nextest run -p web --features server error::` → PASS
      (projection/boundary/owner_lifetime tests).
      `devtool run -- cargo xtask check --no-test`.

- [ ] **Step 4: Verify the floor invariant (A3/A4).**
      `rg -n 'storage|leptos|axum|chrono' host/Cargo.toml` → no such deps;
      `rg -n 'use storage|WebError' host/src` → nothing.

- [ ] **Step 5: Commit.**

```bash
git add host/ web/src/error.rs web/Cargo.toml Cargo.lock
git commit -m "refactor(host): relocate the InternalError carrier to host::error; split boundary log/metric (#334)"
```

---

### Task 4: Uniform external/common error handling — `host` `From` impls + de-stringify

Adds typed `From`s for the _external/common_ sources in `host`, rewires the bulk
call sites to `?`, and converts `chrono` sites to a source-preserving
constructor. (A18/A19; spec D5 error rule.)

**Files:**

- Modify: `host/src/error.rs` (add `From` impls; chrono uses the existing
  `masked`), `host/Cargo.toml` (`sqlx` dep)
- Modify: the bulk sites —
  `web/src/{subscriptions,posts,site,tags,backup,audiences,auth,profile,invites,password_reset,email,media,sessions}/*.rs`,
  `web/src/posts/{listing.rs,server.rs}` (exact anchors: the ~70
  `.map_err(InternalError::storage)` /
  `.map_err(|e| …validation(e.to_string()))` sites enumerated in the spec's
  inventory pass)
- Test: `host/src/error.rs` `#[cfg(test)]`

**Interfaces:**

- Produces (in `host::error`):
  - `impl From<sqlx::Error> for InternalError` — body identical to
    `InternalError::storage(e)` (kind `Storage`, source preserved).
    Behavior-preserving.
  - `impl From<common::slug::InvalidSlug> for InternalError`,
    `impl From<common::…::UsernameParseError> for InternalError`,
    `impl From<common::tag::TagValidationError> for InternalError`,
    `impl From<common::mailer::MailError> for InternalError` — each maps to the
    current wire class of its call sites (validation→`Validation`,
    mail→`External`), preserving the typed source.
  - **No new constructor for chrono.** The reworked
    `masked(kind, class, public_message, source)` (Task 2) already pairs a
    specific public message with an `anyhow` source, so chrono sites route
    through it directly (Step 3) — the anyhow `source` chain _is_ the
    `.context()` mechanism, on the operator side, while the public message stays
    on the wire.

- [ ] **Step 1: Write host `From`/constructor tests.**

```
test "From<sqlx::Error> == storage()":
    let e: InternalError = sqlx::Error::RowNotFound.into();
    e.kind()==Storage; e.class()==Bug; project(e.kind(), e.public_message())==WebError::Storage{"storage operation failed"}
test "From<InvalidSlug> preserves source, class Client, kind Validation":
    let e: InternalError = InvalidSlug::Empty.into();
    e.kind()==Validation; e.operator_message() contains the InvalidSlug display
test "masked pairs a site validation message with an anyhow source (chrono case)":
    let src = chrono_parse_err();
    let e = InternalError::masked(Validation, Client, format!("invalid publish_at: {src}"), anyhow::Error::new(src));
    project(e.kind(), e.public_message()) == WebError::Validation{ <the same site string> }
    e.operator_message() contains the chrono error text   # typed source on the anyhow chain, downcastable
```

- [ ] **Step 2: Run, verify FAIL.**
      `devtool run -- cargo nextest run -p host error::from` → FAIL.

- [ ] **Step 3: Implement + rewire — gated by a class-preservation audit.** Add
      `sqlx.workspace = true` to `host/Cargo.toml`; write the `From` impls +
      `validation_with_source`. **Before rewiring, audit each candidate site**
      (`rg` the `.map_err(InternalError::storage)` /
      `.map_err(|e| …validation(e.to_string()))` sites): record its _source
      type_ and its _current_ `(kind, class)`. Convert a site to bare
      `?`/`.into()` **only where the source's `From`-target class equals the
      site's current class** — `sqlx::Error` sites are `Storage` today and
      `From<sqlx::Error>` is `Storage` (match → `?`); common-local validation
      sites are `Validation` today and `From<InvalidSlug>`/`From<Tag/Username>`
      are `Validation` (match → `?`). **Any site whose current class differs
      from its source's `From` target** — e.g. a validation-y source funnelled
      through `InternalError::storage` (forcing `Storage`/500 today) — is **left
      as an explicit constructor** preserving today's class, never silently
      flipped. `chrono` sites →
      `.map_err(|e| InternalError::masked(ErrorKind::Validation, ErrorClass::Client, <exact current public string, e.g. format!("invalid publish_at: {e}")>, anyhow::Error::new(e)))?`
      — wire string unchanged, typed source now on the `anyhow` chain (the
      `.context()` mechanism), no `chrono` dep on `host`. Leave heterogeneous
      `server_boxed`/`Box<dyn Error>` sites as-is. **This task may be committed
      per-vertical** (one reviewable diff each) if the sweep is large — the
      deliverable is "all external/common lifts typed, no wire class flipped."

- [ ] **Step 4: Run, verify PASS.**
      `devtool run -- cargo nextest run -p host error::from` → PASS.
      `devtool run -- cargo nextest run -p web --features server` → PASS (no
      wire change). `rg -n "\.to_string\(\)\)\)" web/src | rg validation` → no
      lossy validation-stringify sites remain (A19).
      `devtool run -- cargo xtask check --no-test`.

- [ ] **Step 5: Commit.**

```bash
git add host/ web/src Cargo.lock
git commit -m "refactor(#334): typed From<sqlx/common> in host; de-stringify chrono/validation lifts"
```

---

### Task 5: Domain-error `From` impls in `storage`; delete `web` mappers

Collapses the named per-vertical domain mappers into
`impl From<E> for InternalError` in `storage`; call sites become `?`/`.into()`.
(A11/A15/A18; D4.)

**Files:**

- Modify: `storage/Cargo.toml` (`host` dep), and the storage modules owning each
  source enum — `storage/src/{audiences,users,atomic,post_service,posts}.rs`
  (add `impl From<…> for host::error::InternalError`)
- Modify/delete in `web`: `web/src/audiences/mod.rs` (`map_audience_error`
  ~:52), `web/src/auth/server.rs`
  (`register_open_error`/`register_invite_error`/`login_error`),
  `web/src/posts/server.rs` (`perform_update_error`/`perform_creation_error`),
  `web/src/posts/mod.rs` (inline `match` ~:469/:644, the `PostFormat` parse
  ~:243), the `TaggingError` lift in `apply_post_tag_diff`.
- Test: dual-backend storage tests where a mapping has non-trivial branch logic
  (e.g. `AudienceError::DuplicateName→Conflict`), else host-side unit tests over
  the enum.

**Interfaces:**

- Produces (in `storage`, one per enum, each reproducing its current web
  mapper's `(kind, class, public_message)`):
  `impl From<AudienceError|CreateUserError|UserAuthError|RegisterWithInviteError|PerformUpdateError|PerformCreationError|UpdatePostError|TaggingError> for host::error::InternalError`,
  plus `impl From<<PostFormat parse error>>`. `TaggingError` → kind
  `Internal`/public `Server` (behavior-preserving).
- Removed from `web`: the eight mapper fns above; call sites use `?`/`.into()`.

- [ ] **Step 1: Write the mapping tests** (dual-backend where a DB error drives
      a branch; else plain unit). Example for audiences:

```
#[storage_test]  // dual-backend template, CONTRIBUTING backend parity
async fn from_audience_error_maps_variants(env):
    From<AudienceError>(DuplicateName)     -> kind Conflict, project == WebError::Conflict{..}
    From<AudienceError>(NotFound)          -> kind NotFound, project == WebError::NotFound{..}
    From<AudienceError>(Storage(sqlx_err)) -> kind Storage,  project == WebError::Storage{"storage operation failed"}
```

(Repeat one representative test per enum, asserting the same
`(kind → WebError variant)` the deleted mapper produced.)

- [ ] **Step 2: Run, verify FAIL.**
      `devtool run -- cargo nextest run -p storage from_audience_error` → FAIL.

- [ ] **Step 3: Implement.** Add `host = { path = "../host" }` to
      `storage/Cargo.toml`. Add each
      `impl From<…> for host::error::InternalError` beside its source enum, body
      = the deleted web mapper's arms — so the wire class is **preserved by
      construction** (no class-flip audit needed here, unlike Task 4). Delete
      the web mapper fns; convert their call sites to `?` (or `.map_err(...)?`
      where a status side-effect remains — see `not_found_error` in Task 8). Add
      `From<PostFormat parse>` and `From<TaggingError>`. **Committable
      per-vertical** if the diff is large.

- [ ] **Step 4: Run, verify PASS.**
      `devtool run -- cargo nextest run -p storage from_` → PASS;
      `devtool run -- cargo nextest run -p web --features server` → PASS (wire
      unchanged).
      `rg -n 'fn map_audience_error|fn perform_update_error|fn perform_creation_error|fn register_open_error|fn register_invite_error|fn login_error' web/src`
      → nothing. `devtool run -- cargo xtask check --no-test`.

- [ ] **Step 5: Commit.**

```bash
git add storage/ web/src Cargo.lock
git commit -m "refactor(#334): collapse per-vertical error mappers into From impls in storage"
```

---

### Task 6: Push down `viewer.rs`

**Files:**

- Modify: `web/src/viewer.rs`; create/extend `common/src/visibility.rs` (or the
  module owning `ViewerIdentity`) for `account_viewer`/`viewer_user_id`; add
  `local_channel_id` to `storage` (the module owning `SubscriptionStorage`).
- Test: `common` unit tests for the pure projections; a dual-backend storage
  test for `local_channel_id`.

**Interfaces:**

- Produces:
  `common::visibility::account_viewer(user_id: Option<i64>, local_channel: Option<i64>) -> ViewerIdentity`;
  `common::visibility::viewer_user_id(&ViewerIdentity) -> Option<i64>`;
  `storage::…::local_channel_id(subs: &dyn SubscriptionStorage) -> Option<i64>`
  — **fail-closed** (swallows the storage error via `.ok()`, preserving today's
  anonymous fallback, `viewer.rs:101`), memoized via the relocated
  `LOCAL_CHANNEL_ID: OnceLock<i64>`.
- Retained in `web`: `viewer_identity()` — body reduces to the two leptos calls
  (`leptos_axum::extract::<AuthUser>()`,
  `expect_context::<Arc<dyn SubscriptionStorage>>()`) wrapping the pushed-down
  helpers.

- [ ] **Step 1: Write tests.** `common` unit tests for
      `account_viewer`/`viewer_user_id` (pure — table of inputs→ViewerIdentity).
      Dual-backend storage test: `local_channel_id` returns the seeded local
      channel id (functional, both backends). **Do not assert memoization in the
      dual-backend test** — `LOCAL_CHANNEL_ID` is a _process-global_ `OnceLock`,
      so a two-backends-one-process `#[storage_test]` would leak the first
      backend's value into the second; memoization is not backend-specific. Pin
      memoization, if at all, in a separate single-backend unit (or accept it as
      an untested process-global with a justified `// cov:ignore`).
- [ ] **Step 2: Run, verify FAIL.**
      `devtool run -- cargo nextest run -p common viewer` and
      `-p storage local_channel_id` → FAIL.
- [ ] **Step 3: Implement the moves** (bodies unchanged, relocated;
      `viewer_identity` calls them).
- [ ] **Step 4: Run, verify PASS.** the two commands above → PASS;
      `-p web --features server viewer` → PASS.
      `devtool run -- cargo xtask check --no-test`.
- [ ] **Step 5: Commit.**
      `git commit -m "refactor(#334): push viewer identity logic down (common/storage); web keeps the leptos adapter"`

---

### Task 7: Push down `auth/server.rs`

**Files:**

- Modify: `web/src/auth/server.rs`; add pure helpers to `common`
  (`parse_basic_auth`) and `host` (`resolve_credential`, `CookieSettings`,
  `session_cookie_header`/`clear_session_cookie_header`); add
  `session_outcome`/`login_outcome` to `storage`.
- Modify: `host/Cargo.toml` (`http` dep for `resolve_credential`).
- Test: `common`/`host` unit tests for the pure helpers; `storage` unit tests
  for the outcome mappers.

**Interfaces:**

- Produces:
  - `common::auth::parse_basic_auth(header: &str) -> Option<(String, String)>`
    (pure base64/split).
  - `host::auth::resolve_credential(headers: &http::HeaderMap) -> Option<Credential>`
    (COOKIE / Bearer / Basic).
  - `host::auth::{CookieSettings, session_cookie_header(token, secure) -> (HeaderName, HeaderValue)-ish string, clear_session_cookie_header(secure)}`.
  - `storage::…::{session_outcome(&SessionAuthError) -> SessionOutcome, login_outcome(&UserAuthError) -> LoginOutcome}`.
- **STAYS in `web`** (record, don't move): `AuthUser` + `impl FromRequestParts`
  (orphan rule), `AuthRejection` + `IntoResponse`,
  `require_auth`/`require_auth_with_parts`, `auth_rejection_error`.
  `verify_basic_username` splits — pure `Username` comparison → `common`, the
  `AuthRejection`-typed wrapper stays.
  `set_session_cookie`/`clear_session_cookie` split — the header-string builder
  → `host`, the `use_context::<CookieSettings/ResponseOptions>` +
  `insert_header(SET_COOKIE)` adapter stays.

- [ ] **Step 1: Write tests.** `common`: `parse_basic_auth` (valid `user:pass`,
      missing colon, bad base64 → None). `host`: `resolve_credential` over a
      built `HeaderMap` for each scheme; `session_cookie_header`/`clear` produce
      the exact current cookie strings (byte-for-byte — pin against the current
      literals). `storage`: `session_outcome`/`login_outcome` map each variant.
- [ ] **Step 2: Run, verify FAIL.** `-p common auth`, `-p host auth`,
      `-p storage outcome` → FAIL.
- [ ] **Step 3: Implement.** Add `http.workspace = true` to `host/Cargo.toml`
      (`axum::http` is a re-export of `http`, so the `HeaderMap` type is
      compatible). Move the pure cores; leave the leptos/axum adapters in `web`
      calling them (e.g. `set_session_cookie` = build
      `session_cookie_header(token, settings.secure)` then
      `opts.insert_header(SET_COOKIE, …)`). **`Credential`'s fields (`token`,
      `expected_username`) become `pub`** — `web`'s retained
      `AuthUser::from_request_parts` (`auth/server.rs:62-72`) reads them across
      the crate boundary once `Credential` lives in `host`. **Confirm**
      `common::metrics::{SessionOutcome, LoginOutcome}` are in the ungated
      `pub mod metrics` (they are) so `storage`'s
      `session_outcome`/`login_outcome` name them without enabling
      `common/metrics`.
- [ ] **Step 4: Run, verify PASS** (the three commands +
      `-p web --features server auth`).
      `devtool run -- cargo xtask check --no-test`.
- [ ] **Step 5: Commit.**
      `git commit -m "refactor(#334): push auth credential/cookie/outcome logic down; web keeps extractor+cookie adapters"`

---

### Task 8: Push down `posts/server.rs`

**Files:**

- Modify: `web/src/posts/server.rs`; add cursor + effectful helpers to `storage`
  (module owning `PostStorage`/`PostCursor`/`PostRecord`).
- Test: dual-backend storage tests for the effectful helpers.

**Interfaces:**

- Produces (in `storage`): `to_post_cursor(&PostRecord) -> PostCursor`;
  `parse_post_cursor(&str) -> InternalResult<PostCursor>`;
  `apply_post_tag_diff(posts: &dyn PostStorage, post_id, diff) -> InternalResult<()>`;
  `fetch_post_record(posts: &dyn PostStorage, permalink, viewer, now) -> InternalResult<PostRecord>`;
  `find_draft_by_permalink_for_user(posts: &dyn PostStorage, …) -> InternalResult<Option<PostRecord>>`;
  `list_by_tag_rows(result: Result<Vec<PostRecord>, ListByTagError>) -> InternalResult<Vec<PostRecord>>`
  (encodes `TagNotFound→empty`).
- **STAYS in `web`:** `PostResponse`/`TimelinePostSummary` (wire types) and
  their builders `post_response`/`timeline_post_summary` (Ok-side projection);
  `set_not_found_status` (`ResponseOptions` adapter).
  `not_found_error`/`private_post_not_found_error` split —
  `set_not_found_status()` side-effect stays; the
  `InternalError::not_found(...)`/`masked(...)` construction moves with the
  carrier (now `host`).

- [ ] **Step 1: Write dual-backend storage tests** for each effectful helper
      (seed a post, assert `fetch_post_record` returns it / errors NotFound with
      a `set_not_found_status`-free carrier; `list_by_tag_rows(TagNotFound)` →
      `Ok(vec![])`; `apply_post_tag_diff` adds/removes tags;
      `find_draft_by_permalink_for_user` paginates). `parse_post_cursor`
      round-trips `to_post_cursor`.
- [ ] **Step 2: Run, verify FAIL.**
      `-p storage post_cursor fetch_post_record apply_post_tag_diff` → FAIL.
- [ ] **Step 3: Implement the moves.** Effectful helpers → `storage` (returning
      `host::InternalError`); `web`'s `#[server]` bodies call them then project
      the record via the retained `post_response`/`timeline_post_summary` and,
      on the 404 path, call `set_not_found_status()` around the moved
      constructor.
- [ ] **Step 4: Run, verify PASS** (`-p storage …` +
      `-p web --features server posts`).
      `devtool run -- cargo xtask check --no-test`.
- [ ] **Step 5: Commit.**
      `git commit -m "refactor(#334): push posts cursor/effectful helpers to storage; web keeps wire DTOs + status adapter"`

---

### Task 9: Acceptance sweep + full `validate`

**Files:** none (verification), plus any tiny fixups the audit surfaces (each
its own micro-commit if needed).

**Interfaces:** none.

- [ ] **Step 1: Audit the acceptance criteria** against the tree:
  - A2/A20: `rg -n 'WebError' host/src` → nothing;
    `rg -n '\.public\(\)' web/src` → nothing (field+accessor gone; the 3 sites
    use `kind()`).
  - A3/A4: `host/Cargo.toml` has no `storage`/`web`/`leptos`/`axum`/`chrono`;
    `rg -n 'use storage' host/src` → nothing.
  - A6: `rg -n 'kind_class_for|From<WebError>' web/src` → nothing.
  - A9: `owner_lifetime` tests present and green in `web`.
  - A16/A12: `rg -n '#\[cfg\(feature = "server"\)\]' web/src` reviewed against
    the D5 inventory — every remaining item is on the "STAYS" list (wire types,
    `#[server]` fns, extractor/`Parts`, `ResponseOptions`/cookie adapters,
    `AuthRejection`, projections).
  - A19: no lossy `validation(e.to_string())` remains (`rg`).
- [ ] **Step 2: Full gate.** `cargo xtask validate` (Bash background mode —
      long/cold; static + clippy incl. wasm-target + coverage + full
      `{sqlite,postgres}×{chromium,firefox}` e2e). Expected: green (A13/A17).
- [ ] **Step 3: Read the coverage sidecar** for the moved logic
      (`ctx_execute(shell, "jq '.steps' .xtask/last-result.json")`): the
      host-side/storage tests cover the relocated helpers (A12 — win realized).
      Add tests or justified `// cov:ignore` for any gap.
- [ ] **Step 4: Commit** any fixups.
      `git commit -m "test(#334): close coverage on relocated error/push-down logic"`
      (only if fixups were needed).

---

## Self-review

- **Spec coverage:** D1→T2/T3; D2→T2 (`project`); D3→T2/T3 (boundary split,
  `emit_boundary_failure`); D4→T5; D5 (auth/viewer/posts)→T6/T7/T8; D5 error
  rule→T4; D6 floor invariant→T3 (verified), T9 (audited); D7 name→T1 (ADR); the
  ADR + principle→T1. A1–A20 each map to a task's tests/audit (carrier
  A1–A4→T2/T3; wire A5–A7→T2; boundary A8–A10→T2/T3; mapper A11/A15→T5;
  push-down A12–A14→T6–T8; completeness A18/A19/A20→T4/T5/T2; cross-cutting
  A16→T9, A17→T9).
- **No placeholders:** every implement step names its tests, target signatures,
  and exact `devtool run -- cargo …` commands with FAIL/PASS.
- **Type consistency:**
  `InternalError`/`ErrorKind`/`ErrorClass`/`InternalResult` are defined in T2,
  relocated in T3, re-exported from `web::error`, named identically in
  T4/T5/T6/T7/T8; `project`/`emit_boundary_failure`/`public_message` signatures
  are used consistently downstream.
