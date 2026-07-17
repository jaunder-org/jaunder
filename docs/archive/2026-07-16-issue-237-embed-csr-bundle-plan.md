# Plan — #237: embed the CSR bundle (+ public assets) into the server binary

- Spec:
  [2026-07-16-issue-237-embed-csr-bundle.md](../specs/2026-07-16-issue-237-embed-csr-bundle.md)
- Issue: [#237](https://github.com/jaunder-org/jaunder/issues/237)

## Shape & commit strategy

Four tasks; **three code commits** (devtool → server → nix) plus a spike-first
de-risk task and a final verification task (no artifact). Linear order — each
commit builds on the last, keeps the workspace green, and is independently
verifiable:

1. **Spike (no commit)** — validate the two flagged assumptions before building:
   rust-embed `$OUT_DIR` interpolation + debug-path resolution, and how the gate
   /coverage build compiles the server (profile + whether `build.rs`'s
   fail-closed would fire). De-risks the whole design.
2. **devtool precompression** — leaf change (no server dependency).
3. **server embed + handler** — `build.rs`, `Site` embed, precompressing
   handler, wiring, unit tests.
4. **Nix wiring** — `jaunderBin` build-time bundle/public inputs; trim
   vestigial.

No separable-concerns issues to file (precompression is in-scope per the
maximalist decision; #291's host gap is closed incidentally — note on #291 at
ship, don't expand).

---

## Task 0 — Spike: validate build assumptions (no commit)

Resolve the spec's flagged risks with throwaway probes; record findings inline
here (this section becomes the design record). **If the spike falsifies the
`$OUT_DIR` approach, STOP and revisit the spec (Shape-B fallback) before
Task 2.**

- [ ] **rust-embed `interpolate-folder-path`**: confirm the feature exists in
      the pinned rust-embed 8.11 and that `#[folder = "$OUT_DIR/x"]` expands.
      Minimal probe crate or read the vendored rust-embed `README`/`utils`.
- [ ] **Debug runtime path**: confirm that with no `debug-embed`, a debug
      build's `Site::get()` reads the interpolated **absolute** `$OUT_DIR/site`
      at runtime (not re-resolved vs. `CARGO_MANIFEST_DIR`). If it re-resolves
      relatively, switch to an absolute `#[folder]` set via `build.rs`
      `println!` env or the Shape-B cfg-split.
- [ ] **crane `buildDepsOnly` + gate profile (THE crux)**:
      `cargoArtifacts =     craneLib.buildDepsOnly commonArgs` (`flake.nix:357`)
      is **shared** by `jaunderBin`/`coverage`/`clippy`/tests and builds at
      **release** profile while **running `build.rs`** with dummy sources — with
      **no** `JAUNDER_CSR_BUNDLE_DIR` and no `target/site`. So keying the panic
      on `PROFILE=="release"` would fire in `buildDepsOnly` and **kill the whole
      Nix graph**. Confirm this, then adopt the **env-keyed fail-closed rule**
      instead: `build.rs` panics **iff `JAUNDER_CSR_BUNDLE_DIR` is set AND its
      `pkg/` is missing/empty** (exactly AC2's condition). When the env is
      **unset** → tolerant path (warn + stage an empty `$OUT_DIR/site`; the
      crate compiles, serving 404s until a bundle exists). This lets the unset
      deps-only/coverage/clippy builds fall through, while the real release
      artifact (`jaunderBin`, which sets the env → Task 3) is guaranteed to
      embed a populated bundle or fail. Do **not** put the env on `commonArgs`
      (it creates a `commonArgs → csrWasmBundle → csrWasm(uses commonArgs)` eval
      cycle). Probe `buildDepsOnly`'s behavior before Task 2. **Note:** this
      refines the spec's AC2 wording (see spec) — a bare host
      `cargo build     --release` with no env is a _developer_ build (tolerant),
      not a distribution artifact; every real release path sets the env.
- [ ] **wasm-bindgen `pkg/` contents**: run `cargo xtask build-csr` and list
      `target/site/pkg/` — confirm whether a `snippets/` dir exists (the `csr`
      crate may not use JS snippets) and that `.d.ts` is present (to exclude).
- [ ] **Handler testability under empty `Site`**: confirm the coverage build
      stages an empty `Site` (env unset) and that committed fixtures are
      filtered by crane `src` — hence the handler logic must be **pure
      functions** unit- tested without a live embed (Task 2). No probe needed if
      the `buildDepsOnly` finding already confirms the empty-sandbox reality.
- **Verify:** findings recorded; the **env-keyed** fail-closed rule and the
  pure-function test approach are pinned before Task 2.

## Task 1 — Precompression in `devtool csr-bundle`

- [ ] Add `brotli` + `flate2` deps to the **`tools/` workspace** (its own
      `Cargo.lock`; not the main workspace).
- [ ] `tools/devtool/src/csr_bundle.rs`: after writing `jaunder.js` /
      `jaunder.wasm`, also write `jaunder.js.br`, `jaunder.js.gz`,
      `jaunder.wasm.br`, `jaunder.wasm.gz` (brotli quality 11 for the wasm; gzip
      best). Only `.js`/`.wasm` (not `.d.ts`/snippets).
- [ ] Unit-test the compression helper (round-trips: decompress(compress(x))==x)
      if factored; otherwise a smoke assertion the four siblings are written.
- **Verify:** `cargo xtask build-csr` → `target/site/pkg/` contains the four
  compressed files, each smaller than its source; `devtool` tests pass
  (`--manifest-path tools/Cargo.toml`). `cargo xtask check` green.
- **Commit:**
  `feat(devtool): precompress CSR bundle (.br/.gz) in csr-bundle (#237)`.

## Task 2 — Server: `build.rs`, `Site` embed, precompressing handler, wiring

- [ ] **`server/build.rs`** (new): stage `$OUT_DIR/site/`: - `pkg/` ←
      `env JAUNDER_CSR_BUNDLE_DIR` else `<workspace>/target/site/pkg`
      (`CARGO_MANIFEST_DIR/../target/site/pkg`); copy the whole tree **except
      `.d.ts`**. - `public/` ← `env JAUNDER_PUBLIC_DIR` else
      `<workspace>/public`. - **Fail-closed rule** (env-keyed, from Task 0):
      `JAUNDER_CSR_BUNDLE_DIR` **set** but its `pkg/` missing/empty → `panic!`;
      env **unset** → `println!("cargo:warning=…")` + stage an empty
      `$OUT_DIR/site` so the crate still compiles (keeps the shared release
      `buildDepsOnly`, coverage, clippy, and bare `cargo build` green). **Not**
      keyed on `PROFILE`. - `cargo:rerun-if-changed` on both source dirs +
      `rerun-if-env-changed`.
- [ ] **`server/Cargo.toml`**: enable rust-embed `interpolate-folder-path`
      feature (keep `axum` feature).
- [ ] **Embed + handler** (new module, e.g. `server/src/site.rs`):
      `#[derive(RustEmbed)] #[folder = "$OUT_DIR/site"] struct Site;` and an
      axum handler `serve_site` that, for a request path: - looks up the file in
      `Site`; on miss → delegate to `spa_shell` (SPA boot). - parses
      `Accept-Encoding`; if `<path>.br` embedded & `br` accepted → serve it with
      `Content-Encoding: br`; else `<path>.gz` & `gzip` → `gzip`; else identity.
      Always `Vary: Accept-Encoding`. - `Content-Type` from the **logical** path
      via `mime_guess` (`.wasm` → `application/wasm`, `.js` →
      `text/javascript`). - `ETag` from `EmbeddedFile.metadata.sha256_hash()` of
      the **served representation** (so `.br` and identity differ); honor
      `If-None-Match` → `304` empty body.
- [ ] **Factor for embed-free tests (from review).** The coverage build stages
      an **empty `Site`** (env unset) and committed binary fixtures don't
      survive crane's `src` filter — so the handler's logic must be **pure
      functions over injected bytes + metadata**, not over a live `RustEmbed`:
      e.g. `choose_encoding(accept_encoding, has_br, has_gz) -> Encoding`,
      `etag_for(bytes) -> String`, `content_type_for(logical_path) -> Mime`,
      `not_modified(if_none_match, etag) -> bool`. The thin axum handler wires
      `Site::get()` to these. Coverage lands on the pure fns.
- [ ] **Unit tests** (carry AC3/AC4 — no e2e; over the pure fns): br/gz/identity
      selection + `Content-Encoding`/`Vary`; per-encoding `ETag` +
      `If-None-Match` → 304; `application/wasm` & `text/javascript` mime;
      unknown path → SPA-shell fallthrough. **No live embed / no committed
      fixture** (both are empty under the coverage sandbox).
- [ ] **Wire** in `server/src/lib.rs`: replace
      `app.fallback_service(ServeDir::new(&site_root).fallback(spa_shell…))`
      with the `serve_site` handler as the fallback service (keeping the
      projector registration ahead of it). Remove the now-dead
      `ServeDir`/`site_root` local.
- [ ] **`server/src/commands.rs`**: drop `.site_root(...)`/`.site_pkg_dir(...)`
      only if nothing else needs them (the `LeptosOptions` builder may still
      require `site_root` — keep the minimum; confirm via compile).
- [ ] **Committed `/pkg` regression** (Scope 6): add an e2e/integration
      assertion (real serve, since coverage's `Site` is empty) that
      `GET /pkg/jaunder.wasm` → `200` + `application/wasm` — the non-hydration
      proof the bundle serves. Extend an existing e2e spec (NOT
      `static-assets.spec.ts`, which only hits `/style/*`), or add a focused
      one.
- [ ] **Scope 3 (xtask) — deliberate no-op**: no xtask change is needed. Host
      release goes via Nix (`jaunderBin`); `build_csr` already accepts
      `--release` and `e2e_local:61` runs it before the server build. State this
      so Scope 3 isn't left dangling.
- **Verify:**
  1. `cargo xtask build-csr` then `cargo build -p jaunder` green; a bare
     `cargo build -p jaunder` (no bundle) still compiles (tolerant debug path).
  2. `cargo test -p server site` (handler unit tests) green.
  3. `cargo xtask e2e-local` passes (CSR app hydrates via the new handler; AC5,
     AC6) and `GET /pkg/jaunder.wasm` → 200 `application/wasm`.
  4. `cargo xtask check` green (fmt, clippy incl. no new `unwrap`/`expect`,
     coverage — the handler is unit-covered; `build.rs`/embed `#[folder]` under
     `// cov:ignore` as the CSS `StaticAssets` is).
- **Commit:**
  `fix(server): embed the CSR bundle + public assets, serve precompressed (#237)`.

## Task 3 — Nix: build-time bundle input for `jaunderBin`

- [ ] `flake.nix`: `jaunderBin` (`:359`) — add
      `JAUNDER_CSR_BUNDLE_DIR =     "${csrWasmBundle}"` and
      `JAUNDER_PUBLIC_DIR = "${./public}"` to its build env; add `csrWasmBundle`
      to its inputs so the bundle builds first.
- [ ] Trim the now-vestigial runtime wiring: the NixOS module's
      `ln -sfn ${site} target/site` (`:119-121`) and the `site` derivation's
      `pkg/` copy (`:464-468`) are no longer needed **for the binary** — remove
      the `pkg` reliance; keep only whatever else still consumes `site` (verify
      nothing does before deleting).
- **Verify:** `nix build .#<server/checks>` builds `jaunderBin` with the bundle
  embedded; the resulting `--release` binary serves `/pkg/*` + `/favicon.ico`
  with `target/site` absent (AC1). `cargo xtask validate --no-e2e` still clean
  (Nix eval unaffected on host).
- **Commit:**
  `build(nix): embed the CSR bundle into jaunderBin; drop the runtime site symlink (#237)`.

## Task 4 — Verification (no commit)

- [ ] **AC1** (self-contained release): build `--release` (host:
      `build-csr     --release` then `cargo build -p jaunder --release`, or the
      Nix binary), run it from an unrelated CWD with no `target/site`, `curl`
      `/pkg/jaunder.js` (`text/javascript`), `/pkg/jaunder.wasm`
      (`application/wasm`), `/favicon.ico` → all 200.
- [ ] **AC2** (fail-closed): one-shot `cargo build -p jaunder --release` with
      `JAUNDER_CSR_BUNDLE_DIR` = an empty dir → non-zero exit (build.rs panic).
      Documented here; not a standing gate.
- [ ] **AC3/AC4**: `curl -H 'Accept-Encoding: br'` / `gzip` / none against the
      release server → correct `Content-Encoding`/`Vary`/sizes; repeat with
      `If-None-Match` → 304. (Belt-and-suspenders over the Task-2 unit tests.)
- [ ] **AC7**: `cargo xtask validate --no-e2e` clean; e2e matrix (ship step /
      CI).
- **Verify:** all ACs observed; `xtask-done: … ok=true`.

---

## Coverage of spec ACs

| AC                                    | Task                           |
| ------------------------------------- | ------------------------------ |
| AC1 (self-contained --release)        | Task 3 (Nix) / Task 4          |
| AC2 (build fails when bundle absent)  | Task 2 (rule) + Task 4 (check) |
| AC3 (precompression negotiation)      | Task 1 + Task 2 (handler+test) |
| AC4 (conditional / per-encoding ETag) | Task 2 (handler + unit tests)  |
| AC5 (debug loop unbroken)             | Task 2 verify (e2e-local)      |
| AC6 (bundle integrity / hydrates)     | Task 2 verify (e2e)            |
| AC7 (validate clean; e2e pass)        | Task 4 (+ ship)                |

## Risk register (from spec + Task 0)

- **Fail-closed vs. the shared `buildDepsOnly`** — the single highest risk
  (review-confirmed): `cargoArtifacts = buildDepsOnly` builds at **release** and
  runs `build.rs` with no bundle env, so a `PROFILE`-keyed panic would break the
  entire Nix graph. **Resolved** by keying the panic on `JAUNDER_CSR_BUNDLE_DIR`
  being **set** (only `jaunderBin` sets it), never on `PROFILE`;
  env-on-`commonArgs` is rejected (eval cycle). Task 0 probes `buildDepsOnly`
  before Task 2.
- **Handler coverage under empty `Site`** — the coverage build's `Site` is empty
  and fixtures are `src`-filtered, so handler tests must be **pure-function**
  tests over injected bytes (Task 2), not over a live embed. Otherwise the
  coverage/CRAP gate fails.
- **rust-embed `$OUT_DIR`/debug path** — Task 0 gate; Shape-B fallback if it
  fails.
- **Custom handler correctness** — unit tests are the guard (AC3/AC4).
- **Binary size** — raw+br+gz kept per approval; ~6–7 MB added (accepted).
