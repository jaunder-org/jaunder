# Spec: issue #239 — the server owns its SPA shell (embed `csr/index.html`)

- Issue: [#239](https://github.com/jaunder-org/jaunder/issues/239)
- Status: draft (awaiting approval)
- Date: 2026-07-04

## Summary

The host `cargo leptos end-to-end` loop leaves `target/site/index.html`
**absent**, so every SPA-fallback route serves an empty shell and times out on
`body[data-hydrated]` — co-blocking #153's host loop. The fix: **the server
serves its SPA shell from a compile-time constant** (`include_str!` of
`csr/index.html`) instead of reading `{site_root}/index.html` from disk —
exactly as the projector already renders its routes from embedded constants.
`csr/index.html` becomes the **single source** of the shell — embedded in the
server and read from source by the `audit-wasm` guard, copied to no build output
— so it no longer matters that cargo-leptos never writes `index.html` to
`site_root` on the host.

## Why not "let cargo-leptos place it" (spiked, rejected)

The config declares a bin+lib **SSR/hydrate** shape (`bin-package = jaunder`,
`lib-package = csr`), but since #180 we are **CSR-only** — the client is CSR and
the bin is a plain axum static+API server, not an SSR-leptos server. Because
cargo-leptos sees bin+lib it assumes the server renders the HTML, so it emits no
`index.html`. Verified by spike:

- `public/index.html` →
  `Assets source public contains path public/index.html reserved for Leptos`
  (cargo-leptos reserves the name).
- root `index.html` → build succeeds but `target/site/` still has only
  `{pkg,favicon.ico}` — silently ignored.

Making cargo-leptos _own_ `index.html` requires switching it to CSR mode
(lib-only), which stops it building and running our API server for e2e — a
substantial rework of the build+serve+e2e loop (the "align the config with what
we are" project; larger than #239, reshapes #236/#237). Out of scope here.
Instead we **stop involving cargo-leptos in the shell**: cargo-leptos builds the
wasm (its job); the server owns the shell (compile-time), where the projector
already gets its HTML.

## The fix

1. **Add the shell constant.** In `web/src/render/mod.rs` (alongside
   `PREPAINT_SCRIPT`; the module already `include_str!`s `csr/index.html` in
   tests):
   ```rust
   /// The CSR SPA shell, embedded at compile time. The host `cargo leptos` build
   /// never writes `index.html` to `site_root` (#239); the server owns it, the same
   /// way the projector renders its routes from constants.
   pub const SPA_SHELL: &str = include_str!("../../../csr/index.html");
   ```
2. **Serve it from the server** (`server/src/lib.rs:108-121`). Replace the disk
   read
   - `ServeFile` fallback:
   * Projector `Shell` ← `web::render::SPA_SHELL` (not `read_to_string`).
   * The global SPA fallback ← an axum handler returning
     `Html(web::render::SPA_SHELL)`, wired as `ServeDir`'s `.fallback`. The
     exact incantation (a bare handler fn does **not** coerce):
     `use axum::handler::HandlerWithoutStateExt;` then
     `ServeDir::new(&site_root).fallback(spa_shell.into_service())`. `pkg/*` and
     public assets still serve from `site_root` on disk; only the shell moves to
     embedded, and unmatched routes keep returning **200** with the shell (SPA
     semantics unchanged).
   * Drop the `std::fs::read_to_string`, the `index_html` path, and the
     `ServeFile` import.
3. **Make `csr/index.html` the single source — drop the flake copy.** Remove
   `flake.nix`'s `cp ${./csr/index.html} $out/index.html` (and fix the now-false
   comment at `flake.nix:456-457` that says the projector serves this disk
   file). The Nix site then ships only `pkg/*` + public assets; the shell is
   served from the embedded const on both host and Nix.
4. **Point `audit-wasm` at the source shell.** `xtask/src/audit_wasm.rs`
   currently reads `{site}/index.html` to extract the boot URLs; change it to
   read the repo's `csr/index.html` (the shell the server actually embeds), then
   check the emitted `{site}/pkg/*` for those artifacts. This keeps the #234
   shell↔bundle guard — made _more_ correct (it now audits the served shell,
   not a disk copy) — and decouples the audit from the dropped `cp`. Update its
   unit tests accordingly.

The served shell **body** is byte-identical to today's (both derive from
`csr/index.html`); the response **headers** change with the `ServeFile → Html`
swap (see AC5). Nix e2e asserts DOM, not shell headers, so behaviour is
unchanged; the host gains the shell it never had.

## Acceptance criteria

1. With a `site_root` that has **no `index.html`** on disk, the server serves
   the SPA shell (containing the boot script) on a SPA-fallback route — a
   server-level test asserts this (the #239 regression guard).
2. `pkg/*` and public assets (e.g. `favicon.ico`) still serve from `site_root`
   on disk (unchanged); only the shell is embedded.
3. The projector `Shell` is the embedded shell (not a disk read).
4. The SPA-fallback response is **200** with `Content-Type: text/html`, and its
   **body** is byte-identical to the prior disk-served shell. (Header delta from
   `ServeFile → Html` is expected: no `Last-Modified`/`ETag`/`Accept-Ranges`. No
   e2e asserts caching headers on shell routes — confirm during e2e.)
5. `csr/index.html` is the **only** on-disk shell provenance: the flake no
   longer copies it into the site (`nix build .#site` output has no
   `index.html`), and the `flake.nix:456-457` comment is corrected.
6. The #234 shell↔bundle guard is preserved: `audit-wasm` reads the repo's
   `csr/index.html` (not `{site}/index.html`), extracts the boot URL, and
   confirms `{site}/pkg/*` emitted it; its unit tests pass. No arg-less `init()`
   regression.
7. Nix e2e stays green (`cargo xtask validate`).
8. Host loop: `cargo leptos end-to-end` no longer serves an empty shell on
   SPA-fallback routes. (Full host-green also needs playwright provisioning
   locally; the server-level test + Nix e2e cover the behaviour hermetically.)

## Non-goals

- **CSR-mode realignment** — reconfiguring cargo-leptos to CSR (lib-only) and
  running the API server separately. The real "stop fighting the tool" project;
  reshapes #236/#237. Not needed to unblock #153.
- **Embedding the wasm bundle** (`pkg/*`) — #237. This spec embeds only the tiny
  shell; the multi-MB bundle stays disk-served (`ServeDir`).

## Relations

- **Unblocks #153 AC8** (host-loop-green): removes the empty-shell failure on
  SPA-fallback routes.
- Advances **ADR-0003/ADR-0008** (single-binary): the SPA shell is now embedded,
  like the CSS already is; #237's remaining scope shrinks to the wasm bundle.
- Found during #234; sibling to #237 (single-binary), #236 (build unification).
