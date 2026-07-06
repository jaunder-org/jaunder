# Spec — Issue #94: Audit all lint suppressions against the fix-don't-silence policy

**Issue:** jaunder-org/jaunder#94 · **Milestone:** Code quality improvement ·
**Type:** Task **Branch:** `worktree-issue-94-clippy-suppressions`

## Goal

Drive the repository to **zero in-source `#[allow(...)]`**, honoring
`CONTRIBUTING.md`: lints are fixed by changing the code. A suppression survives
**only** as an `#[expect(...)]` carrying a specific inline justification, and
only for the small user-approved keeper set below (genuinely unfixable or
framework/test-only sites). `clippy::pedantic = warn` and
`unwrap_used`/`expect_used = deny` stay on and are not weakened.

## Baseline (re-surveyed 2026-07-05, multiline-aware, excludes `target/`)

Survey (opener-only; authoritative for the _0 allows_ check):
`rg -n -e '#!?\[allow\(' -e '#!?\[expect\(' --glob '*.rs' --glob '!target/**'` →
**192** `#[allow]` sites (zero `#[expect]`) across **73** files.

Because ~27 integration-test files carry a single multi-line `#![allow(...)]`
block, the per-_lint_ footprint is larger than the site count. True
lint-occurrence counts (parsed across line breaks):

| clippy lint                     | occ. | clippy lint                 | occ. |
| ------------------------------- | ---: | --------------------------- | ---: |
| `must_use_candidate`            |   48 | `too_many_arguments`        |    9 |
| `too_many_lines`                |   45 | `needless_pass_by_value`    |    9 |
| `expect_used`                   |   40 | `cast_possible_truncation`  |    6 |
| `unwrap_used`                   |   34 | `cast_precision_loss`       |    5 |
| `similar_names`                 |   27 | `cast_sign_loss`            |    3 |
| `single_component_path_imports` |   27 | `format_collect`            |    1 |
| `items_after_statements`        |   26 | `duration_suboptimal_units` |    1 |
| `unused_async`                  |   26 | `struct_field_names`        |    1 |
|                                 |      | `disallowed_methods`        |    1 |

rustc: `unused_macros` 24, `unused_imports` 2, `dead_code` 1,
`unused_variables` 1.

Counts are re-verified during execution; the survey command is the source of
truth.

## Approved policy (design interview + user directives)

- **P1 — pedantic stays on.** No pedantic flag is disabled without a specific,
  well-reasoned justification; default disposition is _fix the code_.
- **P2 — no production `unwrap`/`expect`.** Any `unwrap`/`expect` on a runtime
  (non-test-gated) path is unacceptable and is removed by fixing the code.
  Non-negotiable.
- **P3 — scope = both clippy and rustc.**
- **P4 — one PR, commits grouped by lint category;** refactors get their own
  tested commits.
- **P5 — keeper bar.** For genuinely _unfixable_ or _framework/test-only_ sites,
  a justified `#[expect(...)]` + inline rationale is acceptable. Everything
  _fixable_ — including all test-code style lints — is fixed, not kept.

## The pre-existing config baseline (do not weaken)

A workspace `clippy.toml` **already exists** (added in #124) and already
provides:

```toml
allow-unwrap-in-tests = true      # unwrap/expect permitted in #[cfg(test)] + test-target crates
allow-expect-in-tests = true
disallowed-methods = [ { path = "leptos::prelude::Resource::new", reason = "…#124…" } ]
```

Consequence: the `unwrap_used`/`expect_used` allows scattered in `#[cfg(test)]`
code and in `server/tests/**` are **already redundant** — clippy would not fire
there even without them. Removing those in-source allows is a pure deletion.
This config is kept as-is (not extended, except D-config-1 below). Note its
`allow-*-in-tests` recognizes `#[cfg(test)]`/`#[test]`/ test-target crates —
**not** arbitrary feature gates (`test-support`, `test-utils`, `cheap-kdf`);
feature-gated `src/` code is handled explicitly (K2, F-misc) rather than assumed
covered.

## Config decisions

### D-config-1 — Disable `must_use_candidate` for the `web` crate only _(approved)_

46 of 48 `must_use_candidate` sites are Leptos view fns (`-> impl IntoView`) the
framework consumes; `#[must_use]` guards nothing there. Add to `web/Cargo.toml`:

```toml
[lints.clippy]
# Leptos view fns return `impl IntoView` consumed by the framework; a caller can't ignore
# the return, so must_use_candidate is noise crate-wide here. (#94)
must_use_candidate = "allow"
```

The flag stays **on** in every other crate. This disable also silences the lint
on the one genuine non-view helper
(`web/src/pages/ui.rs::local_datetime_to_utc_rfc3339`); we still add
`#[must_use]` there as a deliberate, manual keep (no longer clippy-enforced, so
it's called out here so a future reader knows it was intentional).

No other clippy or rustc lint is disabled at crate/workspace level;
`clippy.toml` gains no new entries.

## Approved `#[expect]` keeper set (the only survivors permitted)

Every keeper is converted from `#[allow]`→`#[expect]` and carries an inline
rationale.

- **K1 — `web/src/error.rs:397` `disallowed_methods`.** The single sanctioned
  `Resource::new` inside `web::server_resource`; the clippy.toml ban exists
  precisely to force all other call sites through this wrapper. Structurally
  unfixable.
- **K2 — `storage/src/test_support.rs` `unwrap_used`/`expect_used`
  (module-level).** Deliberately unwrap/expect-heavy both-backend **test
  scaffolding** (`test-support` feature, ADR-0033); not runtime code, and
  feature-gated so clippy.toml's test allowance cannot reach it. Keep the
  module-level `#![expect(...)]` + existing rationale.
- **K3 — `single_component_path_imports` (`use rstest_reuse;`) — conditional.**
  Mandated by the `rstest_reuse` 0.7 dependency for its `#[template]`/`#[apply]`
  macros. **First attempt a fix** (e.g. `use rstest_reuse::*;` or hoisting one
  import to each test crate root); if no formulation satisfies the lint, keep a
  justified `#[expect]`, consolidated to the fewest sites (ideally one per test
  crate root, not 27).

## Fix-the-code work by category (default disposition)

| Category                                                                          |          Occ. | Disposition                                                                                                                                                                                                                   |
| --------------------------------------------------------------------------------- | ------------: | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `must_use_candidate` (web views)                                                  |            46 | D-config-1 crate disable                                                                                                                                                                                                      |
| `must_use_candidate` (real helper)                                                |             1 | manual `#[must_use]` (see D-config-1)                                                                                                                                                                                         |
| `unwrap_used`/`expect_used` in `#[cfg(test)]` + `server/tests/**`                 | most of 34/40 | **delete** the redundant in-source allows (clippy.toml already covers)                                                                                                                                                        |
| `expect_used` **production** (`server/src/media_manager.rs:265`)                  |             1 | **fix (P2):** `.parent().ok_or_else(..)?`                                                                                                                                                                                     |
| `expect_used` feature-gated `src/` (mailer test-double ×2, password cheap-kdf ×1) |             3 | **fix:** recover poisoned lock (`unwrap_or_else(\|e\| e.into_inner())`); propagate `Result` through `hasher()`/`hash()`. Fallback to justified `#[expect]` only if propagation proves disproportionate (flagged, not silent). |
| `too_many_lines` (prod ~12 + test ~33)                                            |            45 | **fix:** split functions (test code included — P5/Q2)                                                                                                                                                                         |
| `similar_names` (prod 1 + test 26)                                                |            27 | **fix:** rename (test code included)                                                                                                                                                                                          |
| `items_after_statements` (test)                                                   |            26 | **fix:** hoist items above statements                                                                                                                                                                                         |
| `unused_async` (test)                                                             |            26 | **fix:** drop needless `async` (or `await` properly)                                                                                                                                                                          |
| `too_many_arguments`                                                              |             9 | **fix:** parameter structs / restructure                                                                                                                                                                                      |
| `needless_pass_by_value`                                                          |             9 | **fix:** borrow instead of move                                                                                                                                                                                               |
| `cast_possible_truncation`/`cast_precision_loss`/`cast_sign_loss`                 |            14 | **fix:** `TryFrom` + handle; justified `#[expect]` only where provably lossless, with an in-comment proof                                                                                                                     |
| `disallowed_methods`                                                              |             1 | **K1 keeper**                                                                                                                                                                                                                 |
| `struct_field_names`, `duration_suboptimal_units`, `format_collect`               |             3 | **fix** each in place                                                                                                                                                                                                         |
| rustc `unused_macros`                                                             |            24 | **delete** — no `macro_rules!` exists in the test tree; the allows are inert cruft (verify green after removal)                                                                                                               |
| rustc `unused_imports`, `dead_code`, `unused_variables`                           |             4 | **fix:** delete dead item / restructure (or justified `#[expect]` if a re-export is legitimately conditional)                                                                                                                 |

## Out of scope

- Runtime test/coverage skips; `#[ignore]`; `cov:ignore`.
- The gate/CI config beyond D-config-1. `clippy.toml` and the workspace lint
  levels are not weakened.
- **The anti-regrowth guardrail** (a CI check flagging new suppressions). #94
  asks only to _consider_ it — the plan's **first task files it as a separate
  follow-up issue**; it is not implemented here.

## Acceptance criteria

Stated so ship-time conformance review can tell delivered from not.

1. **AC-zero-allow.** The survey command reports **0** `#[allow(...)]` in `*.rs`
   (excl. `target/`). Every remaining suppression is `#[expect(...)]`.
2. **AC-keepers.** Every surviving `#[expect(...)]` (a) belongs to the K1–K3
   approved set (or a flagged cast/re-export proof-carrying exception explicitly
   noted in its commit), and (b) carries an inline rationale comment. No
   `#[expect]` exists outside that set.
3. **AC-no-prod-panic.** `unwrap_used`/`expect_used = deny` remain in the
   workspace lints and no in-source allow overrides them on a runtime path; with
   AC-zero-allow + AC-gate green, any production `unwrap`/`expect` is a hard
   compile failure — in particular `media_manager` no longer panics on
   `.parent()`.
4. **AC-config.** `web/Cargo.toml` carries the single
   `must_use_candidate = "allow"` line with rationale; `clippy.toml` is
   unchanged from its pre-existing content; `clippy::pedantic = warn` and the
   `deny`s are intact. No other lint is disabled at crate/workspace level.
5. **AC-gate.** `cargo xtask validate` is green (static + clippy `-D warnings` +
   coverage + all four e2e combos).
6. **AC-behavior.** No runtime behavior change beyond the `media_manager`
   panic→error path; function-splits / param-structs / borrow changes are
   behavior-preserving and covered by the existing suite (coverage gate stays
   green).
7. **AC-followup.** A follow-up issue for the anti-regrowth guardrail exists and
   is linked from #94.

## Risks

- **Test-code churn (Q2):** splitting long test fns and renaming vars across ~27
  modules is broad but low-risk; behavior is unchanged and each category lands
  in its own commit.
- **`cast_*` correctness:** a wrong "provably lossless" claim hides a real
  truncation bug — prefer a checked `TryFrom`; assert losslessness only with an
  in-comment proof.
- **`unused_macros` deletion:** if a macro _is_ actually used-but-elsewhere,
  removing the allow re-surfaces the warning — caught immediately by the gate,
  then fixed.
- **rstest_reuse (K3):** the fix attempt may fail; the fallback justified
  `#[expect]` is bounded and consolidated.
