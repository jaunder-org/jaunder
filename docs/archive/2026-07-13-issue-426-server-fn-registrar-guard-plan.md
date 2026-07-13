# Server-fn Test-Registrar Guard — Implementation Plan (#426)

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Stop the hand-maintained `#[server]` test registrar from rotting —
consolidate the two lists into one and add an `xtask` gate that fails when any
`web` `#[server]` fn is missing from it.

**Architecture:** (a) Delete the second registrar in `server/src/lib.rs`, moving
its router tests into the `web` integration binary where the complete registrar
already lives; (b) register the 10 `web` `#[server]` fns currently missing; (c)
a new `syn`-based `xtask` check (sibling of `test_pattern_check`) enumerates
`web` `#[server]` fns and fails on any absent from
`server/tests/helpers/mod.rs`.

**Tech Stack:** Rust, `xtask` (`cargo xtask check`/`validate`), `syn` v2 +
`proc-macro2` (span-locations), `server_fn::axum::register_explicit`, `nextest`.

**Spec:**
`docs/superpowers/specs/2026-07-13-issue-426-server-fn-registrar-guard.md` — the
"what/why". This plan is the "how"; read the spec's Design §1–§4 and Acceptance
criteria alongside it. **ADR draft:**
`docs/adr/drafts/server-fn-test-registrar-guard.md` (already written; numbered
at ship by `cargo xtask adr promote`).

## Global Constraints

- **Mandatory, no opt-out:** every `web` `#[server]` fn must appear in the
  single registrar; the gate has no exemption marker.
- **One registrar:** after this work, `register_explicit` appears only under
  `server/tests/`; `server/src/lib.rs` has no registrar.
- **Match by leaf type name**, never module path (re-exports make paths differ).
- **`#[server(endpoint = "…")]`-only** naming holds in this repo → generated
  type = `PascalCase(fn ident)`; the gate treats a positional-rename form as a
  hard error.
- **No `Co-Authored-By` trailer.** Each task = one clean commit; run
  `cargo xtask check` first so the pre-commit gate passes clean
  (**jaunder-commit**).
- **#358 coordination:** #358 concurrently edits
  `helpers::ensure_server_fns_registered()` (folding the `media` trio). Expect
  to rebase onto #358 before ship and reconcile (don't double-add `media`); the
  gate is the backstop whichever PR adds an entry.
- **`test-backend-pattern` guard applies to relocated tests:** `server/tests` is
  a `TEST_ROOT`, so any `#[tokio::test]` there must wear a backend template or a
  `// guard:*` marker. The relocated router tests use `#[apply(backends)]` +
  `#[case] backend: Backend` (house style), so Task 1's commit stays gate-green.

---

## Review header (for the approver)

**Scope — in:** consolidate the two registrars into one; register the 10 missing
`web` `#[server]` fns; add + wire the `server-fn-registrar` `xtask` gate; ADR
(drafted). **Out:** Approach B (linkme/macro auto-registration); any behavioral
change to server fns; the production server's own registration path.

**Tasks:**

1. Consolidate — relocate `lib.rs`'s 4 router tests into `server/tests/web/`,
   delete `lib.rs`'s registrar.
2. Reconcile drift — register the 10 missing `web` `#[server]` fns in
   `helpers::ensure_server_fns_registered()`.
3. The gate — new `syn`-based `server-fn-registrar` `xtask` check, unit-tested
   and wired into `check` + `validate`.

**Key risks/decisions:** ordering keeps the pre-commit gate green throughout —
the tree is fully reconciled (Tasks 1–2) _before_ the new server-fn gate is
wired (Task 3), so no commit is ever wired-but-red. Relocating tests into
`server/tests` also brings them under the **existing** `test-backend-pattern`
guard, so they are converted to the `#[apply(backends)]` fixture idiom (not bare
`#[tokio::test]`) — otherwise Task 1's commit would go red. Leaf-name collision
is audited in Task 3 (assert none today). The gate module can't be committed
unwired (dead-code), so Task 3 creates + wires it atomically; its TDD red→green
is at the unit-fixture level.

---

## Task 1: Consolidate — relocate router tests, delete the second registrar

**Files:**

- Create: `server/tests/web/router.rs`
- Modify: `server/tests/web/main.rs:9-21` (add `mod router;`)
- Modify: `server/src/lib.rs:143-308` (delete the entire
  `#[cfg(test)] mod tests`)

**Interfaces:**

- Consumes (from `server/tests/helpers/mod.rs`, already present):
  `ensure_server_fns_registered()`, `test_options() -> LeptosOptions`,
  `noop_mailer() -> Arc<dyn common::mailer::MailSender>`,
  `tmp_storage_path() -> PathBuf`, the `backends` rstest template, `Backend`,
  and `TestEnv { state: Arc<storage::AppState>, base: TempDir }` (state from
  `backend.setup().await`).
- Consumes (public API):
  `jaunder::create_router(LeptosOptions, Arc<storage::AppState>, Arc<dyn MailSender>, bool, PathBuf) -> Router`.
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Write the relocated tests** in `server/tests/web/router.rs`.
      These are the four tests currently in `server/src/lib.rs`
      (`home_route_returns_ok`,
      `spa_fallback_serves_embedded_shell_without_disk_index_html`,
      `home_response_contains_app_content`,
      `current_user_api_route_returns_ok`), rewritten to use the `helpers` shims
      instead of the `lib.rs`-private ones. Full file:

```rust
use axum::{
    body::Body,
    http::{header::CONTENT_TYPE, Request, StatusCode},
};
use leptos::prelude::LeptosOptions;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    backends, ensure_server_fns_registered, noop_mailer, test_options, tmp_storage_path, Backend,
    TestEnv,
};

#[apply(backends)]
#[tokio::test]
async fn home_route_returns_ok(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            ensure_server_fns_registered();
            let app = jaunder::create_router(
                test_options(),
                state,
                noop_mailer(),
                true,
                tmp_storage_path(),
            );
            let response = app
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        })
        .await;
}

#[apply(backends)]
#[tokio::test]
async fn spa_fallback_serves_embedded_shell_without_disk_index_html(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    // A site_root with no index.html on disk (the host reality, #239). The SPA
    // fallback must still serve the embedded shell — 200, text/html, boots wasm.
    let options = LeptosOptions::builder()
        .output_name("test")
        .site_root("/tmp/jaunder-nonexistent-site-239")
        .build();
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            ensure_server_fns_registered();
            let app = jaunder::create_router(
                options,
                state,
                noop_mailer(),
                true,
                tmp_storage_path(),
            );
            // `/login` is a client route → not a projector route → SPA fallback.
            let response = app
                .oneshot(Request::builder().uri("/login").body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(CONTENT_TYPE).unwrap(),
                "text/html; charset=utf-8"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert!(
                body.contains(r#"init("/pkg/jaunder.wasm")"#),
                "SPA fallback serves the embedded shell that boots the wasm: {body}"
            );
        })
        .await;
}

#[apply(backends)]
#[tokio::test]
async fn home_response_contains_app_content(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let app = jaunder::create_router(
                test_options(),
                state,
                noop_mailer(),
                true,
                tmp_storage_path(),
            );
            let response = app
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let html = String::from_utf8(body.to_vec()).unwrap();
            assert!(html.contains("Jaunder"));
        })
        .await;
}

#[apply(backends)]
#[tokio::test]
async fn current_user_api_route_returns_ok(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            ensure_server_fns_registered();
            let app = jaunder::create_router(
                test_options(),
                state,
                noop_mailer(),
                true,
                tmp_storage_path(),
            );
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/current_user")
                        .header("content-type", "application/x-www-form-urlencoded")
                        .header(
                            "traceparent",
                            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                        )
                        .body(Body::empty())
                        .expect("failed to build request"),
                )
                .await
                .expect("failed to get response");
            assert_eq!(response.status(), StatusCode::OK);
        })
        .await;
}
```

Notes on the conversion (why not a straight copy of the `lib.rs` originals):

- **`AppState` is not re-exported from `jaunder`.** `server/src/lib.rs` does
  `use ::storage::AppState;` (private), so `jaunder::AppState` does not resolve.
  The state comes from the fixture — `TestEnv { state, base: _base }` — typed
  `Arc<storage::AppState>`; never write `jaunder::AppState` or a local
  `test_state()` over `sqlite::memory:`.
- **The `test-backend-pattern` guard now applies.** `server/tests` is a
  `TEST_ROOT` (`xtask/src/steps/test_pattern_check.rs:34`), so a bare
  `#[tokio::test]` here would fail the gate. Every sibling web router test wears
  `#[apply(backends)]` + `#[case] backend: Backend` and builds state from
  `backend.setup().await` (e.g. `web_auth.rs:158-159`) — these tests mirror that
  exactly. This is why they parameterize over both backends rather than opening
  a fixed in-memory SQLite (a `// guard:no-backend` marker would be dishonest —
  they do touch a DB — and `sqlite_only` would trip the ADR-0053 homing rule in
  this non-dialect dir). `base: _base` keeps the `TempDir` alive for the test
  body (dropping it unlinks the SQLite file; ADR-0053 / #136).

- [ ] **Step 2: Wire the module** — add `mod router;` to
      `server/tests/web/main.rs` (alphabetical, before `mod web_account;`).

- [ ] **Step 3: Delete the second registrar** — remove the entire
      `#[cfg(test)] mod tests { … }` block at `server/src/lib.rs:143-308` (it
      contains only these four tests + their private shims + the local
      `ensure_server_fns_registered`). Leave the rest of `lib.rs` untouched.

- [ ] **Step 4: Run the relocated tests, verify they pass**

Run: `cargo nextest run -p jaunder --test web router` Expected: PASS — each of
the four `router::*` tests green on **both** backends (the `#[apply(backends)]`
sqlite + postgres cases): 200 on `/`, embedded shell on `/login`, `Jaunder` in
home HTML, 200 on `POST /api/current_user`. (Postgres cases need the local PG
harness, as every `server/tests/web` test does; run under `cargo xtask check` if
not already provisioned.)

- [ ] **Step 5: Verify the second registrar is gone**

Run: `rg -n 'register_explicit|ensure_server_fns_registered' server/src`
Expected: no matches under `server/src`.

- [ ] **Step 6: Commit**

```bash
git add server/tests/web/router.rs server/tests/web/main.rs server/src/lib.rs
git commit -m "test(server): relocate router tests to integration, drop the second registrar"
```

Run `cargo xtask check` first so the pre-commit gate passes clean
(**jaunder-commit**).

---

## Task 2: Reconcile drift — register the 10 missing `web` `#[server]` fns

**Files:**

- Modify: `server/tests/helpers/mod.rs:34-79` (inside
  `ONCE.get_or_init(|| { … })`)

**Interfaces:**

- Consumes: the `web::<mod>::<Type>` server-fn types (all confirmed present and
  `pub` in `web/src/lib.rs`).
- Produces: a complete registrar list Task 3's gate will verify.

- [ ] **Step 1: Add the missing registrations** to
      `ensure_server_fns_registered()`, grouped next to their module siblings.
      The 10 entries (exact paths — the re-export path, not the source-file
      path):

```rust
// media (whole module was unregistered)
server_fn::axum::register_explicit::<web::media::ListMyMedia>();
server_fn::axum::register_explicit::<web::media::MediaUsage>();
server_fn::axum::register_explicit::<web::media::DeleteMedia>();
// posts
server_fn::axum::register_explicit::<web::posts::DefaultAudienceSelection>();
server_fn::axum::register_explicit::<web::posts::PostAudienceSelection>();
server_fn::axum::register_explicit::<web::posts::DeletePost>();
server_fn::axum::register_explicit::<web::posts::UnpublishPost>();
// profile
server_fn::axum::register_explicit::<web::profile::GetDefaultPostFormat>();
server_fn::axum::register_explicit::<web::profile::SetDefaultPostFormat>();
// sessions
server_fn::axum::register_explicit::<web::sessions::CreateAppPassword>();
```

If rebasing onto #358 first, `media`'s three may already be present — do not
duplicate; add only what is missing.

- [ ] **Step 2: Verify the type paths resolve (compile)** —
      `register_explicit::<T>()` forces `T` to exist, so a wrong path fails to
      compile.

Run: `cargo check -p jaunder --tests` Expected: PASS (compiles) — proves all 10
paths resolve.

- [ ] **Step 3: Independently confirm the list is now complete** (a preview of
      Task 3's gate logic, so Task 3 doesn't surprise us). Enumerate `web`
      `#[server]` fns' PascalCase names and diff against the registrar leaf
      names; expect an empty "missing" set.

Run: `cargo nextest run -p jaunder --test web` (all `web` integration tests
still green with the enlarged registrar) — and eyeball that the registrar now
covers `media`/`profile`/`sessions`/`posts` additions. Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add server/tests/helpers/mod.rs
git commit -m "test(server): register the 10 web #[server] fns missing from the registrar"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

## Task 3: The gate — `server-fn-registrar` xtask check (create, unit-test, wire)

**Files:**

- Create: `xtask/src/steps/server_fn_registrar_check.rs`
- Modify: `xtask/src/lib.rs:15-23` (add `pub mod server_fn_registrar_check;`)
- Modify: `xtask/src/lib.rs:286-290` and `:313-317` (add a `run` call in the
  `check`/Fix and `validate`/Check arms, next to `test_pattern_check`)

**Interfaces:**

- Consumes: `crate::result::{CommandResult, StepResult}`; `syn` v2
  (`parse_file`, `visit::Visit`, `ItemFn`, `Attribute`), `proc-macro2` span
  locations (already xtask deps; see `xtask/src/coverage/exempt.rs` for the
  visitor idiom, including its `does_not_exempt_server_fn` fixture).
- Produces: `pub fn run(result: &mut CommandResult)` and the pure core
  `fn problems(web_sources: &[(String, String)], registrar_src: &str) -> Option<String>`.

**Design of the pure core (write these signatures exactly):**

```rust
/// A `#[server]` fn found in a `web` source: the generated type name
/// (PascalCase of the fn ident) and the 1-based line of the `#[server]` attr.
struct ServerFn {
    name: String,
    line: usize,
}

/// Every `#[server]` fn in one source file. `Err` on a syn parse failure, or on
/// the unsupported `#[server(SomeName)]` positional-rename form (this repo uses
/// only `#[server(endpoint = "…")]`, so PascalCase(ident) is exact; a positional
/// rename would silently break that mapping, so it is a hard error).
fn server_fns_in(src: &str) -> Result<Vec<ServerFn>, String>;

/// The leaf type names registered via `register_explicit::<web::…::LEAF>()` in
/// the registrar source. Leaf = the last `::`-segment before `>`.
fn registered_names(registrar_src: &str) -> std::collections::BTreeSet<String>;

/// `snake_case` fn ident → `PascalCase` generated type name.
fn pascal_case(ident: &str) -> String;

/// Failure detail naming every `web` `#[server]` fn absent from the registrar
/// (with `path:line`), or any per-file hard error; `None` when the registrar
/// covers every enumerated fn. Pure given its inputs → unit-tested directly.
fn problems(web_sources: &[(String, String)], registrar_src: &str) -> Option<String>;
```

`server_fns_in` mirrors `coverage/exempt.rs`: `syn::parse_file(src)?`, a `Visit`
impl whose `visit_item_fn` checks `f.attrs` for a path == `server`
(`attr.path().is_ident("server")`), reads the fn ident, and — for the hard error
— inspects the attr's `Meta`: a `Meta::List` whose first token is a bare
identifier (not `endpoint`/`input`/`name = …`) is the positional form → `Err`.
Record `attr.span().start().line` for `ServerFn.line`. `run` walks
`web/src/**/*.rs` (recursive, like `test_pattern_check::rust_files`), reads
`server/tests/helpers/mod.rs`, calls `problems`, and pushes one `StepResult`
(`ok`/`fail(detail)`), with a `recovery:` line pointing at the registrar.

- [ ] **Step 1: Write the failing unit tests** in the module's
      `#[cfg(test)] mod tests` (one per branch):

```rust
#[test]
fn extracts_pascalcase_name_and_line() {
    let src = "#[server(endpoint = \"/create_post\")]\npub async fn create_post() {}\n";
    let fns = server_fns_in(src).unwrap();
    assert_eq!(fns.len(), 1);
    assert_eq!(fns[0].name, "CreatePost");
    assert_eq!(fns[0].line, 1);
}

#[test]
fn ignores_non_server_fns() {
    let src = "pub async fn plain() {}\n#[tokio::test]\nasync fn t() {}\n";
    assert!(server_fns_in(src).unwrap().is_empty());
}

#[test]
fn endpoint_and_input_forms_are_accepted() {
    let src = "#[server(endpoint = \"/x\", input = Json)]\npub async fn x() {}\n";
    assert_eq!(server_fns_in(src).unwrap()[0].name, "X");
}

#[test]
fn positional_rename_form_is_a_hard_error() {
    let src = "#[server(MyThing)]\npub async fn my_thing() {}\n";
    assert!(server_fns_in(src).is_err());
}

#[test]
fn syn_parse_failure_is_an_error() {
    assert!(server_fns_in("fn broken( {").is_err());
}

#[test]
fn registered_names_parses_leaf_types() {
    let reg = "\
        server_fn::axum::register_explicit::<web::posts::CreatePost>();\n\
        server_fn::axum::register_explicit::<web::media::ListMyMedia>();\n\
        let x = 1; // unrelated\n";
    let got = registered_names(reg);
    assert!(got.contains("CreatePost"));
    assert!(got.contains("ListMyMedia"));
    assert_eq!(got.len(), 2);
}

#[test]
fn problems_flags_an_unregistered_fn_by_name_and_path() {
    let sources = vec![(
        "web/src/media/mod.rs".to_string(),
        "#[server(endpoint = \"/list_my_media\")]\npub async fn list_my_media() {}\n".to_string(),
    )];
    let registrar = "server_fn::axum::register_explicit::<web::posts::CreatePost>();\n";
    let detail = problems(&sources, registrar).expect("a problem");
    assert!(detail.contains("ListMyMedia"));
    assert!(detail.contains("web/src/media/mod.rs"));
}

#[test]
fn problems_is_none_when_registrar_covers_every_fn() {
    let sources = vec![(
        "web/src/posts/mod.rs".to_string(),
        "#[server(endpoint = \"/create_post\")]\npub async fn create_post() {}\n".to_string(),
    )];
    let registrar = "server_fn::axum::register_explicit::<web::posts::CreatePost>();\n";
    assert_eq!(problems(&sources, registrar), None);
}

#[test]
fn problems_surfaces_a_hard_error() {
    let sources = vec![(
        "web/src/x.rs".to_string(),
        "#[server(MyThing)]\npub async fn my_thing() {}\n".to_string(),
    )];
    let detail = problems(&sources, "").expect("a hard error is reported");
    assert!(detail.to_lowercase().contains("web/src/x.rs"));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml server_fn_registrar`
Expected: FAIL — `server_fns_in` / `registered_names` / `problems` not defined.

- [ ] **Step 3: Implement against the tests** — write `server_fns_in`,
      `registered_names`, `pascal_case`, `problems`, and `run` to the signatures
      above. Every branch is pinned by a Step-1 test (PascalCase extraction,
      line capture, non-server ignore, endpoint/input accept, positional-rename
      hard error, parse-failure error, leaf parsing, missing-fn detail,
      complete→None, hard-error surfacing) — so the contract determines the
      body; the only non-test-pinned parts are the `syn` visitor traversal and
      the recursive file walk, both copied from `coverage/exempt.rs` and
      `test_pattern_check::rust_files`.

- [ ] **Step 4: Run the unit tests, verify they pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml server_fn_registrar`
Expected: PASS.

- [ ] **Step 5: Wire the gate into the runner** — add
      `pub mod server_fn_registrar_check;` under `mod steps` in
      `xtask/src/lib.rs`, and
      `steps::server_fn_registrar_check::run(&mut result);` immediately after
      the `test_pattern_check::run(&mut result);` line in **both** the `check`
      (Fix, ~:289) and `validate` (Check, ~:316) arms.

- [ ] **Step 6: Leaf-name collision audit + gate green on the real tree**

Run: `cargo xtask check` Expected: PASS — the gate runs against the reconciled
tree (Tasks 1–2) and finds nothing missing. If it names a fn, register it in
`helpers` (a stray beyond the 10) and re-run. As part of this step, confirm no
two `web` `#[server]` fns share a PascalCase leaf name (the enumerated set has
no duplicates); if the gate passes with the complete registrar, that condition
already holds.

- [ ] **Step 7: Manually verify the gate catches an omission (AC#1)** — add a
      throwaway
      `#[server(endpoint = "/xtask_probe")] pub async fn xtask_probe()     -> web::WebResult<()> { Ok(()) }`
      to a `web` module, run `cargo xtask check` (or just the check via
      `cargo nextest ... run` on the wired path), confirm it goes **red** naming
      `XtaskProbe` and the file, then **remove** the throwaway. (Do not commit
      the throwaway.)

- [ ] **Step 8: Commit**

```bash
git add xtask/src/steps/server_fn_registrar_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): gate that every web #[server] fn is in the test registrar (#426)"
```

Run `cargo xtask check` first (**jaunder-commit**) — it now includes the new
gate, so a clean run also re-proves AC#3.

---

## Self-Review

**Spec coverage:**

- Spec §1 (one registrar) → Task 1 (delete second list + relocate tests).
- Spec §2 (reconcile drift) → Task 2 (register the 10).
- Spec §3 (the gate) → Task 3 (create + unit-test + wire).
- Spec §4 (ADR) → already drafted at
  `docs/adr/drafts/server-fn-test-registrar-guard.md` (no task; promoted at
  ship).
- AC#1 (gate catches omission) → Task 3 Step 7. AC#2 (one list) → Task 1 Step 5.
  AC#3 (reconciled tree green) → Task 3 Step 6/8. AC#4 (relocated tests assert
  behavior) → Task 1 Step 4. AC#5 (unit tests, incl. positional hard error) →
  Task 3 Steps 1–4. AC#6 (ADR) → drafted.

**Placeholder scan:** no TBD/TODO; every step has real Rust + exact commands.

**Type consistency:** `ensure_server_fns_registered`, `create_router`,
`AppState`, `problems(web_sources, registrar_src)`, `server_fns_in`,
`registered_names`, `pascal_case`, `ServerFn { name, line }`, `run(result)` used
consistently across Tasks 1–3.
