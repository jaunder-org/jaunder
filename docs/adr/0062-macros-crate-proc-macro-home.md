# ADR-0062: A `macros` crate as the workspace's proc-macro home

- Status: accepted
- Date: 2026-07-11
- Issue: [#370](https://github.com/jaunder-org/jaunder/issues/370)
- Amended: 2026-07-15
  ([#412](https://github.com/jaunder-org/jaunder/issues/412)) — corrected the
  "No new coverage surface" consequence (the `macros` crate **is**
  gate-measured) and documented the covering technique for macro authors (see
  Consequences)

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
- **Covered by the coverage gate** (correcting the original bullet here — #412).
  True: the compiled proc-macro is a build-time `.so`, loaded by rustc and never
  linked into a dependent's _runtime_ binary, so it adds no runtime footprint.
  But it does **not** follow that `macros` contributes no gate-measured lines —
  it does. `macros` is a workspace member, so the Nix coverage source filter
  auto-admits it (per the bullet above) and its own instrumented `#[cfg(test)]`
  unit-test binary measures `macros/src/*.rs` like any other crate; #403 hit
  exactly this (uncovered derive error paths failed `cargo xtask check`).
  - _Covering technique for macro authors:_ the codegen error branches execute
    when the derive fn is called, so drive the `compile_error!` / `?`-error
    paths from in-crate `#[cfg(test)]` unit tests that feed
    `syn::parse_quote!`-built `DeriveInput` fixtures to the fn and assert on its
    token output (see the `macros/src/lib.rs` tests). llvm-cov leaves the
    closing brace of a `?`-fall-through block unmarked as a gap region even when
    both arms are exercised — annotate that single brace `// cov:ignore`
    (precedent `macros/src/str_newtype.rs:304`, mirroring
    `storage/src/backup.rs:515`).
- Commits us to `macros` as the proc-macro home; a future `client` crate
  (ADR-0058) that wants `#[client_only]` depends on `macros`, not the reverse —
  the two stay cleanly separated (runtime vs build-time).
