# Plan: issue #239 — the server owns its SPA shell (embed `csr/index.html`)

Spec:
[`docs/superpowers/specs/2026-07-04-issue-239-embed-spa-shell.md`](../specs/2026-07-04-issue-239-embed-spa-shell.md)
Issue: [#239](https://github.com/jaunder-org/jaunder/issues/239)

## Review header

**Goal.** The server serves its CSR SPA shell from a compile-time constant
(`include_str!(csr/index.html)`) instead of reading `{site_root}/index.html`
from disk — so the host `cargo leptos end-to-end` loop stops serving a broken
shell on SPA-fallback routes, unblocking #153. `csr/index.html` becomes the
single source (embedded; no disk copy anywhere).

**Scope.**

- _In_: `web/src/render/mod.rs` (the `SPA_SHELL` const); `server/src/lib.rs`
  (serve it, drop the disk read); `flake.nix` (drop the `cp`, fix the comment);
  `xtask/src/audit_wasm.rs` (audit the embedded shell, not `{site}/index.html`).
- _Out_: CSR-mode realignment (#236); embedding the wasm bundle `pkg/*` (#237).
  Do not touch cargo-leptos config or the `pkg/*` `ServeDir`.

**Tasks.**

1. `SPA_SHELL` const + server serves it from the embed (+ a server-level test).
2. Single source: drop the flake `cp`; point `audit-wasm` at the embedded shell.
3. Verify: Nix e2e combo (SPA routes served from the embed, no disk
   `index.html`) + `audit-wasm`.

**Key risks / decisions.**

- Tasks 1 & 2 land together before merge: after Task 1 the server no longer
  needs a disk `index.html`; Task 2 removes it from the Nix site. In between,
  Nix still has the disk copy (harmless). Nix e2e is verified after Task 2 (Task
  3).
- **`SPA_SHELL` forces a crane-filter change (mandatory, in Task 1).** It is a
  _production_ `include_str!(csr/index.html)` compiled into `jaunderBin` (`web`
  built with the `server` feature → `render` compiled). The shared `src` filter
  (flake.nix:288-293) **excludes `.html`**, so without a filter change the Nix
  server build fails `couldn't read …/csr/index.html`. The existing
  `#[cfg(test)]` `include_str!` does **not** prove otherwise: it only compiles
  in the `coverage` check (a _different, permissive_ src filter that includes
  `.html`); `clippy` never compiles `render` (web default features `[]`;
  `render` is `#[cfg(feature="server")]`). **`cargo xtask check` cannot catch
  this** (clippy + coverage, neither builds `jaunderBin`) — so Task 1 adds the
  filter rule _and_ verifies with an explicit `nix build .#jaunder`.
- `ServeFile → Html` changes response headers (no
  `ETag`/`Last-Modified`/ranges), not the body; SPA routes stay **200**. Nix e2e
  (Task 3) confirms nothing depends on shell caching headers.

**For agentic workers.** Drive with `jaunder-iterate`; delegate a task via
`jaunder-dispatch` when useful. Tick checkboxes in real time.

## Global constraints

- Worktree: `.claude/worktrees/issue-239-host-index-html-seam` (already
  created).
- Gate before every commit: `cargo xtask check` clean (pre-commit hook runs it);
  commit per `jaunder-commit`. **No `Co-Authored-By` trailer.**
- No storage changes → no backend-parity/dialect concerns.
- Crate names: web = `web`, server = `jaunder`, xtask = `xtask` (separate
  workspace — test via `cargo nextest run --manifest-path xtask/Cargo.toml`).
- Leave `docs/archive/**` untouched.

---

## Task 1 — `SPA_SHELL` const + server serves the embedded shell

### Files

- `flake.nix:288-293` — add `csr/index.html` to the crane src filter (required
  so the production `SPA_SHELL` include finds it in the Nix server build).
- `web/src/render/mod.rs` — add the `pub const` (beside `PREPAINT_SCRIPT`).
- `server/src/lib.rs:108-121` — serve the const; drop the disk read +
  `ServeFile`. Add a server-level test in the existing `#[cfg(test)] mod tests`
  (beside `home_route_returns_ok`, line 175).

### Step 1.0 — crane src filter (do first)

In `flake.nix`'s shared src filter (the `|| (…)` chain at 288-293, beside `.sql`
/ `.css`), add:

```nix
|| (pkgs.lib.hasSuffix "csr/index.html" path)
```

Specific (not a broad `.html` suffix) to avoid pulling stray HTML (e.g.
playwright reports) into `src`. Without this, `nix build .#jaunder` fails to
read `csr/index.html` once `SPA_SHELL` is a non-test include.

### Step 1.1 — the constant

In `web/src/render/mod.rs`, near `PREPAINT_SCRIPT`:

```rust
/// The CSR SPA shell, embedded at compile time. The host `cargo leptos` build
/// never writes `index.html` to `site_root` (#239); the server owns it and serves
/// it — the same way the projector renders its routes from constants. Single
/// source of the shell; copied to no build output.
pub const SPA_SHELL: &str = include_str!("../../../csr/index.html");
```

### Step 1.2 — the server test (RED first)

Add to `server/src/lib.rs` tests (mirrors `home_route_returns_ok`'s harness):

```rust
#[tokio::test]
async fn spa_fallback_serves_embedded_shell_without_disk_index_html() {
    use axum::http::header::CONTENT_TYPE;
    // A site_root with no index.html on disk (the host reality, #239). The SPA
    // fallback must still serve the embedded shell — 200, text/html, boots the wasm.
    let options = LeptosOptions::builder()
        .output_name("test")
        .site_root("/tmp/jaunder-nonexistent-site-239")
        .build();
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            ensure_server_fns_registered();
            let app = create_router(
                options,
                test_state().await,
                test_mailer(),
                true,
                test_storage_path(),
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
```

- Run (RED):
  `cargo nextest run -p jaunder spa_fallback_serves_embedded_shell_without_disk_index_html`
  → **FAIL** (today `ServeFile` on the missing file → 404).

### Step 1.3 — serve the embedded shell (GREEN)

Replace the `let app = { … }` block at `server/src/lib.rs:108-121`:

```rust
let app = {
    use axum::handler::HandlerWithoutStateExt;
    use axum::response::Html;
    use tower_http::services::ServeDir;
    let site_root = leptos_options.site_root.to_string();
    // The CSR SPA shell is embedded (#239): the server owns it, like the projector
    // renders its routes from constants. The host `cargo leptos` build never writes
    // index.html to site_root. `pkg/*` + public assets still serve from disk.
    let app = crate::projector::register(
        app,
        crate::projector::Shell(web::render::SPA_SHELL.into()),
    );
    async fn spa_shell() -> Html<&'static str> {
        Html(web::render::SPA_SHELL)
    }
    // ServeDir serves `pkg/*` + public assets from disk; any non-file route falls
    // back to the embedded shell (200) so the CSR client boots and routes client-side.
    app.fallback_service(ServeDir::new(&site_root).fallback(spa_shell.into_service()))
};
```

Removed vs. today: `std::fs::read_to_string`, the `index_html` binding, and the
`ServeFile` import.

- Run (GREEN): same command → **PASS**.

### Verify & commit

- `cargo xtask check` → PASS (host + coverage).
- **`devtool run -- nix build .#jaunder -L`** → PASS. This is the load-bearing
  check: it builds the server binary against the _restrictive_ shared `src`, so
  it compiles the production `SPA_SHELL` include and fails if Step 1.0's filter
  rule is missing. `cargo xtask check` alone does **not** build `.#jaunder`, so
  do not skip this.
- Commit:
  `fix(server): serve the CSR SPA shell from an embedded constant (#239)`.

---

## Task 2 — single source: drop the flake copy, audit the embedded shell

### Files

- `flake.nix` — the `site` derivation (~455-463): drop the `cp`, fix the
  comment.
- `xtask/src/audit_wasm.rs` — read the embedded shell instead of
  `{site}/index.html`; update the missing-artifact test.

### Step 2.1 — drop the flake copy

`flake.nix` `site` derivation — remove `cp ${./csr/index.html} $out/index.html`
and correct the comment (currently "the projector serves this same `index.html`
as its SPA fallback" — now false):

```nix
# The site the server serves: the CSR client's wasm bundle (`pkg/*`) + public
# assets. The SPA shell (`csr/index.html`) is NOT staged here — the server embeds
# it (#239) and serves it from a compile-time constant on host and Nix alike.
site = pkgs.runCommand "jaunder-site" { } ''
  mkdir -p $out/pkg
  cp -r ${csrWasmBundle}/. $out/pkg/
  cp -r ${./public}/. $out/
'';
```

### Step 2.2 — audit the embedded shell

`xtask/src/audit_wasm.rs`: the shell is no longer in the built site, so read it
from source (the same `csr/index.html` the server embeds), cwd-independently via
`include_str!`. Add near the top:

```rust
/// The CSR SPA shell the server embeds and serves (#239). Audited from source —
/// it is no longer copied into the built site — against the emitted bundle. xtask
/// is rebuilt from the live tree every run, so this stays current.
// Path depth: `audit_wasm.rs` is at `xtask/src/`, so `../../` reaches the repo root
// (NOT `../../../` — that's the web crate's depth, `web/src/render/`).
const SPA_SHELL: &str = include_str!("../../csr/index.html");
```

In `run()`, replace the `{site}/index.html` read with `SPA_SHELL`:

```rust
pub fn run(site_path: Option<&str>) -> Result<AuditReport> {
    let site_path = resolve_site_path(site_path)?;
    let names = shell_boot_artifacts(SPA_SHELL)?;
    // … unchanged: for each name, read `{site_path}/{name}`, measure, Err if absent …
}
```

Remove the `index_path` / `read_to_string({site}/index.html)` lines.

Update the test `run_errors_when_a_referenced_artifact_is_missing`: `run()` now
takes the shell from `SPA_SHELL`, so the temp `index.html` write is no longer
needed — the temp dir just needs to lack `pkg/jaunder.wasm`:

```rust
#[test]
fn run_errors_when_a_referenced_artifact_is_missing() {
    // The embedded shell boots `/pkg/jaunder.wasm`; a site without that file → Err
    // naming the missing artifact, without invoking `nix`.
    let dir = std::env::temp_dir().join(format!("audit-wasm-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let res = run(Some(dir.to_str().unwrap()));
    std::fs::remove_dir_all(&dir).ok();
    let err = res.unwrap_err().to_string();
    assert!(err.contains("jaunder.wasm"), "error names the missing artifact: {err}");
}
```

(`shell_boot_artifacts_*` unit tests are unaffected — they test the pure
function.)

### Verify & commit

- `cargo nextest run --manifest-path xtask/Cargo.toml` → PASS.
- `cargo xtask audit-wasm` → **PASS**: builds `.#site` (now with **no**
  `index.html`), reads the embedded shell, confirms `pkg/jaunder.wasm` +
  `pkg/jaunder.js` exist. (Also confirms Step 2.1: the site builds without the
  `cp`.)
- `cargo xtask check` → PASS. Commit:
  `build(nix): drop the site index.html copy; audit the embedded shell (#239)`.

---

## Task 3 — verify the embedded shell serves the whole suite

No code. After Tasks 1-2 the Nix site has **no `index.html`**, so every route —
the projector's content routes _and_ the SPA-fallback routes — is served from
the embedded shell. A green e2e is the hermetic proof #239 is fixed.

- **AC1/AC7 (browser):** `cargo xtask e2e sqlite chromium` → green. This
  exercises SPA-fallback routes (`/login`, `/register`, `/app`, …) against a
  site with no disk `index.html`, so passing proves the embedded shell serves
  them and hydration completes. Confirms AC4's "no e2e depends on shell caching
  headers" implicitly.
- **AC6:** `cargo xtask audit-wasm` green (from Task 2).
- **AC3 (projector `Shell` is the embedded shell):** covered by the diff-visible
  one-line wiring `crate::projector::Shell(web::render::SPA_SHELL.into())` in
  `create_router` (no disk read remains) plus the e2e's projector-route coverage
  (`/`, `/~user`, `/tags/*`), which render + hydrate only if the shell is
  correct. No separate unit assertion (reaching the projector's `shell_response`
  fallback reliably needs seeded fixtures; not worth the fragility for a
  structural change).
- Full `cargo xtask validate` runs at ship (`jaunder-ship` step 3).

Record outcomes in the PR. If any SPA-fallback route fails in the browser,
investigate the fallback wiring (`into_service`) before proceeding.

## Self-review

- Small, cohesive commits (server embed; flake+audit single-source; verify).
- The #234 shell↔bundle guard is preserved and made more correct (audits the
  embedded shell, not a disk copy).
- No cargo-leptos config or `pkg/*` serving changes leak in from #236/#237.
- Single source: after this, `csr/index.html` is `include_str!`'d (server +
  audit) and copied nowhere; `rg 'cp .*csr/index.html' flake.nix` returns
  nothing.
- Standalone of #153; the Nix e2e (no disk `index.html`) is the hermetic proof.
