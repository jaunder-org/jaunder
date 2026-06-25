# Spec — #41: gate xtask workspace clippy + fmt in check/validate

**Issue:** jaunder-org/jaunder#41 (milestone 1, "Verify-gate hardening")
**Date:** 2026-06-25
**Sibling:** #38 (wired xtask *unit tests* into the gate); this does the same for xtask *clippy + fmt*.

## Problem

`steps/static_checks::run` runs `cargo clippy` / `cargo fmt` for the app workspace
and (via `--manifest-path tools/Cargo.toml`) for the `tools/` workspace, but never
for `xtask/`. `xtask/Cargo.toml` declares its own `[workspace]`, so it is excluded
from the app/root clippy and fmt. Consequence: a clippy lint or formatting drift in
`xtask/` ships green. During #3 the xtask clippy had to be run by hand to catch a
dead-code and an `unnecessary_get_then_check` lint the gate missed.

## Goal

Add xtask clippy + fmt to the static-check suite, in a form that carries a
regression test for the acceptance criteria — not just a manual check.

## Design

### Refactor: separate step definition from execution

`xtask/src/steps/static_checks.rs` becomes a pure spec builder plus a thin executor:

```rust
pub struct StepSpec {
    pub name: &'static str,
    pub program: &'static str,
    pub args: Vec<&'static str>,
}

/// Pure: the ordered static-check steps for a given mode. No I/O.
pub fn specs(mode: Mode) -> Vec<StepSpec> { /* ... */ }

pub fn run(sh: &Shell, mode: Mode, result: &mut CommandResult) {
    for spec in specs(mode) {
        result.push(step(sh, spec.name, spec.program, &spec.args));
    }
}
```

`run`'s observable behavior is unchanged for every existing step: identical names,
programs, args, and order. Only the internal structure changes, plus two appended
steps.

### New steps (appended after `tools-clippy`)

- **xtask-fmt** — mirrors `tools-fmt`'s mode switch, without `--all`:
  - `Mode::Check`: `["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"]`
  - `Mode::Fix`:   `["fmt", "--manifest-path", "xtask/Cargo.toml"]`
- **xtask-clippy** — identical in both modes:
  - `["clippy", "--manifest-path", "xtask/Cargo.toml", "--all-targets", "--", "-D", "warnings"]`

**Why no `--all`:** `tools/` is a *virtual* workspace (no root package), so its
fmt/clippy need `--all` to reach the member crates. `xtask/Cargo.toml` is a
`[workspace]` *with* a root `[package]` named `xtask`; its only path dependency
(`coverage`) lives in the separate `tools/` workspace and is not an xtask workspace
member. So a bare `--manifest-path xtask/Cargo.toml` covers the xtask package, which
is the whole workspace. This matches the command bodies the issue specifies.

## Testing

In-file `#[cfg(test)] mod tests` in `static_checks.rs` (consistent with `nix.rs` and
`result.rs`, both of which keep in-file tests; xtask is coverage-exempt — see "Scope"
— so this does not interact with the coverage gate). Tests assert the *spec list*,
never shell behavior:

1. `specs(Mode::Check)` contains `xtask-fmt` with args
   `["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"]`.
2. `specs(Mode::Fix)` contains `xtask-fmt` with args
   `["fmt", "--manifest-path", "xtask/Cargo.toml"]`.
3. Both modes contain `xtask-clippy` with args
   `["clippy", "--manifest-path", "xtask/Cargo.toml", "--all-targets", "--", "-D", "warnings"]`.
4. Order-lock: the full ordered list of step *names* equals the expected sequence
   for each mode, so a future reorder or dropped step trips a test.

## Acceptance

From the issue, verified manually on top of the unit tests:

- A clippy warning in `xtask/` makes `cargo xtask check --no-test` go red.
- An unformatted `xtask/` file makes `cargo xtask validate` go red, and
  `cargo xtask check` auto-fixes it.

Manual verification procedure: inject a clippy lint into an xtask source file,
confirm `cargo xtask check --no-test` red; revert. Inject a formatting drift,
confirm `cargo xtask validate` red and `cargo xtask check` fixes it; revert.

## Scope

- **In:** the refactor above, the two new steps, and the spec-list unit tests.
- **Out:** coverage instrumentation for xtask (same boundary as #38); any change to
  the observable behavior of a non-xtask step; routing app clippy through Nix
  (that is #10).
