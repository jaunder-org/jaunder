# Spec — #237: embed the CSR bundle (+ public assets) into the server binary

- Issue: [#237](https://github.com/jaunder-org/jaunder/issues/237) (bug; split
  from #234)
- Milestone: Off concurrent SSR (web re-architecture v1)
- Governing ADRs: **ADR-0003** (asset management for single-binary
  distribution), **ADR-0008** (single-binary deployment model)
- Date: 2026-07-16

## Problem

The single-binary goal (ADR-0003/0008: "assets are bundled into a single
binary", served "without requiring external files on the target filesystem") is
regressed for the CSR client bundle. Today the server disk-serves the bundle and
public assets:

```rust
// server/src/lib.rs:127
app.fallback_service(ServeDir::new(&site_root).fallback(spa_shell.into_service()))
// site_root = "target/site"  (server/src/commands.rs:528)
```

`ServeDir` reads `target/site/pkg/jaunder.js`, `target/site/pkg/jaunder.wasm`,
and public assets (`favicon.ico`) **from disk at runtime**. A released binary
run outside the build tree cannot serve its own client — the ADR's "external
files".

**Already resolved** (do not re-do): `index.html`/the SPA shell is embedded as
the `web::render::SPA_SHELL` constant (#239, `web/src/render/mod.rs:51`,
`lib.rs:114-116`); CSS/themes are embedded via `StaticAssets`
(`server/src/assets.rs`, served at `/style`). Only the **JS/wasm bundle and
public assets** remain on disk. That is this issue's whole remaining scope.

## Decision

Embed the runtime **site tree** (`pkg/jaunder.js`, `pkg/jaunder.wasm`, plus
precompressed variants, plus `public/` — currently `favicon.ico`) into the
server binary via `rust-embed`, and serve it through a small **precompression-
aware embed handler**, replacing the disk `ServeDir` fallback. **Release =
embedded; debug = disk-read** (rust-embed's default, no `debug-embed` feature),
preserving a fast inner loop. Maximalist per the design interview: `public/` is
embedded (also closing the host-side #291 gap) and the bundle is precompressed.

### 1. Build-order seam — `server/build.rs` stages into `OUT_DIR` (interview: Shape A)

The server crate gains a `build.rs` that assembles the site tree into
`$OUT_DIR/site/` at server-compile time:

- **pkg/** ← copied from `env JAUNDER_CSR_BUNDLE_DIR` if set, else the host
  default `<workspace>/target/site/pkg` (host defaults resolve relative to the
  workspace root — `CARGO_MANIFEST_DIR/..`, since `CARGO_MANIFEST_DIR` is
  `server/`). In a **release** build the source must exist or `build.rs`
  **panics** (fail-closed — a release binary must never ship without its
  client). `cargo:rerun-if-changed` on the source dir + env. Copy the **whole
  `pkg/` tree** (so wasm-bindgen `snippets/`, if the `csr` crate uses JS
  snippets, are served); exclude `.d.ts` (dev-only type defs — dead weight).
  Only `.js`/`.wasm` get precompressed siblings; snippets are served identity.
- **public/** ← copied from `env JAUNDER_PUBLIC_DIR` (Nix sets it to
  `${./public}`; host default `<workspace>/public`), into `$OUT_DIR/site/` root.
  `public/` is **not** admitted by the Nix crane `src` filter
  (`flake.nix:281-298` — only `.sql`/`.css`/`csr/index.html`/`scripts/*`/Cargo
  sources), so it must arrive via this env var, **not** the crate source.
- A single `#[derive(RustEmbed)] #[folder = "$OUT_DIR/site"] struct Site;`
  embeds it. `$OUT_DIR` expansion requires rust-embed's
  **`interpolate-folder-path`** feature (off by default — add it). rust-embed
  default gives **debug = read `$OUT_DIR/site` from disk at runtime** (the
  staged copy; `$OUT_DIR` is an absolute path baked in at compile, so
  CWD-independent), **release = embed at compile**. The handler is used in both
  profiles, but note the debug read is of the staged `$OUT_DIR` copy —
  **`e2e-local` (debug) does not prove release embedding** (see AC1/AC5).

Build-order wiring:

- **Host:** `cargo xtask build-csr` already writes `target/site/pkg/*` before
  the server is built (`xtask/src/steps/build_csr.rs`; `e2e_local.rs:61` runs it
  before `cargo build -p jaunder`). `build.rs`'s default source is that path.
  Any release packaging path must run `build-csr --release` first (plan
  enumerates the sites).
- **Nix:** `jaunderBin` (`flake.nix:359`) gains a build-time dependency: set
  `JAUNDER_CSR_BUNDLE_DIR = ${csrWasmBundle}` **and
  `JAUNDER_PUBLIC_DIR = ${./public}`** in its build environment (adding
  `csrWasmBundle` as an input). Both the bundle and `public/` are supplied via
  env (neither is in the crane `src` sandbox). The `site` derivation's `pkg/`
  copy and the runtime `ln -sfn ${site} target/site` (`flake.nix:119-121`,
  `464-468`) become **vestigial for the binary** and are removed/trimmed (the
  binary no longer reads `target/site`).

### 2. Serving — a precompressing embed handler (NOT `axum-embed`)

`axum-embed 0.1.0`'s `ServeEmbed` does **no** `Accept-Encoding` negotiation (its
API is fallback-file/fallback-behavior/index-file only; verified against its
docs). So precompression requires a **custom handler** over the `Site` embed:

- Route: a fallback service (or `/pkg/{*path}` + a `/favicon.ico` route) that,
  for a requested path, looks the file up in `Site`.
- **Content negotiation:** parse `Accept-Encoding`; prefer `br` if `<path>.br`
  is embedded, else `gzip` if `<path>.gz`, else identity (the raw file). Set
  `Content-Encoding` accordingly and always `Vary: Accept-Encoding`.
- **Conditional requests:** ETag from rust-embed's file hash
  (`EmbeddedFile.metadata.sha256_hash()`); honor `If-None-Match` → `304`. ETag
  is per **representation** (encoding-specific) to stay HTTP-correct.
- **Content-Type:** from the _logical_ path via `mime_guess` (`.js` →
  `text/javascript`, `.wasm` → `application/wasm`), not the `.br`/`.gz` suffix.
- **Fallback:** a path with no embedded file falls through to the existing
  `spa_shell` handler (SPA boot), exactly as `ServeDir(...).fallback(spa_shell)`
  does today. CSS stays on its existing `/style` `ServeEmbed` mount (small,
  uncompressed — unchanged).

### 3. Precompression generation — in `devtool csr-bundle` (shared host + Nix)

`devtool csr-bundle` (`tools/devtool/src/csr_bundle.rs`), which both the host
`build-csr` and the Nix `csrWasmBundle` derivation call, emits — alongside
`jaunder.js` / `jaunder.wasm` — `jaunder.js.br`, `jaunder.js.gz`,
`jaunder.wasm.br`, `jaunder.wasm.gz` (brotli + gzip). Brotli via a `brotli`
crate (new dep); gzip via `flate2` (already a workspace dep). So both the host
and Nix bundles carry the compressed siblings, and the embed picks them up
uniformly.

## Scope

1. **`tools/devtool`** — `csr_bundle.rs`: after writing `jaunder.{js,wasm}`,
   generate `.br` + `.gz` for each; add `brotli` + `flate2` deps **to the
   `tools/` workspace** (its own `Cargo.lock`, `flake.nix:392-403` — not the
   main workspace). (Shared by host + Nix.)
2. **`server`** — new `build.rs` (stage `$OUT_DIR/site`); a bundle-embed module
   (`#[derive(RustEmbed)] Site` + the precompressing handler with negotiation /
   conditional / mime); wire the handler as the fallback in `lib.rs`, removing
   the `ServeDir::new(&site_root)` line; `brotli`/`flate2` not needed
   server-side (serving precompressed bytes verbatim). Add `rust-embed` `Site`
   next to `StaticAssets`, and enable rust-embed's **`interpolate-folder-path`**
   feature (for `$OUT_DIR` expansion) in `server/Cargo.toml`.
3. **`xtask`** — ensure the release build path stages the bundle before building
   the server (`build-csr --release` precedes the server build wherever a
   release binary is produced).
4. **`flake.nix`** — `jaunderBin` gets
   `JAUNDER_CSR_BUNDLE_DIR = ${csrWasmBundle}`
   - input; trim the now-vestigial `site`→`target/site` runtime symlink for the
     binary (keep whatever the NixOS module still needs, if anything).
5. **`server/src/commands.rs`** — `site_root`/`site_pkg_dir` may become unused
   for serving; keep only if still needed (e.g. cargo-leptos options builder).
6. **tests** — unit tests for the handler (negotiation, per-encoding 304, mime,
   fallthrough) — these carry AC3/AC4, which have no e2e. The **`posts.spec.ts`
   hydration** flow is the only proof the served wasm still boots
   (`static-assets.spec.ts` only checks `/style/*.css`, **not** `/pkg/*`); add a
   small e2e/integration assertion that `GET /pkg/jaunder.wasm` returns `200`
   with `application/wasm`. Self-containment (AC1) is a dedicated
   **`--release`** check with `target/site` absent — not part of the debug
   `e2e-local` loop.

**Out of scope / follow-ups:** any change to what the bundle _contains_; the
`public/`-sync issue #291 is closed _incidentally_ by embedding `public/` (note
it there, don't expand); wasm-opt size reduction.

## Acceptance criteria (observable)

- **AC1 (self-contained release):** a **`--release`** binary (embedding is
  release-only; a debug binary reads the staged `$OUT_DIR` copy and proves
  nothing here), run with **no `target/site` on disk** and from an unrelated
  CWD, serves `GET /pkg/jaunder.js` (`content-type: text/javascript`),
  `GET /pkg/jaunder.wasm` (`application/wasm`), and `GET /favicon.ico`
  (`image/x-icon` / `image/vnd.microsoft.icon`) with `200` and correct bytes.
  (Today this 404s.) This is the ADR-0003/0008 fix.
- **AC2 (build-order enforced):** a build that **declares it needs the bundle**
  (`JAUNDER_CSR_BUNDLE_DIR` set) but finds it empty/absent **fails at build
  time** (build.rs panic), not silently shipping an empty client. The trigger is
  **env-keyed, not `PROFILE`-keyed** — because crane's shared release
  `buildDepsOnly` runs `build.rs` with no bundle env, and a `PROFILE`-based
  panic would break the whole Nix graph. So: env **set** + empty → panic (every
  real release path — `jaunderBin` — sets it); env **unset** → tolerant (a
  developer `cargo build`/deps-only/coverage build compiles, serving 404s until
  a bundle exists). Verified **once, deliberately** (a one-shot build with
  `JAUNDER_CSR_BUNDLE_DIR` = an empty dir → non-zero exit). `jaunderBin` (Nix)
  and the host `build-csr`-then-build path produce a populated binary.
- **AC3 (precompression):** `GET /pkg/jaunder.wasm` with `Accept-Encoding: br`
  returns `Content-Encoding: br`, `Vary: Accept-Encoding`, and a body materially
  smaller than identity; `Accept-Encoding: gzip` returns
  `Content-Encoding: gzip`; no/`identity` returns the raw wasm. Content-Type is
  `application/wasm` in all three.
- **AC4 (conditional):** a second request with `If-None-Match: <etag>` returns
  `304` with an empty body; the ETag differs per encoding.
- **AC5 (debug loop unbroken):** a debug build serves the same paths from the
  staged `$OUT_DIR` copy; `cargo xtask e2e-local` still passes (the CSR app
  hydrates). Note this exercises the _handler_ but **not** release embedding —
  AC1 is the embedding proof.
- **AC6 (bundle integrity):** the served wasm/js are byte-identical to what
  `devtool csr-bundle` produced (the app hydrates in e2e — the #234-class
  regression does not recur).
- **AC7:** `cargo xtask validate --no-e2e` clean; the e2e matrix passes.

## Risks / open validations

- **rust-embed `$OUT_DIR` interpolation + debug path resolution.** The design
  assumes `#[folder = "$OUT_DIR/site"]` interpolates and that debug mode reads
  the interpolated absolute path at runtime (CWD-independent). Must be validated
  early in implementation; fallback is a `cfg`/feature-gated embed with an
  explicit disk `ServeDir` debug path (interview Shape B) if rust-embed's debug
  resolution doesn't cooperate.
- **Custom handler correctness.** Hand-rolled content-negotiation + conditional
  requests are a classic bug nest (per-encoding ETag, `Vary`, 304 empty body,
  mime from logical path). Mitigated by unit tests (AC3/AC4) + the e2e hydration
  proof. This is the complexity the axum-embed limitation forces.
- **Binary size (open decision at spec approval).** Embedding raw + `.br` +
  `.gz` of a ~4.3 MB wasm adds ~6–7 MB to the binary (identity kept for
  no-`Accept-Encoding` clients). The current spec keeps all three per the
  "maximalist" choice. **Cheaper alternative to weigh:** `raw + .br only` drops
  ~1.7 MB — defensible because ADR-0008 mandates a production reverse proxy (can
  gzip identity on the fly) and brotli has ~97% browser support, making the
  embedded `.gz` the lowest-value copy. Flagged for the user to confirm keep-gz
  vs. drop-gz.
- **Nix derivation edge.** `jaunderBin` gaining a bundle input lengthens its
  critical path (bundle must build before the binary); the previously-parallel
  build becomes ordered. Expected and correct.
- **cargo-leptos `LeptosOptions` coupling.** `site_root`/`site_pkg_dir` in
  `commands.rs` feed the options builder; confirm nothing else depends on them
  once serving no longer does.
