# Spec — #304 web: inline the vestigial `read_signal!` macro

**Issue:** [#304](https://github.com/jaunder-org/jaunder/issues/304) — inline
the vestigial `read_signal!` macro (a pure `.get()` pass-through after #300).

## Context

`web/src/pages/signal_read.rs` defines a one-arm macro:

```rust
macro_rules! read_signal { ($signal:expr) => {{ $signal.get() }}; }
```

Since #300 (`pages` compiles wasm-only), its former `.get_untracked()` server
arm is gone — it is now an unconditional `$signal.get()`, i.e. a no-op wrapper.
It also lives in `pages/`, which #330 will delete; inlining it removes one of
the three remaining `pages/` files (alongside `ui.rs`/#312 and `mod.rs`) and is
a prerequisite of #330 (now recorded as a blocked-by).

## Scope

Inline the macro at every call site and delete it. Purely mechanical —
`read_signal!(x)` is **exactly** `x.get()`, so there is no behavior change.

- **`web/src/timeline/component.rs`**: drop
  `use crate::pages::signal_read::read_signal;` (line 19); rewrite the three
  sites (104–106): `read_signal!(state.rows)` → `state.rows.get()`,
  `read_signal!(state.has_more)` → `state.has_more.get()`,
  `read_signal!(state.status).is_in_flight()` →
  `state.status.get().is_in_flight()`.
- **`web/src/home/component.rs`**: drop
  `use crate::pages::signal_read::read_signal;` (line 9); rewrite site 61:
  `read_signal!(state.status).into_failure()` →
  `state.status.get().into_failure()`.
- **Delete `web/src/pages/signal_read.rs`.**
- **`web/src/pages/mod.rs`**: remove `pub(crate) mod signal_read;`.

## Acceptance criteria

- **AC1** `rg 'read_signal' web/src` yields nothing — the macro, its module, and
  all call sites and imports are gone.
- **AC2** The four former call sites read their signal directly via `.get()`,
  semantics unchanged (the four exact rewrites above).
- **AC3** `web/src/pages/signal_read.rs` does not exist; `web/src/pages/mod.rs`
  no longer declares `mod signal_read;`.
- **AC4** No behavior change: `cargo xtask validate` green including the e2e
  matrix (timeline + home rendering, which exercise these reads, unaffected).

## Out of scope

- The rest of the `pages/` dissolution — `App`/Router/`pages/mod.rs` and
  `pages/ui.rs` belong to #330 and #312 respectively.

## Decisions / ADRs

No new ADR. Deleting a no-op macro wrapper.
