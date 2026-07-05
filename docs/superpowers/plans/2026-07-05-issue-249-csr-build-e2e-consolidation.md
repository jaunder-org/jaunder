# Plan — CSR build + e2e consolidation (issue #249 cluster: #236 → #249 → #237)

**Spec:**
[`docs/superpowers/specs/2026-07-05-issue-249-csr-align-cargo-leptos.md`](../specs/2026-07-05-issue-249-csr-align-cargo-leptos.md)
**Base:** `origin/main` @ `f63ac058` (**#256/#153, #224, #145 already merged**;
ADR-0051 single Playwright config in; `xtask/src/steps/e2e_local.rs` present;
`run-e2e.sh` gone). **For agentic workers:** drive with **`jaunder-iterate`**;
delegate tasks via **`jaunder-dispatch`**. Tick checkboxes in real time. Commit
per **`jaunder-commit`** (full `cargo xtask check` via the pre-commit hook; **no
`Co-Authored-By`**).

---

## Review header (approve this layer)

### Goal

Drop cargo-leptos from the CSR **build path** and make the host e2e loop **own
its server**, sharing infrastructure with the Nix VM — one branch, separate
commits, three issues. Build tool decided: raw `wasm-bindgen` via a
`devtool csr-bundle` subcommand (not Trunk, not cargo-leptos — #236 comments).
See spec §0/§3 for the "why".

### Scope

**In (this session):**

- **Phase A / #236** — `devtool csr-bundle` (**wasm only**: `wasm-bindgen` +
  rename + js sed — exactly today's `csrWasmBundle`). Host + Nix call it; retire
  the inline Nix step and host `cargo leptos build` for the bundle.
- **Phase B / #249** — host harness owns `jaunder serve` (ephemeral port +
  runtime.json, capture env, **per-run temp DB**); `devtool seed-e2e` (shelling
  out to the `test-support` binary) shared by host + VM; retire
  `cargo leptos end-to-end` (+ its `end2end-cmd`, which would double-serve once
  `e2e-local` owns the server).
- **Phase C / #237** — embed `pkg/*` into the binary (separable tail).

**Out (follow-ups filed by Task 0):**

- Dev-loop replacement for `cargo leptos watch`/`serve` + **full** cargo-leptos
  removal (the `[[workspace.metadata.leptos]]` config + devShell entry).
  cargo-leptos stays installed for `watch`-only this session.
- Host-side diagnostics/panic-gate parity (spec §3.4); #155 flake data point;
  #212 reconcile.

**Explicitly NOT done:** Trunk migration; un-embedding `SPA_SHELL`; serving disk
`index.html`; **any SCSS/CSS work** — the served CSS
(`server/assets/jaunder{,-themes}.css`) is committed + rust-embedded
(`server/src/assets.rs`), NOT part of the bundle.

### Decoupling & ordering

Phases A and B are **independent** (spec §5; #236 comment): #249 needs _a_ built
bundle + `jaunder serve`, however produced — it does **not** depend on A4. They
share a branch for coherence and are ordered A→B→C only to keep commits legible,
not by dependency. Any phase could land alone.

### Tasks at a glance

- **T0** — Admin: file follow-ups (dev-loop; diag-gate); confirm claims; §3.2
  stays in #249.
- **Phase A — #236:**
  - **A1** — Confirm the bundle contract = today's `csrWasmBundle` (wasm only);
    record CSS is committed+embedded, `main.scss` is a dead 55-byte scaffold,
    `/pkg/jaunder.css` is confirmed-dead — so NO sass (departs from the #236
    comment).
  - **A2** — `devtool csr-bundle` subcommand (wasm-bindgen + rename + js-sed) +
    unit tests.
  - **A3** — Nix `csrWasmBundle` → crane builds `csr.wasm`, `devtool csr-bundle`
    post-processes.
  - **A4** — Host `cargo xtask build-csr` (`cargo build -p csr` +
    `devtool csr-bundle`); stop using `cargo leptos build` for the bundle.
  - **A5** — Parity: **Nix** bundle byte-identical (same release input);
    **host** debug bundle functional-only; `cargo xtask validate` green.
- **Phase B — #249 (independent of A — needs _a_ build command, satisfied by A4
  in-branch):**
  - **B1** — `devtool seed-e2e` (**seed only**, shells to `test-support`) + add
    `devtoolBin` to e2e guests + tests; flake `seed_db()` (`759`/`890`) uses it
    (keeps its own reset).
  - **B2** — Rework `e2e_local.rs`: build (build-csr) → per-run temp DB → serve
    on `:0` + runtime.json discovery + capture env → `seed-e2e` →
    playwright(baseURL, **preserves single-test filter**) → teardown by PID;
    retire the `:3000`-wait contract.
  - **B3** — Parameterize `baseURL` in `playwright.config.ts` (host feeds
    discovered port; VM feeds `:3000`).
  - **B4** — Retire `cargo leptos end-to-end`: drop `end2end-cmd`/`end2end-dir`
    (works today, but would double-serve post-B2).
  - **B5** — Verify: `e2e-local` green from clean; no orphan; concurrent runs
    isolated; **single-test isolated run** works.
- **Phase C — #237 (after A; separable):**
  - **C1** — Embed `pkg/*` into the binary; serve from memory.
  - **C2** — Verify: single binary serves the SPA with no on-disk `site_root`.

### Key risks / decisions

- **`csr-bundle` is wasm-only.** No sass (served CSS is committed +
  rust-embedded; `main.scss` is a dead scaffold, `/pkg/jaunder.css`
  confirmed-dead — CI omits it, green), no wasm-opt (would break the Nix
  byte-parity net). Both are out of this cluster. This **departs from the #236
  decision comment** that over-scoped `dart-sass` into `csr-bundle`.
- **Byte-parity is Nix-only.** `csrWasmBundle` runs over **release** `csr.wasm`;
  the host builds **debug**, so host bytes never matched Nix. A5 asserts
  byte-parity only for the Nix path (same release input); host is
  functional-parity.
- **`seed-e2e` is seed-only.** The SQLite VM reset
  (`systemctl stop`→wipe→`start`, `flake.nix:765`) also clears server caches —
  an in-guest CLI can't do that, so reset stays environment-specific (host temp
  DB; VM systemctl). B1 must also add `devtoolBin` to the e2e guests (it's
  absent today).
- **Crane dep-cache preserved** — Nix keeps building `csr.wasm` via crane
  (`csrWasm`, `flake.nix:472`); `devtool csr-bundle` only post-processes over
  `${csrWasm}/lib/csr.wasm`. A3 must NOT move the cargo wasm build into a
  `runCommand`.
- **Per-run temp DB (B2)** — ephemeral ports isolate the _server_; a temp
  storage dir per run isolates the _DB_. Both are required for the "concurrent
  runs" criterion and to stop polluting `data/jaunder.db`. Resolves spec §8's
  DB-location question: **temp dir**.
- **cargo-leptos removal is staged** — build path now; `watch`/`serve` + config
  later (T0 follow-up). No dev-loop regression mid-session.
- **No runtime coupling to cargo-leptos** (verified): `LeptosOptions` is built
  in code (`commands.rs`), not via `get_configuration()`/`LEPTOS_OUTPUT_NAME`,
  so dropping it as the _runner_ doesn't touch `jaunder serve`.

---

## Global constraints

- `devtool` lives in the **separate `tools/` workspace** (`tools/devtool/`),
  crane-built as `devtoolBin`, and runs in-sandbox (`devtool coverage emit`,
  `flake.nix:~1279`). New subcommands go there, NOT `xtask` (host-only). It can
  call the **`test-support` binary** (available in-sandbox) but must NOT link
  the `test-support` _crate_ (main workspace — would defeat `tools/`'s lean
  crane build).
- Every commit passes `cargo xtask check` (pre-commit hook). Run it before
  committing.
- Storage-touching tests follow the dual-backend template (`CONTRIBUTING.md`
  backend parity); `seed-e2e`'s reset touches both SQLite + Postgres.
- Each phase = its own commit(s), referencing the phase's issue
  (`#236`/`#249`/`#237`).
- Bundle outputs keep the fixed `/pkg/jaunder.{js,wasm}` URLs and the embedded
  `SPA_SHELL`
  - #181/#234 drift guards **unchanged**.

---

## Task 0 — Admin / issue hygiene

- [x] File the **dev-loop replacement** follow-up → **#268** (blocked-by
      #236/#249/#237).
- [x] File the **host-side diagnostics/panic-gate parity** follow-up → **#269**
      (blocked-by #249).
- [x] #236 + #249 **In Progress**; #237 set at Phase C.
- [x] §3.2 decision: `devtool seed-e2e` lands **inside #249** (B1) — kept.

---

## Phase A — #236: `devtool csr-bundle` (wasm only), drop cargo-leptos from the build path

### A1 — Confirm the bundle contract (discovery)

- [x] Confirmed the served asset contract: the shell + projector reference
      `/pkg/jaunder.js`, `/pkg/jaunder.wasm` (`web/src/render/mod.rs`,
      `csr/index.html`), and `/style/jaunder.css`
  - `/style/jaunder-themes.css` served from the **committed, rust-embedded**
    `server/assets/` (`server/src/assets.rs`). The bundle produces **only**
    `pkg/jaunder.{js,wasm}`.
- [x] Recorded the exact current bundle steps as the A5 reference:
      `csrWasmBundle` (`flake.nix:493-511`) = `wasm-bindgen --target web` →
      `mv csr.js jaunder.js` → `mv csr_bg.wasm jaunder.wasm` →
      `sed 's/csr_bg\.wasm/jaunder.wasm/g' jaunder.js`.
- [x] Confirmed-dead facts (recorded on #236, comment correcting the sass
      over-scope): `style/main.scss` is a **55-byte `cargo leptos new`
      scaffold** (untouched); the served `/style/jaunder.css` is the 27KB
      committed `server/assets/jaunder.css` (rust-embedded).
      `web/src/pages/mod.rs:122` injects a
      `<Stylesheet href="/pkg/jaunder.css">` that the wasm-only bundle never
      produces — Nix CI already omits it and is green, proving it
      non-load-bearing. So **`csr-bundle` is wasm-only, no sass** — this
      deliberately **departs from the #236 comment** that had scoped
      `dart-sass main.scss→jaunder.css` into `csr-bundle` (that output is dead).
      Optionally drop the `mod.rs:122` line (cosmetic; a console 404 today).
- **No code.** Output: a one-paragraph contract note on #236 (correcting the
  sass over-scope).

### A2 — `devtool csr-bundle` subcommand (wasm only)

- **Files:** `tools/devtool/src/main.rs` (add `CsrBundle(CsrBundleArgs)`), new
  `tools/devtool/src/csr_bundle.rs`.
- **Behaviour:** `devtool csr-bundle --wasm <csr.wasm> --out <dir>` runs
  `wasm-bindgen --target web --out-dir <dir> <wasm>`, then
  `csr.js`→`jaunder.js`, `csr_bg.wasm`→`jaunder.wasm`, and rewrites
  `csr_bg.wasm`→`jaunder.wasm` in the js. Shells out to `wasm-bindgen` from PATH
  (devShell) / `nativeBuildInputs` (Nix). **No sass, no wasm-opt.**
- **Test:** `tools/devtool/src/csr_bundle.rs` `#[cfg(test)]` — unit-test the
  pure logic: the `csr.*`→`jaunder.*` filename map and the js string-rewrite
  (feed a fixture js string, assert `csr_bg.wasm` becomes `jaunder.wasm`); arg
  parsing. (Real `wasm-bindgen` invocation is validated by A3/A5 where the tool
  exists.)
- **Run:** `cargo nextest run -p devtool csr_bundle` → PASS.
- **Commit** (`#236`):
  `feat(devtool): csr-bundle — shared wasm-bindgen post-processing`.

### A3 — Wire the Nix bundle to `devtool csr-bundle`

- **Files:** `flake.nix` (`csrWasmBundle`, `493-511`).
- **Change:** keep `csrWasm` (crane, `472`) untouched (dep-cache intact).
  Replace the inline `wasm-bindgen`/`mv`/`sed` in `csrWasmBundle`'s `runCommand`
  with `devtool csr-bundle --wasm ${csrWasm}/lib/csr.wasm --out $out`; add
  `devtoolBin` + `wasm-bindgen-cli` to `nativeBuildInputs`.
- **Run:**
  `devtool run -- nix build .#checks.x86_64-linux.<bundle-consuming check>`;
  confirm `$out` matches the A1 reference tree; a full e2e check builds green.
- **Commit** (`#236`):
  `refactor(nix): build CSR bundle via devtool csr-bundle (retire inline wasm-bindgen)`.

### A4 — Host bundle build without cargo-leptos

- **Files:** `xtask/src/lib.rs` + `xtask/src/steps/` (new `build_csr` step);
  devShell (`flake.nix:~1279`) already has `wasm-bindgen-cli`.
- **Change:** `cargo xtask build-csr` =
  `cargo build -p csr --target wasm32-unknown-unknown` then
  `devtool csr-bundle --wasm target/wasm32-unknown-unknown/<profile>/csr.wasm --out target/site/pkg`.
  Server bin via `cargo build -p jaunder`. Host no longer needs
  `cargo leptos build`.
- **Run:** `cargo xtask build-csr` then `jaunder serve` → hydrating page (manual
  smoke).
- **Commit** (`#236`):
  `feat(xtask): build-csr — host CSR bundle via devtool (no cargo-leptos)`.

### A5 — Parity verification

- [x] **Nix path — byte parity:** built the new `devtool`-based `site` and the
      old inline one from `origin/main`; `diff -r` of `pkg/` → **IDENTICAL**.
- [x] **Host path — functional parity:** `cargo xtask build-csr` produces
      `target/site/pkg/jaunder.{js,wasm}` (debug; 0 `csr_bg.wasm` refs in the
      js). (Note: the `cargo xtask build-csr` wrapper needs a devShell whose
      `devtool` post-dates A2 — a fresh checkout/CI rebuilds it; verified via
      source-run.)
- [x] `/pkg/*` URLs + `SPA_SHELL` + guards still pass; `cargo xtask validate` →
      **green** (all `{sqlite,postgres}×{chromium,firefox}` combos + coverage).
- **Commit** (`#236`, if fixups):
  `test(nix): confirm devtool csr-bundle byte-parity (Nix) + functional (host)`.

---

## Phase B — #249: host e2e loop owns the server _(independent of Phase A)_

### B1 — `devtool seed-e2e` (seed only; shells out to `test-support`)

**Scope correction (cold review):** `devtool seed-e2e` owns **only the seed** —
the canonical fixture arg-list — not the reset. The VM's SQLite reset is
`systemctl stop jaunder` → `rm -rf /var/lib/jaunder/data` → `start`
(`flake.nix:765-767`): it stops the service first (can't wipe an open SQLite
file) and the restart also clears the server's in-memory caches. An in-guest CLI
can do neither, so **reset stays environment-specific**: host = per-run temp DB
(no reset); VM = its existing Python/systemctl reset (unchanged). What's deduped
is the fixture arg-list (the real "must match by comment" hazard).

- **Files:** `tools/devtool/src/main.rs` (`SeedE2e`), new
  `tools/devtool/src/seed_e2e.rs`; **`flake.nix`** — add `devtoolBin` to the e2e
  guest `environment.systemPackages` (`730` sqlite, `828` postgres — today they
  list `testSupportBin` but **not** `devtoolBin`, so `devtool` isn't on the
  guest PATH; B1-1).
- **Behaviour:** `devtool seed-e2e` invokes the **`test-support` binary** with
  the canonical fixture args (3 users, 2 site-config keys, `reset-mail` — the
  args `e2e_local.rs:86-153` and `seed_db` use today). devtool owns the
  arg-list; `test-support` remains the impl (devtool cannot link it — separate
  `tools/` workspace).
- **Test:** dual-backend (`CONTRIBUTING.md`) — seed yields expected fixture
  state on SQLite and Postgres; idempotent (re-run doesn't abort on existing
  users).
- **Run:** `cargo nextest run -p devtool seed_e2e` → PASS.
- **Change flake `seed_db()`** (`759` sqlite / `890` postgres): keep its
  existing reset, replace the inline `test-support` seed calls with
  `devtool seed-e2e`.
- **Commit** (`#249`):
  `feat(devtool): seed-e2e — shared e2e fixture seed for host and VM`.

### B2 — Harness owns `jaunder serve` (ephemeral port + temp DB)

Today `e2e_local.rs` builds only `test-support`, **waits** for a server on
`:3000`, seeds, runs Playwright — the bundle+server are built/started by
`cargo leptos end-to-end` (`Cargo.toml:134` `end2end-cmd`→`e2e-local`). This
task inverts that: `e2e-local` builds the bundle+server and **owns** the server.

- **Files:** `xtask/src/steps/e2e_local.rs` (rewrite the module).
- **Build step (names the S2 gap):** `cargo xtask build-csr` (A4) when Phase A
  has landed in-branch; otherwise `cargo leptos build`. Plus
  `cargo build -p jaunder` for the server bin. _(This is the only place #249
  touches the bundle build — hence "independent of A" ⇒ "needs **a** build
  command," satisfied by A4 in this branch.)_
- **Change:** build → **per-run temp storage dir + DB** → spawn `jaunder serve`
  child with `JAUNDER_BIND=127.0.0.1:0`, per-run `JAUNDER_RUNTIME_FILE`, capture
  env (`JAUNDER_MAIL_CAPTURE_FILE`, `JAUNDER_WEBSUB_CAPTURE_FILE`, diag-log),
  **dev environment** (schema auto-inits; `commands.rs:~385-401`) → poll
  runtime.json for `{ip,port}` then HTTP-ready → `devtool seed-e2e` (temp DB is
  fresh, no reset) → playwright with `baseURL=http://ip:port` → **teardown by
  tracked PID on every exit path**.
- **Preserve the single-test filter:** the existing optional `test` arg
  (`lib.rs:118/343`, `e2e_local.rs` `test_filter`) must survive the rewrite — it
  passes through to Playwright (`file.spec.ts:line` / path). Consider adding
  `-g "<title>"` grep so a single test-by-name also works (today's single
  positional handles file:line/path only).
- **Test:** a failing-playwright run reaps the server child (no orphan); two
  invocations get distinct ports **and** temp DBs; `e2e-local <spec>` runs only
  the matching test.
- **Commit** (`#249`):
  `feat(xtask): e2e-local owns an ephemeral-port jaunder serve (VM parity)`.

### B3 — Parameterize `baseURL`

- **Files:** `end2end/playwright.config.ts` / `end2end/tests/helpers.ts` (the
  `BASE_URL`), `flake.nix` (VM feeds its `:3000`).
- **Change:** `BASE_URL` reads an env var both host (discovered port) and VM
  feed; remove the hardcoded `:3000`. (Spec §8: VM stays fixed `:3000` unless we
  opt it into ephemeral too.)
- **Commit** (`#249`):
  `refactor(e2e): parameterize Playwright baseURL for host+VM`.

### B4 — Retire `cargo leptos end-to-end`

- **Files:** `Cargo.toml` (`[[workspace.metadata.leptos]]`).
- **Rationale (corrected):** `end2end-cmd = "cargo run … -- e2e-local"`
  (`Cargo.toml:134`) currently **works** — cargo-leptos builds+serves on
  `:3000`, then runs `e2e-local`. It is **not** broken. But once B2 makes
  `e2e-local` build+serve its **own** ephemeral server,
  `cargo leptos end-to-end` would **double-serve** (cargo-leptos's `:3000` +
  e2e-local's ephemeral). So retire it.
- **Change:** drop `end2end-cmd`/`end2end-dir`; `cargo xtask e2e-local` is the
  host e2e entry point. (Leaves the rest of the leptos config for `watch`; full
  removal = dev-loop follow-up.)
- **Commit** (`#249`):
  `chore(build): retire cargo leptos end-to-end (e2e-local now owns the loop)`.

### B5 — Verify

- [ ] `cargo xtask e2e-local` green from a clean checkout; mail/websub specs
      pass (capture env present); each run fresh DB; a failed run leaves no
      orphan; two concurrent runs don't collide (distinct ports + temp DBs).
- [ ] **Single-test dev loop:** `cargo xtask e2e-local <spec.ts[:line]>` runs
      _only_ that test, fully self-contained (builds, spins up its own ephemeral
      server + temp DB, seeds, runs it, tears down) — no pre-existing server, no
      `:3000` conflict with a running dev server. This is the headline DX win
      for agents iterating on one e2e test.
- [ ] `cargo xtask validate` green (VM `seed_db` now via `devtool seed-e2e`).

---

## Phase C — #237: single-binary embed _(after Phase A; separable)_

> Sketch — expand when reached; independent of Phase B.

### C1 — Embed `pkg/*` into the binary

- **Files:** `server/src/lib.rs` (currently `ServeDir(site_root)` for `pkg/*`),
  the embed mechanism (`rust_embed`, matching `server/src/assets.rs`;
  ADR-0003/0008).
- **Change:** embed the `devtool csr-bundle` output (`pkg/*`) and serve from
  memory (the shell is already embedded via `SPA_SHELL`; the CSS already via
  `StaticAssets`), so no on-disk `site_root` is required.

### C2 — Verify

- [ ] A single `jaunder serve` with **no** `target/site` on disk serves the
      SPA + assets; `cargo xtask validate` green.

---

## Self-review checklist (before HALT)

- [ ] Base is `origin/main` (post-#256); no obsolete "#256 gate" language.
- [ ] `csr-bundle` is wasm-only; no sass/CSS work anywhere; A5 byte-parity is
      achievable.
- [ ] Phase B is decoupled from Phase A; `seed-e2e` shells out to `test-support`
      (no crate link).
- [ ] Per-run temp DB is explicit (B2); the concurrency criterion is now
      founded.
- [ ] `end2end-cmd` cleanup is a task (B4), not an open question.
- [ ] Every code task has Files → Test → Run(FAIL/PASS) → Commit, or is a marked
      discovery task.
- [ ] Crane dep-cache preservation explicit in A3; cargo-leptos removal staged.
