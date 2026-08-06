# Issue #826 — move the whole workspace to Rust edition 2024

Status: proposed Issue: https://github.com/jaunder-org/jaunder/issues/826

## Summary

Move all 13 crates from `edition = "2021"` to `edition = "2024"`, and pay the
migration cost the change forces. The cost was **measured** on this branch, not
estimated; see [Measurement](#measurement).

No production code _fails to compile_ under 2024. The compile-breakage is
entirely test-side (79 `unsafe`-env call sites) plus **four** wasm-only view
helpers in `web` (five error sites), fixable with a four-token change.
Production code is still _edited_ — 19 nested `if let` ladders collapse into
let-chains — but only where clippy asks and only in ways the gate verifies.

The edition is worth taking for what it makes _expressible_: precise capturing
(`use<..>`) lets a helper borrow a parameter and still return a `'static` view,
and let-chains collapse the ladders clippy is already flagging.

## Scope

**In scope**

- Flip `edition` to `"2024"` in all 13 crate manifests: the 9 root-workspace
  members (`client`, `common`, `csr`, `host`, `macros`, `server`, `storage`,
  `test-support`, `web`), `xtask`, and `tools/{coverage,devtool,doctests}`.
- Adopt `resolver = "3"` explicitly in all three workspace roots (see D5).
- Flip the doctest harness's generated fixture manifest to 2024 (see D6).
- Introduce `common::test_support::with_env` and route all 79 test-side process
  environment mutations through it, deleting the ad-hoc per-module `ENV_LOCK`
  statics and `lock_env()` helpers and the manual `remove_var` cleanup.
- Declare `common/test-support` explicitly in `server`'s dev-dependencies (D7).
- Add `+ use<>` to the four `web` view helpers that borrow a parameter.
- Clear the migration warnings (19 `collapsible_if` → let-chains, two mis-gated
  test-only structs).
- **Pin `style_edition = "2024"` and take the resulting reformat** — added to
  scope mid-execution (D8), and by file count the largest part of the diff.
- Record the resulting conventions in one ADR, and **promote it** at ship.

**Out of scope**

- **The three `CONTINGENT ON EDITION 2021` suppressions from #301.** They do not
  exist on `main`; they live only in the unmerged
  `worktree-issue-301-web-lint-suppressions` branch. #301 rebases onto the new
  edition and deletes them itself. This spec deliberately does not touch those
  files, so the two branches do not conflict. #826's issue body lists this as
  in-scope; it is hereby re-homed to #301, and #826 is closed by this branch
  with a comment recording the reassignment. Merge order between the two
  branches is unconstrained.
- `PostDisplay::{post, banner, tag_context}` — the fourth #301 suppression, a
  structural question about post ownership. Untouched.
- Any behavioural change. This migration is intended to be behaviour-preserving.

## Measurement

Taken on this branch (`worktree-issue-826-edition-2024`, forked at `a772e1a7`,
tagged `wt-base-issue-826`) with all 13 editions already flipped to 2024.
Toolchain is `channel = "stable"`, measured at rustc/clippy 1.95.0.

| Target                                                                                   | Result                                                          |
| ---------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| `cargo clippy -p jaunder -p storage -p host` (libs only)                                 | **clean**                                                       |
| `cargo clippy -p common -p macros -p csr -p client -p web -p test-support --all-targets` | clean, 5 warnings                                               |
| `cargo clippy --all-targets` in `xtask`                                                  | **clean**                                                       |
| `cargo clippy --all-targets` in `tools/{devtool,coverage,doctests}`                      | **clean**                                                       |
| `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown`    | 5 errors → **fixed and re-verified clean**                      |
| `cargo clippy --workspace --all-targets`                                                 | `E0133` wall in `host`/`storage` tests; aborted before `server` |

Three probes carry the argument:

1. **Production code compiles.** `server`/`storage`/`host` without
   `--all-targets` succeeds, proving the `E0133` wall is entirely inside
   `#[cfg(test)]`.
2. **The wasm errors are real and cheap.** All five reproduce exactly as #826
   describes, and all five are resolved by four `+ use<>` annotations — verified
   by a clean re-run, with no signature change and no added clone.
3. **The tooling crates are free.** `xtask` and all three `tools/` crates need
   nothing but the edition line.

### Breakage class 1 — `env::set_var` / `remove_var` are `unsafe` (RFC 3543)

**79 call sites, all under `#[cfg(test)]`**, in exactly five files:

| File                          | Sites |
| ----------------------------- | ----- |
| `server/src/observability.rs` | 39    |
| `server/src/cli.rs`           | 17    |
| `storage/src/postgres/mod.rs` | 15    |
| `host/src/capture.rs`         | 5     |
| `storage/src/db.rs`           | 3     |

Every one is spelled `std::env::set_var` / `std::env::remove_var`; there is no
`use std::env` anywhere in the workspace. (`test-support/tests/cli.rs:12`
mentions `set_var` in a doc comment only — it is not a call site and does not
break.)

Today each test module defends itself with a private
`static ENV_LOCK: Mutex<()>` or a local `lock_env()` — at
`server/src/cli.rs:433`, `server/src/observability.rs:540`,
`storage/src/db.rs:302`, `storage/src/postgres/mod.rs:324`,
`host/src/capture.rs:84`. Those do **not** serialize against each other:
separate modules hold separate mutexes. The cleanup is also lossy — a trailing
`remove_var` clobbers a pre-existing value instead of restoring it, and is
skipped entirely on panic.

### Breakage class 2 — RPIT captures all lifetimes (RFC 3498)

Five errors, wasm-only, in `web`, arising from **four** helpers:

| Error site                                  | Helper at fault               |
| ------------------------------------------- | ----------------------------- |
| `web/src/invites/component.rs:75` (`E0515`) | `render_invite_row(&Info)`    |
| `web/src/media/component.rs:258` (`E0515`)  | `render_media_row(&Item, …)`  |
| `web/src/media/component.rs:310` (`E0515`)  | `force_delete_form(&Item, …)` |
| `web/src/media/component.rs:336` (`E0521`)  | `force_delete_form`'s `view!` |
| `web/src/media/component.rs:397` (`E0521`)  | `render_media_row`'s `view!`  |

The fix set is those three **plus `media_key_fields(&Item)`**, which both
`force_delete_form` and `render_media_row` call and whose captured lifetime
propagates into them. Four helpers, four annotations.

Each helper already derives owned data (`to_string()`, `into_owned()`) before
its `view!` and lends nothing across the view boundary — the captured lifetime
is spurious. Every caller already _owns_ the value it passes by reference
(`.into_iter().map(|item| render_media_row(&item, …))`).

### Breakage class 3 — warnings

- `collapsible_if` × **19** (this measurement was wrong at spec time and said 3
  — see below). Clippy's suggested fix is a **let-chain**, 2024-only, so these
  are a payoff rather than a cost. Spread across `storage/src`,
  `client/src/dom.rs`, `web/src/tags/input_state.rs`, `server/src`, and 16 sites
  in `xtask`.

  **Why the count was wrong.** The original probe ran `cargo clippy` per crate
  and judged the result by exit code. Warnings do not fail an exit code, so
  "clean" meant only "not fatal" — while the gate runs `-D warnings`. Any future
  measurement of lint impact must deny warnings explicitly.

- `dead_code` × 2 — `SourceError` (`web/src/error.rs:140`) and `OuterError`
  (`:151`). **Not actually dead:** both are used at
  `:218,219,221,250,267,305, 378,438,472,529`, in tests gated
  `#[cfg(feature = "server")]` while the two definitions are ungated. The
  warning is a feature-gating artifact and the fix is to gate the definitions to
  match — _not_ to delete them (which would break ten tests) and not to
  `#[allow(dead_code)]` them. This warning is edition-independent and therefore
  lands outside the edition commit.

### Residual risks (accepted, not resolved)

1. **Unmeasured test targets.** `server`, `storage`, and `host` test targets
   have never compiled under 2024, because `E0133` aborts first. Further
   test-only 2024 breakage there is undiscovered. Class 1 is therefore fixed
   first and the full gate re-run before the work is considered measured.
2. **Silent semantic changes.** Edition 2024 also changes **tail-expression and
   `if let` temporary drop scopes** and **match ergonomics**. These produce _no
   diagnostic_; they change runtime behaviour. This codebase is full of
   `MutexGuard` and `sqlx` guard temporaries, which is exactly the shape that
   drop-timing changes affect. The only defence is AC2's full gate including the
   four e2e combos, and that defence is imperfect by construction. This is the
   migration's genuine risk and is accepted knowingly.

## Decisions

### D1 — Test env access goes through a scoped closure that receives a handle

**Amended after the plan soundness review** — see "Why a handle" below.

Add to `common::test_support`:

```rust
with_env(|env| {
    env.set("JAUNDER_LOG_FORMAT", "json");
    env.remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
    assert_eq!(resolve().format, Format::Json);
});
```

- It is the **only** place in the workspace naming `set_var`/`remove_var`, and
  the only env-related `unsafe`.
- It takes **one process-global lock** shared workspace-wide, held for the whole
  closure, so serialization is real rather than per-module.
- It **restores prior values** on the way out, including on panic.
- Lock poisoning is **ignored** (`unwrap_or_else(PoisonError::into_inner)`), so
  one panicking test cannot cascade into every later env test.
- The **closure** form was chosen over a bare RAII guard because a guard can be
  leaked, bound to `_`, or held past its scope; the closure keeps the bracket
  structural. The handle is borrowed from the closure and cannot escape it, so
  that property is preserved.

**Why a handle rather than an up-front delta array.** The original form,
`with_env([(k, Some(v)), …], f)`, cannot express two real shapes that exist in
this codebase:

1. **Reader-only critical sections.** `ENV_LOCK`'s actual contract
   (`server/src/cli.rs:430`) is "serializes all tests that **read or write**"
   the env. Of 32 lock acquisitions in `server/src/cli.rs` only 10 mutate; of 21
   in `server/src/observability.rs` only 16 do. That is ~26 tests that hold the
   lock purely to read a stable environment. Deleting `ENV_LOCK` without giving
   them a replacement would silently drop their serialization. They become
   `with_env(|_env| { … })`.
2. **Interleaved states.** `host/src/capture.rs:117-126` removes a variable,
   asserts, sets it to `"   "`, asserts again, removes. Two delta states with an
   assertion between them. Expressing that as two sequential `with_env` calls
   would split one critical section into two and open a race window the current
   single guard closes. With a handle it stays one acquisition.

The handle form also removes the `None::<&str>` type-annotation wart entirely,
since `set` and `remove` are separate methods.

Accepted limitation: the closure is synchronous, so an env delta cannot span an
`.await`. Verified to constrain nothing today — the `#[tokio::test]` tests that
mutate env call only synchronous code (`init_tracing_impl`) inside the
env-sensitive region.

### D1a — One existing test relocates, by exception

`server/src/observability.rs:982 lock_env_recovers_from_poisoned_mutex` tests
`lock_env()` itself, which AC5 deletes. Its behavioural content — a poisoned
lock must not break later callers — is preserved as a `common` unit test (AC9).
This is the **one** sanctioned test relocation in this branch; AC16 names it
explicitly so the set-comparison stays meaningful.

### D2 — Borrowing view helpers return `impl IntoView + use<>`

```rust
fn render_media_row(item: &Item, …) -> impl IntoView + use<> { … }
```

Precise capturing (RFC 3617) states exactly what is true: the returned view
borrows nothing. Chosen over taking owned parameters because it needs no
signature change, adds no clone, and keeps the _reference_ in the signature —
which is the expressiveness the migration exists to obtain.

### D3 — Both conventions land in one ADR, and it is promoted

A single ADR covering D1 and D2, authored as a numberless draft under
`docs/adr/drafts/` per the `jaunder-adr` flow. Because `docs/adr/drafts/` is
gitignored, the draft is invisible to `git diff`; **`cargo xtask adr promote`
must run before the PR merges** so the ADR actually lands in git. AC12 covers
this.

### D4 — #301 is not touched

See [Scope](#scope).

### D5 — Adopt resolver 3 in all three workspace roots

**Amended after measurement — an earlier revision of this spec deferred this to
a follow-up issue on a false premise.** The claim was that resolver 3 is "a
feature-unification change". It is not: feature unification changed in resolver
**v1 → v2**. Resolver **v2 → v3** is _MSRV-aware version selection_ — it prefers
dependency versions compatible with a declared `rust-version`.

This workspace declares **no `rust-version` in any manifest**, so v3 has nothing
to act on. Measured on this branch:

| Probe                                                                          | Result             |
| ------------------------------------------------------------------------------ | ------------------ |
| `cargo tree -e features --workspace` (host), v2 vs v3                          | **byte-identical** |
| `cargo tree -e features --workspace --target wasm32-unknown-unknown`, v2 vs v3 | **byte-identical** |
| `Cargo.lock`, v2 vs v3                                                         | **byte-identical** |

So resolver 3 is a no-op here, and adopting it is strictly better than pinning
against it: edition 2024 _implies_ resolver 3 for a package workspace root, so
`resolver = "2"` in `xtask/Cargo.toml` would be an override fighting the edition
— and a trap for whoever later deletes the line as redundant and gets v3
silently after all.

All three roots therefore declare `resolver = "3"`. All three need the line
explicitly: `Cargo.toml` and `tools/Cargo.toml` are **virtual** manifests, which
have no package edition to infer from and default to resolver **1**, so their
explicit resolver has always been load-bearing and is unrelated to the edition.
Only `xtask/Cargo.toml` is subject to the edition inference.

The follow-up issue (#835) is closed as folded in, and its body — which repeated
the same feature-unification error — is corrected there.

### D8 — Pin `style_edition` and take the reformat

**Added to scope mid-execution, by user decision.** Not anticipated when this
spec was approved, and by file count the largest component of the diff — so it
is recorded here rather than left to be discovered in the commit log.

rustfmt's `style_edition` defaults to whatever `edition` is in force, and
`cargo fmt` passes each crate's _manifest_ edition, overriding `.rustfmt.toml`'s
own `edition` line. So the formatting style had silently been tracking the crate
edition, and flipping to 2024 reformatted **209 files / 361 hunks**:

| Kind                                                    | Hunks |
| ------------------------------------------------------- | ----- |
| `use` declarations resorted (incl. within brace groups) | 161   |
| Macro arguments re-wrapped — mostly `assert!` bodies    | 192   |
| Whitespace only                                         | 8     |

An initial characterisation of this as "almost entirely `use` declarations" was
wrong — it generalised from two sampled files, and the user caught it. The
`assert!` re-wraps are the more invasive half.

The alternative was pinning `style_edition = "2021"`, which would have kept
every source file byte-identical while still moving the language to 2024 — style
edition exists precisely so an edition migration need not force a reformat. The
user chose to take the reformat, as its own commit, landed on the **2021** tree
so that it provably carries no edition semantics.

`style_edition` is now pinned explicitly, so a future edition move changes the
language and nothing else. **Consequence:** this reformat is the sole cause of
the AC15 deviation below.

### D6 — The doctest harness's generated fixtures move to 2024

`tools/doctests/src/harness.rs:35` writes a fixture `Cargo.toml` containing
`edition = "2021"`. The doctest gate (ADR-0095) would therefore validate doc
fences under a different edition than every crate whose fences it gates — a
fence using a let-chain would fail spuriously. It flips to 2024 with the rest.

### D7 — `server` declares `common/test-support` explicitly

`server/Cargo.toml:71` dev-deps `common` with `features = ["test-utils"]` only.
`common::test_support` is reachable from `server`'s tests today purely by
feature unification through `storage`'s `test-support` feature — a transitive
accident. `server` holds 56 of the 79 call sites, so the largest consumer of
`with_env` would depend on an undeclared path. It gains `"test-support"`
explicitly.

## Acceptance criteria

Each is observable. Greps are scoped to `-g '*.rs'` / source directories,
because `docs/archive/` and this spec itself contain the very strings being
searched for — an unscoped grep can never come back empty.

1. **AC1.** All 13 crate manifests declare `edition = "2024"`, and no manifest
   declares 2021. Verify: `rg -n 'edition' -g 'Cargo.toml' -g '!target'` shows
   13 lines, all `"2024"`. (The two virtual roots, `Cargo.toml` and
   `tools/Cargo.toml`, correctly have no `edition` key.)
2. **AC2.** `cargo xtask validate` passes on the branch — the full local gate,
   including the four `{sqlite,postgres}×{chromium,firefox}` e2e combos. This is
   the primary criterion and subsumes AC3.
3. **AC3.**
   `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown`
   exits 0. (Already gate-covered via `xtask/src/steps/static_checks.rs`.)
4. **AC4.** `set_var`/`remove_var` are named in exactly one Rust module,
   `common`'s test-support env helper. Verify:
   `rg -n '(set|remove)_var' -g '*.rs'` matches only that module's file. The
   unqualified spelling is included in the pattern deliberately, so a future
   `use std::env::set_var;` cannot evade it. This requires rewording the
   now-stale doc comment at `test-support/tests/cli.rs:12`, which mentions
   `set_var` in prose; it is reworded, not excluded from the grep.
5. **AC5.** No `ENV_LOCK` or `lock_env` remains in the three affected crates.
   Verify: `rg -n 'ENV_LOCK|lock_env' server/src storage/src host/src` returns
   no matches.
6. **AC6.** `with_env` restores a variable's prior value even when the closure
   panics — unit test in `common` using `catch_unwind`.
7. **AC7.** `with_env` removes a variable that was previously unset, rather than
   leaving it set — unit test.
8. **AC8.** One `with_env` call supports **multiple mutations and interleaved
   states** under a **single** lock acquisition, without self-deadlock — unit
   test that sets a variable, asserts, changes it, asserts again, and removes a
   third, all in one closure. This is the claim that distinguishes D1 from
   wrapping 79 sites in `unsafe`, so it is tested, not asserted.
9. **AC9.** The lock is process-global (exactly one lock static in the
   workspace) and **poison-tolerant**: a unit test panics inside a `with_env`
   under `catch_unwind`, then asserts a subsequent `with_env` still runs. This
   preserves the behaviour of the relocated
   `lock_env_recovers_from_poisoned_mutex` (D1a). 9b. **AC9b.** Reader-only
   critical sections are preserved, not dropped: every test that previously took
   `ENV_LOCK` without mutating now runs inside `with_env(|_env| …)`. Verify: the
   count of `with_env(` call sites in `server/src`, `storage/src`, `host/src` is
   at least the number of former lock acquisitions (32 + 21 + the storage/host
   holders), not merely the number of former mutation sites.
10. **AC10.** The three `collapsible_if` sites are let-chains and the two
    `dead_code` warnings are gone; the gate is warning-clean (implied by AC2,
    stated separately because it is the visible production edit).
11. **AC11.** All three workspace roots — `Cargo.toml`, `tools/Cargo.toml`,
    `xtask/Cargo.toml` — declare `resolver = "3"`. Verify:
    `rg -n 'resolver' -g 'Cargo.toml' -g '!target'` → three lines, all `"3"`.
    The measured no-op is recorded in D5; #835 is closed as folded in.
12. **AC12.** The ADR exists **in git** as a numbered file under `docs/adr/`
    (i.e. `cargo xtask adr promote` has run), and the ADR index lists it.
    Verify: `git diff --name-only wt-base-issue-826..HEAD` includes it.
13. **AC13.** `tools/doctests/src/harness.rs` generates `edition = "2024"`, and
    the doctest gate passes (part of AC2).
14. **AC14.** `server/Cargo.toml`'s dev-dependency on `common` lists
    `"test-support"`.
15. **AC15.** No file owned by the in-flight #301 branch is modified —
    specifically `web/src/avatar/component.rs` and
    `web/src/feed_discovery/component.rs`. Verify:
    `git diff --name-only wt-base-issue-826..HEAD`.

    **Result: violated in letter, by one line.** `web/src/avatar/component.rs`
    is untouched, but `web/src/feed_discovery/component.rs` has a single import
    resorted by the repo-wide rustfmt style-edition change:

    ```
    -use common::feed::{canonicalize, FeedFormat, FeedSurface};
    +use common::feed::{FeedFormat, FeedSurface, canonicalize};
    ```

    This was unavoidable once the reformat was taken: excluding two files from a
    repo-wide format leaves them unformatted and fails the `fmt` gate. The
    criterion's _purpose_ — don't do #301's work, don't create a meaningful
    conflict — holds: no suppression is touched, no signature changed. #301 will
    hit a one-line rebase conflict in that import, which is trivial to resolve.
    Recorded rather than silently ticked.

16. **AC16.** The test population changes only in the two sanctioned ways.
    Verify mechanically with `cargo nextest list` on `wt-base-issue-826` versus
    `HEAD`:
    - **Removed:** exactly one —
      `jaunder observability::tests::lock_env_recovers_from_poisoned_mutex`
      (relocated per D1a).
    - **Added:** only the new `common` env unit tests covering AC6–AC9.
    - **Everything else is set-equal**, and `rg -n '#\[ignore\]' -g '*.rs'`
      gains no new matches.

    Beyond that, "no assertion was weakened" is a reviewer judgment made during
    the ship review, not a mechanical check — it is called out here so the
    reviewer knows to make it. It matters most in the ~26 reader-only
    conversions (AC9b), where a dropped `with_env` wrapper would be invisible to
    every other check.

## Notes

The fork-point anchor for review is the tag `wt-base-issue-826`; diff with
`git diff wt-base-issue-826..HEAD` or `git diff main...HEAD`.

The gated test runner is **cargo-nextest** (CONTRIBUTING.md:286), which is
process-per-test. That materially affects how the env hazard should be argued —
see the ADR — but not whether it must be discharged: the `unsafe` requirement is
a language rule, independent of runner.
