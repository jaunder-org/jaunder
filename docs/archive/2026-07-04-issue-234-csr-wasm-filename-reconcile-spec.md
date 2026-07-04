# Spec: issue #234 — Host CSR bundle non-hydrating: reconcile wasm filename + bootstrap URL

- Issue: [#234](https://github.com/jaunder-org/jaunder/issues/234)
- Status: draft (awaiting approval)
- Date: 2026-07-04

## Summary

The host `cargo leptos end-to-end` loop serves a **non-hydrating** CSR build:
the server HTML shell renders but the wasm never loads, so `body[data-hydrated]`
is never set and hydration-dependent e2e tests time out (68/71). This blocks
#153's `cargo xtask e2e-local` driver (AC8).

Root cause (verified): two hand-written bootstraps call wasm-bindgen's `init()`
**with no argument**, so it falls back to its baked-in default `jaunder_bg.wasm`
— but cargo-leptos writes the wasm as `jaunder.wasm`. `GET /pkg/jaunder_bg.wasm`
404s → `init()` fails → no hydration. The fix is minimal and standalone: point
both bootstraps at the real filename, reconcile the Nix build to emit that same
name, and add a drift-guard so this cannot silently re-break.

This is a deliberately **narrow bug fix**. Two larger, separable concerns
surfaced during investigation — unifying the CSR bundle build and restoring the
single-binary goal — are filed as follow-up issues (see Non-goals) and are
**not** prerequisites for unblocking #153.

## Verified root cause

Reproduced with `cargo leptos build` (cargo-leptos 0.3.5) in the devShell:

- `target/site/pkg/` contains `jaunder.js` and **`jaunder.wasm`**, plus
  `jaunder_bg.wasm.d.ts` — but **no `jaunder_bg.wasm`**.
- `target/site/pkg/jaunder.js`'s `__wbg_init` defaults its wasm URL to
  `new URL('jaunder_bg.wasm', import.meta.url)` **when called with no
  argument**.
- **Two** hand-written bootstraps call `init()` with no argument:
  - `csr/index.html:16` — the SPA shell.
  - `server/src/projector/mod.rs:80` — the projector's cacheable anonymous HTML,
    served for the public routes it owns (`/`, `/tags/{tag}`, `/~{username}`,
    …).
- So both request `/pkg/jaunder_bg.wasm`, which the host build never wrote → 404
  → hydration never completes.

cargo-leptos writes `jaunder.wasm` **by design** — leptos's own
`HydrationScripts` bootstrap calls `init` _with_ the `/pkg/<name>.wasm` URL; our
hand-written bootstraps do not, so they hit wasm-bindgen's `jaunder_bg.wasm`
default. The **Nix build works today** because `csrWasmBundle`
(flake.nix:500-502) renames the file to `jaunder_bg.wasm`, which aligns with
that arg-less default. The host has no equivalent reconciliation.

The real defect is that the served wasm **filename** and the bootstrap's **URL**
are maintained in separate places and drifted. Note this drift is in
hand-written HTML/Rust, independent of which tool builds the wasm.

## The fix

Canonicalize on `jaunder.wasm` (cargo-leptos's natural output), so the host
needs no post-build reconciliation:

1. **Both bootstraps load the wasm explicitly** by its emitted name, following
   leptos's own convention:
   - `csr/index.html:16`: `init("/pkg/jaunder.wasm")`.
   - `server/src/projector/mod.rs:80`:
     `import init … init("/pkg/jaunder.wasm")`.
2. **Reconcile the Nix bundle to the same name.** In `csrWasmBundle`
   (flake.nix:500-502): rename the wasm to `jaunder.wasm` (not
   `jaunder_bg.wasm`) and update the internal reference in `jaunder.js`
   accordingly, so the Nix site serves `pkg/jaunder.wasm` and both bootstraps'
   explicit URL resolves.
3. **Update the wasm audit.** `xtask/src/audit_wasm.rs:133` hard-codes
   `pkg/jaunder_bg.wasm` (it reads `nix build .#site` and errors if the artifact
   is absent); change it and its tests (lines ~202/220/222/239) to
   `pkg/jaunder.wasm`.
4. **Drift-guard.** Add a test that fails if a bootstrap's wasm URL and the
   emitted wasm filename diverge — modeled on the existing prepaint-script drift
   guard (`web/src/render/mod.rs`, which `include_str!`s `csr/index.html` and
   asserts a byte-match). Concretely: assert both bootstraps reference
   `/pkg/jaunder.wasm`, and have `audit-wasm` (which already reads the built
   site) assert the built site contains `pkg/jaunder.wasm`. Together these catch
   drift on both the reference side and the Nix-emit side.

The host (`cargo leptos end-to-end`) needs **no** build-side change:
cargo-leptos already emits `jaunder.wasm`; the explicit `init` URL makes the
bootstrap load it directly, ignoring wasm-bindgen's unused internal default.

## Acceptance criteria

1. The CSR wasm bundle hydrates — `body[data-hydrated]` is set. **Verified via
   the Nix e2e** (`cargo xtask e2e sqlite chromium` → 71/71, covering both
   projector and SPA-fallback routes). A fully-green _host_
   `cargo leptos end-to-end` run additionally requires #239 (the host
   `index.html` seam — `csr/index.html` is never placed in `site_root` on the
   host), discovered during execution and out of scope here.
2. Nix e2e (CI) stays green; the Nix site serves `pkg/jaunder.wasm` and both
   bootstraps load it.
3. Both bootstraps (`csr/index.html`, `server/src/projector/mod.rs`) load the
   wasm explicitly by its emitted name; no arg-less `init()` remains.
4. A drift-guard test fails if a bootstrap's wasm URL and the emitted/served
   wasm filename diverge.
5. The wasm fix is **standalone** — it depends on no #153 changes. (Note: #153's
   host-loop-green also requires #239, the host `index.html` seam — a co-blocker
   discovered during execution.)
6. `cargo xtask audit-wasm` succeeds against the renamed artifact (its name +
   tests updated).

## Non-goals (filed as follow-up issues)

- **Unify the CSR bundle build onto cargo-leptos** — **#236**. Retire the
  crane + `wasm-bindgen` reimplementation in the flake, so host and Nix can't
  diverge. Pursued on its own merit as a consolidation, with the crane-seeded
  hybrid prototyped before it's spec'd. Not required for this bug — the
  drift-guard (above) is the safety net against filename drift, whether or not
  the build is unified.
- **Restore single-binary** — **#237**. Embed `pkg/*` + `index.html` into the
  server binary; today they are served from disk via `ServeDir`/`ServeFile`,
  regressing ADR-0003/ADR-0008 since #177/#180. Its own design (rust-embed
  release-embed / debug-disk, build-order, precompression).

## Risks & mitigations

- **Missed bootstrap / future new bootstrap** → the drift-guard test (AC4) fails
  loudly instead of shipping a silent 404.
- **Nix rename breaks other consumers of the old name** → `audit_wasm.rs` is the
  known one (AC6); the drift-guard and a full `validate` run catch others.
- **Transitional coupling within the PR**: the bootstrap change and the flake
  rename must land together (the index.html edit alone would break Nix, which
  still emits `jaunder_bg.wasm`). Both are in this issue → the result is
  standalone of #153.

## Relations

- Partially unblocks #153 AC8: fixes the wasm defect (Nix-verified 71/71);
  #153's host-loop-green additionally needs #239.
- Follow-ups filed: #236 (build unification); #237 (single-binary restoration);
  #239 (host `index.html` seam).
