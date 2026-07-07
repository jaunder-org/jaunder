# ADR-0055: web host/wasm boundary is module-level, not line-level

- Status: accepted
- Date: 2026-07-06
- Issue: [#300](https://github.com/jaunder-org/jaunder/issues/300)

## Context

The `web` crate compiles for two targets — the host (x86_64) and `wasm32`. This
dual-target compile is deliberate and its host build is **load-bearing**, not
vestigial:

- The `server` binary's ADR-0040 projector uses `web::render` (+ `SPA_SHELL`);
  render coincidence requires the _same_ pure render fn on both sides.
- `#[server]` fn definitions compile for both targets by construction — the
  macro generates the client fetch stub from the same definition.
- `cargo llvm-cov` cannot instrument `wasm32`, so the coverage gate compiles the
  client feature on the host; host clippy and 30+ host unit tests of pure page
  logic ride on that same host build.
- `web/Cargo.toml` target-gates the browser-API deps (`js-sys`, `wasm-bindgen`),
  so ungated browser-API code is a host **compile error**, not a style choice —
  exactly the pattern the wasm-bindgen guide documents.

The smell was never the dual-target compile itself. It was that the boundary was
enforced **line by line** inside host-compiling Leptos `#[component]` UI modules
in `web/src/pages/`: ~20 `#[cfg(not(target_arch = "wasm32"))]` stub arms and
`#[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]` shims, each
dragging `cov:ignore` acceptance marks — and, worse, it invited **fake-value
host stubs** (e.g. `local_datetime_to_utc_rfc3339` returning
`Some(trimmed.to_string())` off-wasm), a divergent host substitute that a test
can assert while verifying a branch that never ships.

A structural fact makes the cleaner boundary safe: the projector **never mounts
the Leptos `App`** — it rebuilds public markup from `web::render`. So `pages`
components never actually SSR; the only real consumers of `web::pages` are the
`wasm32` client bundle and (formerly) the coverage host-compile. The host build
outside `pages/` references only `web::{render, posts, auth, tags, media}`.

## Decision

The `web` host/wasm boundary is drawn at the **module level**, once, not per
line:

1. **`pages/` (the Leptos UI) compiles wasm-only** —
   `#[cfg(target_arch = "wasm32")]` on `pub mod pages` (and the
   `pub use pages::App` re-export) in `web/src/lib.rs`. This deletes every
   per-line `target_arch` gate, `cfg_attr` shim, and fake-value host stub inside
   `pages/` in one move.
2. **Pure logic lives in host-compiled homes** — helpers that must stay
   host-tested and coverage-measured are relocated _out_ of `pages/` first:
   tag-slug validation to `web::tags`; display formatters (`format_bytes`,
   `avatar_parts`, `format_post_time`) to `web::render`; the auth marker
   encode/decode already lives in `web::auth::marker`. Relocation precedes
   gating so no pure line silently loses its test/coverage obligation.
3. **`#[server]` fn definitions and `web::render` stay host-compiled** —
   untouched. The dual-target `#[server]` surface is managed by the macro +
   `feature = "server"` gates, not manual `target_arch` gates.
4. **Wasm-only code is still linted** — a wasm-target clippy pass
   (`cargo clippy -p web --features csr --target wasm32-unknown-unknown`) is
   added to the gate (`cargo xtask check`/`validate` + the Nix check), since
   host clippy no longer sees `pages/`.

**No fake-value host stub may remain in `web`.** Code that cannot run on the
host is gated wasm-only; it is never given a divergent host substitute.

The larger **crate split** (a shared pure crate + a server-fn API crate + a
wasm-only client crate) is **deferred** to
[#303](https://github.com/jaunder-org/jaunder/issues/303). This module-level
boundary pre-partitions `web`'s modules into exactly those three groups,
de-risking the split, but the split's build-wiring cost (leptos feature
unification, relocating the `#[server]` macro surface + shared types,
re-pointing the projector / `csr` entry / cargo-leptos / Nix / xtask) is a
separate, larger step.

## Consequences

- The ~20 line-level gates, the `unused_variables` shims, and the fake-value
  host stubs are gone; `pages/` carries no
  `#[cfg(not(target_arch = "wasm32"))]`.
- The coverage-measured line set shrinks **only** for genuinely wasm-only UI
  glue (Leptos reactive components + browser-API calls), whose verification
  story is the e2e matrix — already the gate's official position for
  `#[component]` bodies (CONTRIBUTING's structural exemption, ADR-0050).
  Relocated pure logic stays measured; a per-cycle measured-line audit records
  which lines leave the set and why each is glue.
- The gate gains a wasm-target clippy pass. Note this does **not** moot the
  target-independent `must_use_candidate` / `too_many_arguments` allows — those
  remain their own concern (#94 / #299).
- Commits the project to relocating pure logic to a host-compiled home _before_
  gating a UI module wasm-only, and to never substituting a fake host value for
  wasm-only logic. Ties into ADR-0040 (render coincidence), ADR-0044 (the auth
  marker), ADR-0050 (stateless coverage gate / `#[component]` exemption), and
  ADR-0051/#268 (post-CSR module/crate boundaries). The crate split follow-on is
  #303.
