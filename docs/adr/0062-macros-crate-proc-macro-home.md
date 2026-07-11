# ADR-0062: A `macros` crate as the workspace's proc-macro home

- Status: proposed
- Date: 2026-07-11
- Issue: [#370](https://github.com/jaunder-org/jaunder/issues/370)

## Context

Issue #370 generalizes the coverage gate's `#[component]` exemption to
client-only reactive helpers that aren't components
(`web::reactive::Invalidator`'s `resource`, `action`, `patched`, `sticky`) by
introducing a `#[client_only]` attribute the coverage framework recognizes
syntactically. That attribute has to be **real** — a custom attribute on a
method cannot be an inert marker on stable Rust; it must be backed by a
proc-macro.

A proc-macro crate (`[lib] proc-macro = true`) has two properties that decide
where it can live:

- It can export **only** proc-macros — no runtime types or functions. So the
  attribute cannot share a crate with `web` (or any normal library).
- It is compiled for the **compiler host** and loaded by rustc at build time. It
  is not target-scoped runtime code, so it is not a fit for the target-scoped
  runtime crates — `common` (dual-target), `host` (host runtime), or a future
  `client` (wasm runtime) — of ADR-0058's trio.

So the attribute needs a new, dedicated crate.

## Decision

Introduce a **`macros`** workspace crate: the home for the workspace's
proc-macros. It is a target-agnostic, host-compiled **build-time** crate —
deliberately named tersely and unprefixed (matching `common`/`host`/`web`), and
deliberately **not** narrowed to `web-`/`client-`/`coverage-`, so any future
workspace proc-macro (for any purpose) belongs here.

**`macros` is orthogonal to the `common`/`host`/`client` runtime trio
(ADR-0058), not a fourth member of it.** The trio partitions _runtime_ shared
code by compilation target; `macros` is _build-time_ tooling with no runtime
footprint. The distinction is load-bearing: a proc-macro must never be mistaken
for a target-scoped runtime home, and the trio's target-partition reasoning does
not apply to it.

Its **first tenant** is `#[client_only]` (#370): an **identity** attribute macro
(item in, item out, unchanged). Its only effect is to be a syntactic marker
`xtask/src/coverage/exempt.rs` recognizes and exempts — a macro-backed peer of
the `cov:ignore` / `crap:allow` comment markers, needed as a real crate only
because a _custom attribute_ (unlike a comment) must be macro-backed.

## Consequences

- Future workspace proc-macros land in `macros` rather than spawning one crate
  per macro or bloating a runtime crate.
- No explicit coverage/CI wiring: the Nix coverage source filter auto-admits any
  new top-level crate, and nextest/clippy run workspace-wide, so `macros` is
  picked up simply by being a workspace member (as ADR-0058 noted for `host`).
- No new coverage surface: a proc-macro crate is a separate `.so` loaded by
  rustc at build time and is not linked into the instrumented test binaries, so
  its code contributes no gate-measured lines.
- Commits us to `macros` as the proc-macro home; a future `client` crate
  (ADR-0058) that wants `#[client_only]` depends on `macros`, not the reverse —
  the two stay cleanly separated (runtime vs build-time).
