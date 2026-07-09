# ADR-0058: A `host` crate for strictly-host-focused shared code

- Status: accepted
- Date: 2026-07-08
- Issue: [#227](https://github.com/jaunder-org/jaunder/issues/227)

## Context

The `common` crate holds code shared across the workspace and is deliberately
**target-agnostic**: it compiles to both the host (server) and wasm (web
frontend), and every `#[cfg]` in it today is `#[cfg(test)]` or `#[cfg(feature)]`
— there are **zero** host-only `#[cfg(not(target_arch = "wasm32"))]` carve-outs.

Issue #227 introduced shared code that is inherently **host-only and e2e-only**:
a helper reading `JAUNDER_CAPTURE_DIR` and touching `std::fs`/`std::env` to
resolve capture-file paths. It is needed by both `server` (which writes the
streams) and `test-support` (which resets/queries them), so it cannot live in
`server` (`test-support` must not depend on the heavy server binary crate, and
the dependency direction would be backwards). Putting it in `common` would force
the first host-only `#[cfg(not(wasm32))]` carve-out purely to keep
`std::fs`/`std::env` plumbing out of the wasm bundle — eroding `common`'s
uniform-dual-target invariant.

## Decision

Introduce a new **`host`** workspace crate: the home for
**strictly-host-focused** shared code. It is the host-side sibling of the
target-agnostic `common`. Because it never targets wasm, it may use
`std::fs`/`std::env` freely with no cfg gating, and `common` keeps its "zero
host-only carve-outs" invariant.

**Load-bearing invariant: `host` depends on no _workspace_ crate except
`common`.** `host` is the host _floor_ — code that needs a `storage` (or higher)
_workspace_ type (`PostStorage`, `AudienceError`, …) belongs in
`storage`/`server`, not here. It **may** depend on external _infrastructure_
crates the shared code genuinely needs (`anyhow`, `tracing`, `sqlx`, `chrono`,
`http`): the dividing line is that `host` knows _raw infrastructure types_ (e.g.
classifying a `sqlx::Error`) but never our domain/storage abstractions. Only
workspace-crate deps can cycle or recreate the grab-bag, so the rule that keeps
the graph acyclic and stops `host` drifting into an omnibus (the `AppState`
failure mode ADR-0016 addressed) is specifically _no workspace crate above it_.
The rule earns its keep the moment a second, non-leaf tenant lands and other
crates depend _on_ `host` rather than the reverse.

_(Clarified 2026-07-09 by #334, whose error-carrier tenant is the first to take
external infra deps — `sqlx`/`chrono`/`http` — while still depending on no
workspace crate but `common`.)_

The intended trio, by compilation target:

- **`common`** — shared code that compiles to _both_ host and wasm
  (target-agnostic).
- **`host`** — shared code that only makes sense on the host (server/CLI/e2e
  tooling).
- **`client`** _(future)_ — the symmetric peer for strictly-client
  (wasm/browser) code, to be created when such shared code first appears.

Its **first** tenant is the `JAUNDER_CAPTURE_DIR` capture-path helper (see the
capture-dir contract ADR) — but that is an example, not the archetype. The
charter is broader than e2e plumbing: **any** strictly-host-focused shared code
belongs here, including _production_ machinery pushed down out of `web` (e.g. a
server-side error carrier), not only e2e tooling.

## Consequences

- Host-only shared utilities have a clear home; future ones land in `host`
  rather than bloating `common` or forcing cfg gates.
- Initially `server` and `test-support` depend on `host` (a snapshot at #227,
  not a bound — later tenants bring more dependents); `common`'s dual-target
  invariant is preserved.
- No explicit coverage/CI wiring: the Nix coverage source filter auto-admits any
  new top-level crate and nextest/clippy run workspace-wide, so `host` is picked
  up simply by being a workspace member.
- `tools/devtool` is a separate `tools/` workspace and cannot link `host`; it
  never needs the capture filenames (it only passes `JAUNDER_CAPTURE_DIR`
  through to the `test-support` subprocess it spawns).
- Commits us to the `common`/`host`/`client` naming for the target-scoped
  shared-crate trio.
