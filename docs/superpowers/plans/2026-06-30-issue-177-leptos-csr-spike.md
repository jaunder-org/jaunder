# Spike #177 — leptos-CSR gate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (inline,
> with checkpoints) to implement this plan task-by-task. This is exploratory
> build/integration infra work (nix derivations, a wasm build, e2e fallout), not unit-TDD
> — verification per task is "it compiles / the nix derivation builds / the e2e boots,"
> not a red→green unit test. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Stand up a feature-gated leptos-CSR build of the web app + a CSR e2e path, then
run a ~30-run `workers:4` postgres+chromium campaign to confirm zero `already been
disposed` panics — a go/no-go gate on leptos-CSR for milestone 8.

**Architecture:** A new `csr/` wasm crate mounts `web::App` client-side
(`mount_to_body`, no hydration). A `csr` cargo feature on the `web` crate compiles the
components + `#[server]` client stubs for CSR. A `csr` feature on the `jaunder` server
crate swaps the reactive `leptos_routes_with_context` page render for a static
`index.html` SPA fallback, **keeping** the `/api/{*fn_name}` server-fn handler (server
fns are not the #173 panic — the page render is). Parallel nix crane derivations build the
CSR wasm/server/site; a CSR `nixosTest` drives it under `workers:4`/`fullyParallel`. The
default SSR build and CI matrix are untouched.

**Tech Stack:** Rust, leptos 0.8.2 (CSR), wasm-bindgen 0.2.121 (via Cargo `^0.2.106`),
crane, nix flakes / `nixosTest`, Playwright, axum + tower-http.

## Global Constraints

- leptos = **0.8.2**; wasm-bindgen-cli = **0.2.121** (matches Cargo's `^0.2.106`
  resolution). **Do NOT change either; do NOT bump leptos to 0.8.20** — it regresses
  rendering (handoff).
- **Every CSR change is feature-gated or a parallel nix attr.** The default SSR build,
  the existing `e2e-{backend}-{browser}` checks, and CI must remain byte-for-byte
  unchanged. `cargo xtask check --no-test` on the default build stays green after every
  task.
- Worktree: `/home/mdorman/src/jaunder/.claude/worktrees/issue-177-leptos-csr-spike`
  (branch `worktree-issue-177-leptos-csr-spike`). Run the gate via the Bash tool (already
  in the worktree) or `cd <worktree> &&` — context-mode runs against the MAIN repo.
- No `Co-Authored-By` trailers (project policy).
- Per-task gate: `cargo xtask check --no-test` (clippy + fmt + compile). Reserve nix
  builds / e2e for the tasks that need them.
- Campaign reproduction recipe (from the #173 handoff): postgres (latency widens the
  Suspense window; sqlite too fast) + chromium; `nix build --rebuild` (nix caches a
  passing e2e); VM `cores=4` + `memorySize=6144` (1 vCPU → false hydration-timeouts;
  2 GB OOMs at 4 browsers).

## Separable concerns

None. All work here is the spike. (The e2e VM sizing bump is reused by #182 but belongs to
the CSR e2e path landed here.)

## File structure

- `csr/Cargo.toml` — **create**. New wasm cdylib crate, mirrors `hydrate/`.
- `csr/src/lib.rs` — **create**. `mount_to_body(web::App)` + a `data-hydrated` readiness
  marker.
- `csr/index.html` — **create**. The static CSR shell (head + boot script).
- `web/Cargo.toml` — **modify**. Add a `csr = ["leptos/csr"]` feature.
- `server/Cargo.toml` — **modify**. Add a `[features] csr = []` section.
- `server/src/lib.rs` — **modify**. cfg-gate the page-render routes vs the CSR static
  fallback in `create_router`.
- `Cargo.toml` (root) — **modify**. Add `"csr"` to `[workspace] members`.
- `flake.nix` — **modify**. Add `csrWasm`, `csrWasmBundle`, `jaunderBinCsr`, `csrSite`
  derivations; expose them as `packages`; add a CSR `nixosTest` + `e2e-csr-postgres-chromium`
  check; CSR playwright config.
- `docs/issue-177-csr-spike-findings.md` — **create**. Campaign run count + GO/NO-GO.
- `docs/adr/0040-web-rendering-leptos-csr.md` — **create** (confirm number at task time).
- `docs/README.md` — **modify**. Add the ADR row.
- Campaign loop: **scratch script in scratchpad, NOT committed.**

---

### Task 1: `web` `csr` feature + the `csr` mount crate (compiles to wasm)

**Files:**
- Modify: `web/Cargo.toml` (features block, lines 63-79)
- Modify: `Cargo.toml` (root `members`, lines 3-9)
- Create: `csr/Cargo.toml`
- Create: `csr/src/lib.rs`

**Interfaces:**
- Consumes: `web::App` (re-exported at `web/src/lib.rs:41`).
- Produces: a `csr` crate building to `csr.wasm` (cdylib) whose `#[wasm_bindgen(start)]`
  entry mounts `App` and sets `document.body[data-hydrated]`.

- [x] **Step 1: Add the `csr` feature to `web/Cargo.toml`.** After the `hydrate` line
  (web/Cargo.toml:65), add:

```toml
csr = ["leptos/csr"]
```

  Resulting `[features]` block:

```toml
[features]
default = []
hydrate = ["leptos/hydrate"]
csr = ["leptos/csr"]
ssr = [
    "dep:anyhow",
    "common/metrics",
    "leptos/ssr",
    "leptos_meta/ssr",
    "leptos_router/ssr",
    "dep:leptos_axum",
    "dep:axum",
    "dep:base64",
    "dep:chrono",
    "dep:storage",
    "dep:email_address",
    "dep:tracing",
]
```

- [x] **Step 2: Add `csr` to the workspace members.** In root `Cargo.toml` (lines 3-9),
  add `"csr",`:

```toml
members = [
  "common",
  "csr",
  "hydrate",
  "server",
  "storage",
  "web"
]
```

- [x] **Step 3: Create `csr/Cargo.toml`** (mirrors `hydrate/Cargo.toml`, swapping the
  client feature):

```toml
[package]
name = "csr"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
web = { path = "../web", default-features = false, features = ["csr"] }
leptos = { workspace = true, features = [ "csr" ] }

console_error_panic_hook.workspace = true
console_log.workspace = true
log.workspace = true
wasm-bindgen.workspace = true

[lints]
workspace = true
```

- [x] **Step 4: Create `csr/src/lib.rs`.** Mount `App` client-side; reuse the existing
  `data-hydrated` body marker (so the e2e `waitForHydration` helper works unchanged —
  see Task 5):

```rust
// web::App's ParentRoute generates a wide route tuple; raise the recursion limit
// to monomorphize it (mirrors hydrate/src/lib.rs and web/src/lib.rs).
#![recursion_limit = "512"]

// The e2e suite waits on `body[data-hydrated]` (end2end/tests/hydration.ts) as the
// "app is mounted and interactive" signal. CSR has no hydration, but the same marker
// cleanly means "mount_to_body done" here, so the specs need no changes.
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
    export function mark_ready() {
        if (document && document.body) {
            document.body.setAttribute('data-hydrated', 'true');
        }
    }
")]
extern "C" {
    fn mark_ready();
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    use web::App;
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
    mark_ready();
}
```

- [x] **Step 5: Verify the CSR wasm compiles.**

Run: `cargo build -p csr --target wasm32-unknown-unknown --release`
Expected: builds clean. If `leptos/csr` alone leaves `leptos_router`/`leptos_meta`
unsatisfied (a compile error about a missing reactive impl), add the matching client
features to the `web` `csr` feature (e.g. `"leptos_meta/csr"` is NOT a feature — meta/router
run client-side without an `ssr` flag, so a failure here means a genuinely missing symbol,
not a feature toggle; investigate the exact error before adding anything). The hydrate
precedent (`leptos/hydrate` only) predicts `leptos/csr` alone suffices.

- [x] **Step 6: Verify the default build is untouched.**

Run: `cargo xtask check --no-test`
Expected: green (the new crate compiles for the host too; default SSR build unaffected).

- [x] **Step 7: Commit.**

```bash
git add csr/ web/Cargo.toml Cargo.toml Cargo.lock
git commit -m "feat(web): add leptos-CSR mount crate + web csr feature (spike #177)"
```

---

### Task 2: `jaunder` server `csr` feature — static-shell router

**Files:**
- Modify: `server/Cargo.toml` (add a `[features]` section after line 58)
- Modify: `server/src/lib.rs` (imports lines 25-29; route wiring lines 41, 102-117)
- Create: `csr/index.html`

**Interfaces:**
- Consumes: `LeptosOptions` (its `site_root`), the existing `create_router` params.
- Produces: under `--features csr`, a router that serves `<site_root>/index.html` as the
  SPA fallback and `<site_root>/pkg/*` static assets, with **no**
  `leptos_routes_with_context` and **no** `file_and_error_handler(shell)`. The
  `/api/{*fn_name}` server-fn route and all raw HTTP routes (feed, media, atompub, style)
  are unchanged.

- [x] **Step 1: Add the `csr` feature to `server/Cargo.toml`.** After line 58, add:

```toml
[features]
# Spike #177: build the server without the leptos reactive page render. `web/ssr`
# stays on (the #[server] fn bodies must exist server-side); only the page-route
# rendering is swapped for a static CSR shell in create_router.
csr = []
```

- [x] **Step 2: cfg-gate the SSR-only imports in `server/src/lib.rs`.** Replace lines
  27-29:

```rust
use axum::Router;
use axum_embed::ServeEmbed;
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use web::{shell, App};
```

  with:

```rust
use axum::Router;
use axum_embed::ServeEmbed;
use leptos::prelude::*;
#[cfg(not(feature = "csr"))]
use leptos_axum::{generate_route_list, LeptosRoutes};
#[cfg(not(feature = "csr"))]
use web::{shell, App};
```

- [x] **Step 3: cfg-gate `generate_route_list` and the render-route block.** In
  `create_router`, the `let routes = generate_route_list(App);` line (server/src/lib.rs:41)
  becomes:

```rust
    #[cfg(not(feature = "csr"))]
    let routes = generate_route_list(App);
```

- [x] **Step 4: Branch the page-render wiring.** Replace the `.leptos_routes_with_context(
  ... ).fallback(leptos_axum::file_and_error_handler(shell))` chain (server/src/lib.rs:102-117)
  so the `app` builder splits the render tail by feature. Concretely, end the common
  builder at the feed routes (keep `.route("/~{username}/tags/{tag}/feed.{ext}", ...)` as
  the last common `.route`), bind it to `let app = ...;`, then:

```rust
    // --- SSR (default): the leptos reactive page render. The #173 panic home. ---
    #[cfg(not(feature = "csr"))]
    let app = app
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            move || {
                crate::context::provide_app_state_contexts(&state);
                crate::context::provide_mailer_context(&leptos_mailer);
                provide_context(web::auth::CookieSettings {
                    secure: secure_cookies,
                });
            },
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler(shell));

    // --- CSR (spike #177): no reactive render. Serve the static SPA shell + site
    //     assets (pkg/*). Server fns (/api) and raw HTTP routes are untouched above. ---
    #[cfg(feature = "csr")]
    let app = {
        use tower_http::services::{ServeDir, ServeFile};
        // `state`/`leptos_mailer` are consumed by the SSR context closure above; under
        // csr they're unused here. Silence that without dropping them from the signature.
        let _ = (&state, &leptos_mailer, secure_cookies);
        let site_root = leptos_options.site_root.to_string();
        let index_html = format!("{site_root}/index.html");
        app.fallback_service(ServeDir::new(&site_root).fallback(ServeFile::new(index_html)))
    };
```

  Note: keep the trailing `.layer(...)` extension chain and the
  `crate::observability::with_http_observability(app).with_state(leptos_options)` return
  **after** this block, applied to the now-feature-selected `app`. (The `.layer` calls and
  return are common to both branches — leave them where they are, operating on `app`.)

- [x] **Step 5: Create `csr/index.html`** — the static CSR shell. Mirrors `web::shell`'s
  head (same stylesheets, served from `/style` embedded assets) but with an empty body +
  a module boot script; `#[wasm_bindgen(start)]` runs `main()` on `init()`:

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <link rel="stylesheet" href="/style/jaunder.css" />
    <link rel="stylesheet" href="/style/jaunder-themes.css" />
  </head>
  <body>
    <script type="module">
      import init from "/pkg/jaunder.js";
      init();
    </script>
  </body>
</html>
```

- [x] **Step 6: Verify the CSR server compiles.**

Run: `cargo build -p jaunder --features csr`
Expected: builds clean. Common compile snags to expect and fix in place: an unused-import
warning (the `#[cfg]`-gated imports cover this) or an unused-variable on `state`/`mailer`
(the `let _ = (...)` covers this). `unwrap`/`expect` are denied — none introduced here.

- [x] **Step 7: Verify the default SSR build + gate are untouched.**

Run: `cargo xtask check --no-test`
Expected: green. (The default build compiles `create_router` with the `not(csr)` branch,
identical to today.)

- [x] **Step 8: Commit.**

```bash
git add server/Cargo.toml server/src/lib.rs csr/index.html
git commit -m "feat(server): csr feature serves static SPA shell, no reactive render (spike #177)"
```

---

### Task 3: Nix CSR build derivations + `packages` exposure

**Files:**
- Modify: `flake.nix` (add derivations next to `hydrateWasm`/`wasmBundle`/`jaunderBin`/`site`,
  ~lines 293-424; add `packages` entries)

**Interfaces:**
- Consumes: `craneLib`, `commonArgs`, `cargoArtifacts`, `wasm-bindgen-cli`, `./public`.
- Produces: `csrWasm`, `csrWasmBundle`, `jaunderBinCsr`, `csrSite` let-bindings; flake
  `packages.<system>.{csr-server,csr-site}` for standalone `nix build` verification.

- [x] **Step 1: Read the exact surrounding bindings first.** Read `flake.nix` lines
  290-425 to confirm the `let`-scope where `hydrateWasm`/`wasmBundle`/`jaunderBin`/`site`
  live, and that `commonArgs`/`cargoArtifacts` are in scope. Add the new bindings in the
  same scope, immediately after `site` (flake.nix:424).

- [x] **Step 2: Add the four CSR derivations** after the `site` binding:

```nix
csrWasm = craneLib.buildPackage (
  commonArgs
  // {
    cargoArtifacts = craneLib.buildDepsOnly (
      commonArgs
      // {
        CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
        cargoExtraArgs = "-p csr";
        doCheck = false;
      }
    );
    CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
    cargoExtraArgs = "-p csr";
    doCheck = false;
    installPhaseCommand = ''
      mkdir -p $out/lib
      cp target/wasm32-unknown-unknown/release/csr.wasm $out/lib/
    '';
  }
);

csrWasmBundle =
  pkgs.runCommand "jaunder-csr-wasm-bundle"
    {
      nativeBuildInputs = [
        wasm-bindgen-cli
        pkgs.gnused
      ];
    }
    ''
      mkdir -p $out
      wasm-bindgen \
        --target web \
        --out-dir $out \
        ${csrWasm}/lib/csr.wasm
      # Rename to the "jaunder" output-name the CSR shell's <script> imports.
      mv $out/csr.js $out/jaunder.js
      mv $out/csr_bg.wasm $out/jaunder_bg.wasm
      sed -i 's/csr_bg\.wasm/jaunder_bg.wasm/g' $out/jaunder.js
    '';

jaunderBinCsr = craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;
    cargoExtraArgs = "-p jaunder --features csr";
    doCheck = false;
  }
);

csrSite = pkgs.runCommand "jaunder-csr-site" { } ''
  mkdir -p $out/pkg
  cp -r ${csrWasmBundle}/. $out/pkg/
  cp -r ${./public}/. $out/
  cp ${./csr/index.html} $out/index.html
'';
```

- [x] **Step 3: Expose them as flake packages** for standalone verification. Find the
  `packages` attrset in the flake's per-system outputs (search `packages =` /
  `packages.default`) and add:

```nix
csr-server = jaunderBinCsr;
csr-site = csrSite;
```

- [x] **Step 4: Verify the CSR site builds.**

Run: `nix build .#csr-site -L`
Expected: builds; `ls -R ./result` shows `pkg/jaunder.js`, `pkg/jaunder_bg.wasm`,
`index.html`, and the public assets.

- [x] **Step 5: Verify the CSR server builds.**

Run: `nix build .#csr-server -L`
Expected: builds `result/bin/jaunder`.

- [x] **Step 6: Commit.**

```bash
git add flake.nix
git commit -m "build(nix): csr wasm/server/site derivations + packages (spike #177)"
```

---

### Task 4: CSR e2e `nixosTest` at `workers:1` — get the existing specs green under CSR

**Files:**
- Modify: `flake.nix` (add a CSR playwright config; a `mkCsrE2eCombo` mirroring
  `mkE2eCombo`; a `checks.<system>.e2e-csr-postgres-chromium` entry)
- Possibly modify: a few `end2end/tests/*.spec.ts` if they assert server-painted content
  that only exists post-mount under CSR.

**Interfaces:**
- Consumes: `jaunderBinCsr`, `csrSite`, the existing e2e plumbing (`e2eRunAndCapture`,
  `e2ePanicGate`, seed scripts, `self.nixosModules.jaunder`).
- Produces: `checks.<system>.e2e-csr-postgres-chromium` — a nixosTest booting the CSR
  server (postgres backend) and running the Playwright suite at `workers:1`.

- [x] **Step 1: Read the exact e2e machinery first.** Read `flake.nix` around the
  `nixosTest` definition and `mkE2eCombo`/`e2eCombos`/`e2eWarmChecks` (the extraction
  pointed at ~629-717 and ~847-879; read the full `mkE2eCombo` body and the `nixPlaywrightConfig`
  / VM-node config it uses). Identify exactly how `mkE2eCombo` selects backend and wires
  the jaunder systemd service (it imports `self.nixosModules.jaunder`, which hardcodes
  `${jaunderBin}` + `${site}`).

- [x] **Step 2: Add a CSR playwright config** beside `nixPlaywrightConfig` (flake.nix:430).
  Start at `workers:1` (same as today) so this task isolates "do the specs pass under CSR"
  from "does concurrency panic." Copy `nixPlaywrightConfig` verbatim and rename the binding
  to `csrPlaywrightConfig` (identical body for now; Task 5 flips its `workers`/`fullyParallel`).

- [x] **Step 3 (refined): parametrize `mkE2ePostgresCheck`** instead of a separate
  `mkCsrE2eCombo` — added `jaunderPkg`/`sitePkg`/`playwrightConfig`/`vmMemory`/`vmCores`
  args defaulting to the SSR values, with the binary/site/ExecStart overrides
  `lib.mkIf (jaunderPkg != jaunderBin)`-guarded. Verified the default
  `e2e-postgres-chromium` drvPath is byte-identical (stash-compare), so the existing
  checks are provably untouched — cleaner and safer than duplicating the builder.
  Original text: Add `mkCsrE2eCombo` mirroring `mkE2eCombo`, overriding the jaunder
  service to the CSR binary + site and using `csrPlaywrightConfig`. The override (applied
  in the test's `nodes.machine` config, alongside the existing `imports = [
  self.nixosModules.jaunder ]`):

```nix
systemd.services.jaunder.preStart = lib.mkForce ''
  mkdir -p target
  ln -sfn ${csrSite} target/site
  ${jaunderBinCsr}/bin/jaunder init --db "$JAUNDER_DB" --skip-if-exists
'';
systemd.services.jaunder.serviceConfig.ExecStart =
  lib.mkForce "${jaunderBinCsr}/bin/jaunder serve";
```

  Keep `virtualisation.memorySize = 6144` and add `virtualisation.cores = 4` on the CSR
  node now (the campaign needs it; harmless at `workers:1`). Wire the test to copy
  `csrPlaywrightConfig` to `/tmp/e2e/playwright.nix.config.js` instead of
  `nixPlaywrightConfig`. Keep `e2ePanicGate "postgres"`.

- [x] **Step 4: Register the check.** Add a single CSR combo (postgres+chromium) to the
  generated checks:

```nix
e2e-csr-postgres-chromium = mkCsrE2eCombo {
  backend = "postgres";
  browser = "chromium";
  traceDigit = "5";
  warmupEnv = " JAUNDER_E2E_WARMUP=1";
};
```

  (Merge this into the same `checks` attrset that holds `e2eWarmChecks`; pick an unused
  `traceDigit`.)

- [x] **Step 5: Build the CSR e2e check and triage fallout.** RESULT: green on the
  first run — **all 66 tests passed (2.9m), zero panics, NO spec edits required.** The
  reused `data-hydrated` marker + Playwright's auto-waiting handled content-after-mount
  cleanly; no spec asserted pre-mount server-painted content in a way that raced.

Run: `nix build .#checks.x86_64-linux.e2e-csr-postgres-chromium -L --keep-failed`
Expected (first run): may fail on specs that assert content present *before* JS
(CSR paints nothing until the wasm mounts). For each failure, inspect the captured
trace/journal under `.xtask/diagnostics/` or the `--keep-failed` build dir. Fix by making
the spec wait for `waitForHydration` (the CSR-ready marker) before asserting content, OR —
if a spec genuinely tests no-JS/SEO server-painted HTML — mark it `test.skip` under CSR
with a comment referencing #178 (the projector restores server-painted content; out of
scope for the spike). Re-run until the CSR check is **green at `workers:1`**.

- [x] **Step 6: Verify the default SSR e2e is untouched.** Proven two ways: zero
  `end2end/` edits (`git diff end2end/` empty) AND the default `e2e-postgres-chromium`
  drvPath is byte-identical with/without the flake edits (stash-compare).

Run: `git diff main...HEAD -- end2end/ | rg -n 'data-hydrated|waitForHydration'`
Expected: any spec edits are additive waits / CSR-scoped skips; the existing
`e2e-{backend}-{browser}` checks and `nixPlaywrightConfig` are unchanged. Optionally
sanity-run one default combo: `nix build .#checks.x86_64-linux.e2e-postgres-chromium -L`.

- [x] **Step 7: Commit.**

```bash
git add flake.nix end2end/
git commit -m "test(e2e): CSR e2e check green at workers:1 (spike #177)"
```

---

### Task 5: Flip the CSR e2e to `workers:4` + `fullyParallel`

**Files:**
- Modify: `flake.nix` (`csrPlaywrightConfig` only)

- [ ] **Step 1: Set concurrency** in `csrPlaywrightConfig`: change `workers: 1` to
  `workers: 4` and add `fullyParallel: true` to the `defineConfig({...})` object. Leave
  `nixPlaywrightConfig` (the SSR one) at `workers: 1`.

- [ ] **Step 2: Verify a single concurrent run passes (bypassing the nix cache).**

Run: `nix build .#checks.x86_64-linux.e2e-csr-postgres-chromium --rebuild -L --keep-failed`
Expected: green (no `already been disposed`, no other panic). If it OOMs or times out,
confirm the node has `cores=4`/`memorySize=6144` (Task 4 Step 3). A genuine
`already been disposed` panic here is a **NO-GO signal** — stop and escalate (see Task 6).

- [ ] **Step 3: Commit.**

```bash
git add flake.nix
git commit -m "test(e2e): CSR e2e at workers:4 + fullyParallel (spike #177)"
```

---

### Task 6: Run the campaign (scratch) + write the findings doc

**Files:**
- Create (scratchpad, **NOT committed**): a campaign loop script.
- Create: `docs/issue-177-csr-spike-findings.md`

- [ ] **Step 1: Write the campaign loop** to
  `/tmp/claude-1000/-home-mdorman-src-jaunder/9e2d69ce-26ee-43e5-b391-00b10401ef73/scratchpad/csr-campaign.sh`.
  It loops ~30×: each iteration runs
  `nix build .#checks.x86_64-linux.e2e-csr-postgres-chromium --rebuild -L --keep-failed`
  (the `--rebuild` defeats the nix e2e cache so every iteration actually re-runs the VM),
  records exit code, and on failure greps the captured journal
  (`.xtask/diagnostics/e2e-csr-postgres-chromium/` and/or the `--keep-failed` dir) for
  `already been disposed` to classify **PANIC** vs **OTHER**; success → **PASS**. Tally and
  print `PASS/PANIC/OTHER` counts and the first failing run index.

```bash
#!/usr/bin/env bash
set -uo pipefail
RUNS="${1:-30}"
CHECK=".#checks.x86_64-linux.e2e-csr-postgres-chromium"
pass=0; panic=0; other=0; first_fail=""
for i in $(seq 1 "$RUNS"); do
  echo "=== run $i/$RUNS ==="
  if nix build "$CHECK" --rebuild -L --keep-failed > "/tmp/csr-run-$i.log" 2>&1; then
    pass=$((pass+1)); echo "run $i: PASS"
  else
    if rg -q "already been disposed" "/tmp/csr-run-$i.log"; then
      panic=$((panic+1)); echo "run $i: PANIC"
    else
      other=$((other+1)); echo "run $i: OTHER"
    fi
    [ -z "$first_fail" ] && first_fail="$i"
  fi
done
echo "TOTAL: PASS=$pass PANIC=$panic OTHER=$other first_fail=${first_fail:-none}"
```

- [ ] **Step 2: Run the campaign** (long-running — use the Bash tool's background mode;
  ~30 VM boots).

Run: `bash <scratchpad>/csr-campaign.sh 30`
Expected: `PANIC=0`. (`OTHER` failures are infra flakes — VM boot, OOM, timeout — not the
#173 class; investigate any, re-run if infra, but they don't fail the gate. Document them.)

- [ ] **Step 3: Write `docs/issue-177-csr-spike-findings.md`** recording: the exact run
  count, the PASS/PANIC/OTHER tally, the verdict (**GO** if PANIC=0, **NO-GO** otherwise),
  the recipe used (postgres+chromium, workers:4, fullyParallel, cores=4/6144,
  `nix build --rebuild`), and — if NO-GO — the exact panic site + run index (this would
  contradict the root-cause analysis, so capture it precisely and escalate the framework
  decision toward Dioxus per the issue). Reference the #173 baseline (~12%, first panic
  ~run 7) for contrast.

- [ ] **Step 4: Commit the findings doc.**

```bash
git add docs/issue-177-csr-spike-findings.md
git commit -m "docs(#177): leptos-CSR spike findings (campaign result + verdict)"
```

---

### Task 7: ADR confirming leptos-CSR + `docs/README.md` row

**Files:**
- Create: `docs/adr/0040-web-rendering-leptos-csr.md` (confirm the number)
- Modify: `docs/README.md` (ADR table)

- [ ] **Step 1: Confirm the next ADR number.** Run
  `ls docs/adr/ | sort | tail -3` — take highest + 1 (expected `0040`; memory references
  ADR-0039). Use that number in the filename and table.

- [ ] **Step 2: Write the ADR.** Title: "Web rendering: leptos-CSR (drop concurrent
  reactive SSR)". Status `accepted`. Content: the #173 forcing function (concurrent-SSR
  reactive-disposal, upstream NOT_PLANNED), the decision (CSR-only for the web leg,
  narrowing ADR-0002 from "anything" to a Rust client framework → leptos-CSR), the
  spike evidence (link `docs/issue-177-csr-spike-findings.md`: N runs, zero panics),
  consequences (server becomes UI-free over #178/#179/#180; relates to ADR-0039/#173/#61;
  the public projector + render-coincidence land in #178). Keep it tight; this records the
  decision, the findings doc holds the data.

- [ ] **Step 3: Add the ADR row** to the table in `docs/README.md` (number, title,
  `accepted`), matching the existing table format.

- [ ] **Step 4: Final gate.** Run the full pre-push-equivalent gate to confirm the default
  build + coverage are green (the CSR scaffolding is feature-gated, so coverage/CRAP
  baselines should be unaffected; if a `.ts` spec edit busted the coverage cache, reanchor
  per the repo's coverage workflow).

Run: `cargo xtask validate --no-e2e`
Expected: green.

- [ ] **Step 5: Commit.**

```bash
git add docs/adr/0040-web-rendering-leptos-csr.md docs/README.md
git commit -m "docs(adr): accept leptos-CSR for the web leg (#177)"
```

---

## Self-review

- **Spec coverage:** feature-gated CSR path (Tasks 1-5) ✓; whole-app CSR (`mount_to_body(App)`,
  Task 1) ✓; structural "no reactive SSR in the CSR binary" (Task 2 cfg-gate) ✓;
  VM cores=4/mem=6144 + workers:4/fullyParallel (Tasks 4-5) ✓; ~30-run postgres+chromium
  campaign classifying on `already been disposed` (Task 6) ✓; campaign harness is a scratch
  script, not merged (Task 6) ✓; findings doc (Task 6) ✓; ADR + README row (Task 7) ✓;
  default SSR build/CI untouched (every task's verify step) ✓; escalate-on-panic (Tasks 5-6) ✓.
- **Placeholder scan:** ADR number resolved in Task 7 Step 1; the only deliberately
  read-first-then-edit points are large existing-file edits (flake.nix `mkE2eCombo`,
  Tasks 3-4) where the exact surrounding nix is needed at edit time — each has a concrete
  delta + a read step.
- **Type/name consistency:** `data-hydrated` marker is set by `csr/src/lib.rs` (Task 1) and
  awaited by the unchanged `waitForHydration` (Task 4); `jaunderBinCsr`/`csrSite`/`csrWasmBundle`/
  `csrWasm`/`csrPlaywrightConfig`/`mkCsrE2eCombo`/`e2e-csr-postgres-chromium` are used
  consistently across Tasks 3-6; the `csr` feature name is identical on `web` (Task 1),
  `jaunder` (Task 2), and the nix `--features csr` (Task 3).
