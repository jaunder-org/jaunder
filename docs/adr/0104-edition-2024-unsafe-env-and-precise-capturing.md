# ADR-0104: Edition 2024 has one audited `unsafe` env seam, and borrowing views capture precisely

- Status: accepted
- Date: 2026-08-05
- Deciders: Michael Alan Dorman

## Context

The workspace moved all 13 crates from edition 2021 to edition 2024 (#826). The
migration was measured rather than estimated: no production code failed to
compile. The compile cost fell in two places, and each forced a decision that
outlives the migration itself.

### Process environment mutation became `unsafe`

Edition 2024 makes `std::env::set_var` and `std::env::remove_var` `unsafe` (RFC
3543). They are not thread-safe: a mutation racing any concurrent read —
including reads inside libc, in a thread the test never spawned — is undefined
behaviour.

The workspace had **79** such calls, all under `#[cfg(test)]`, in five files:
`server/src/observability.rs` (39), `server/src/cli.rs` (17),
`storage/src/postgres/mod.rs` (15), `host/src/capture.rs` (5),
`storage/src/db.rs` (3).

**How much real hazard was there?** Less than the naive reading suggests, and it
is worth being accurate about. The gated runner is **cargo-nextest**
(CONTRIBUTING.md:286), which executes each test in its **own process**. Under
nextest, one test's `set_var` cannot race another test's read, and env state
cannot leak from one test to the next. So the classic "parallel test threads
share one environment" argument — true of `cargo test`, and the argument the
existing per-module locks were written against — largely does not apply to how
this repo actually runs its suite.

What remains, and is real:

1. **Within a single test**, a multi-threaded tokio runtime (`#[tokio::test]`,
   and the `sqlx`/OTLP machinery underneath) can read the environment on another
   thread while the test body mutates it. Three env-mutating tests here are
   `#[tokio::test]`. This is genuine UB and nextest does nothing about it.
2. **`cargo test` remains usable** by developers and is how doctests run
   (`cargo nextest` structurally cannot run doctests — CONTRIBUTING.md:431). The
   thread-sharing hazard is live on that path.
3. **The `unsafe` obligation is a language rule**, not a property of the runner.
   It must be discharged regardless of how comfortable nextest makes us.

The pre-existing defence was in any case unsound on its own terms: each module
held a _private_ `static ENV_LOCK: Mutex<()>`, so two modules in the same binary
took two different mutexes and did not serialize against each other at all. The
cleanup was lossy too — a trailing `remove_var` clobbers whatever value the
variable had on entry, and is skipped entirely when a test panics.

The mechanical migration — wrap each of the 79 sites in `unsafe { … }` — would
have compiled, and would have spread 79 unaudited safety claims through the test
suite while preserving both defects.

### Return-position `impl Trait` began capturing all lifetimes

Edition 2024 adopts RFC 3498: `-> impl Trait` captures every in-scope lifetime.
Leptos requires a **stored** view to be `'static`. So

```rust
fn render_media_row(item: &Item, …) -> impl IntoView
```

stopped compiling on `wasm32-unknown-unknown` — five errors in `web` (`E0515`
and `E0521`, the latter saying `'1` must outlive `'static` in as many words),
arising from four helpers: `render_invite_row`, `render_media_row`,
`force_delete_form`, and `media_key_fields` (which the middle two call, so its
captured lifetime propagates into them).

The edition also turned 19 nested `if let` ladders into `collapsible_if`
warnings whose suggested fix is a **let-chain** — a 2024-only construct, so a
payoff rather than a cost. Worth recording how that count was arrived at: an
early probe ran `cargo clippy` per crate and read the exit code, which reported
"clean" because warnings do not fail an exit code. The gate runs `-D warnings`,
and found 19 where the probe had found 3.

The captured lifetime was spurious in all four. Each derives owned data
(`to_string()`, `into_owned()`) _before_ its `view!` and lends nothing across
the view boundary; every caller already owned the value it passed by reference.

## Decision

### 1. Tests mutate the process environment only through `common::test_support::with_env`

```rust
with_env(|env| {
    env.set("JAUNDER_LOG_FORMAT", "json");
    env.remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
    assert_eq!(resolve().format, Format::Json);
});
```

`with_env` is the **only** place in the workspace that names `set_var` or
`remove_var`, and the only env-related `unsafe` block. It:

- takes **one process-global lock**, shared workspace-wide, held for the whole
  closure — so that on the `cargo test` path the serialization actually holds,
  unlike the per-module mutexes it replaces;
- **restores prior values** on exit, including on panic, recording each key's
  value at _first_ touch so set-then-change-then-remove still unwinds to the
  original rather than an intermediate;
- ignores lock poisoning (`unwrap_or_else(PoisonError::into_inner)`): the lock
  guards no invariant of its own, so a panicking test must not cascade into
  every later one. It is deliberately **not reentrant**; nesting is a documented
  bug.

The **scoped-closure** form was chosen over an RAII guard deliberately. A guard
can be leaked, bound to `_` and dropped immediately, or held past its intended
scope; the closure makes the bracket structural, so the invariant is discharged
by construction rather than by reviewer vigilance. The handle is borrowed from
the closure and cannot escape it, so that property survives.

**Why a handle rather than an up-front delta array.** The first design took the
whole delta as an argument — `with_env([(k, Some(v)), …], f)`. Two shapes in
this codebase defeat it:

1. **Reader-only critical sections.** The lock's actual contract was "serializes
   all tests that **read or write**" the environment, and clap resolves env vars
   _during_ `parse`. Of 32 lock acquisitions in `server/src/cli.rs` only 10
   mutated; of 21 in `server/src/observability.rs`, 16. So ~26 tests held the
   lock purely to read a stable environment. They are now `with_env(|_env| …)`.
   Scoping the migration by _mutation site_ rather than _lock acquisition_ would
   have silently dropped their serialization, and no test or gate would have
   noticed.
2. **Interleaved states.** `host::capture`'s unset/blank test needs the variable
   removed, an assertion, then set to blank, then another assertion. Two
   sequential `with_env` calls would split one critical section into two and
   reopen the window the single guard closes.

The handle also removes the `None::<&str>` turbofish that an `Option`-valued
delta array forces at remove-only call sites.

**What the rule does _not_ cover.** The lock exists to serialize against
**in-process mutation**. A read of a variable that nothing in the process ever
writes has no writer to race, and needs no lock. The live example is
`storage::test_support`'s reads of `JAUNDER_PG_TEST_URL` and
`JAUNDER_PG_BOOTSTRAP_TEST_URL`: those are set by `devtool` on the _child_
process via `Command::env` before the test binary starts
(`tools/devtool/src/pg.rs`, `coverage/emit.rs`), exactly as
`test-support/tests/cli.rs` passes `JAUNDER_CAPTURE_DIR`. Wrapping them would
buy nothing and would cost real throughput — `postgres_url_string()` runs during
per-test database provisioning, so taking a process-global lock there would
serialize the entire Postgres suite.

The test to apply, then, is _"does any in-process code write this variable?"_ —
not _"is this an env read?"_.

Accepted limitation: the closure is synchronous, so an env delta cannot span an
`.await`. Verified to constrain nothing at the time of writing — the three
`#[tokio::test]` tests that mutate env run only synchronous code
(`init_tracing_impl`) inside the env-sensitive region. Note this does **not**
address hazard (1) above, where the runtime's _other_ threads read env during
the closure; it narrows the window rather than closing it. Closing it would mean
not reading configuration from the environment at all, which is a larger design
question this ADR does not settle.

**Why `common`.** `common` is the only candidate home compiled for
`wasm32-unknown-unknown`; `storage::test_support` (ADR-0033) and the
`test-support` binary crate (ADR-0046) are host-only, and `storage` is anyway
downstream of the crates that need this. `common::test_support` is already
feature-gated (`#[cfg(any(test, feature = "test-support"))]`) and already
dev-depended on by `host` and `storage`. `server` had been reaching it only by
transitive feature unification and now declares it explicitly.

### 2. A view helper that borrows a parameter returns `impl IntoView + use<>`

```rust
fn render_media_row(item: &Item, …) -> impl IntoView + use<> { … }
```

`use<>` is precise capturing (RFC 3617): the returned opaque type captures no
lifetimes and no type parameters. That is exactly the fact in question — the
view borrows nothing — so the annotation documents the invariant rather than
merely silencing the error.

Chosen over changing the helpers to take owned values. Taking ownership would
have worked (every caller owns what it passes), but it discards a reference that
correctly describes the call, and would force a clone where `media_key_fields`
feeds the other two. Keeping `&Item` in the signature is the point: edition 2024
is what makes "borrow the parameter, return an owned view" expressible at all.

### 3. The resolver and the formatting style are pinned, not inferred

Edition 2024 silently drags two things along with it. Both are now stated
explicitly so a future edition move changes the language and nothing else.

**`resolver = "3"` in all three workspace roots.** A package workspace root
infers its resolver from its package's edition, so flipping `xtask` to 2024
would have moved it to resolver 3 with no diff line to see it in. We adopted 3
everywhere instead of pinning 2 against it — measured as a genuine no-op here
(`cargo tree -e features` on host and wasm, and `Cargo.lock`, all byte-identical
across v2 and v3), because resolver v2→v3 is _MSRV-aware version selection_ and
no manifest declares `rust-version`. Note this is **not** the
feature-unification change; that was v1→v2, and confusing the two is what nearly
split this into a needless follow-up PR.

**`style_edition = "2024"` in `.rustfmt.toml`.** `cargo fmt` passes each crate's
manifest edition to rustfmt, overriding the config's `edition`, and
`style_edition` then defaults to whatever edition is in force — so formatting
style had been quietly tracking the crate edition. The 2024 style edition
reformatted 209 files (161 hunks of resorted `use` declarations, 192 of
re-wrapped macro arguments, mostly `assert!` bodies). That was taken
deliberately as its own commit; pinning the value decouples the two decisions
permanently.

## Consequences

- `unsafe` for this hazard is auditable by reading one function. A new raw
  `set_var` is greppable (`rg '(set|remove)_var' -g '*.rs'`) and should be
  rejected in review. Note the workspace has **no** `unsafe_code` lint in
  `[workspace.lints]` and CONTRIBUTING.md does not otherwise constrain `unsafe`,
  so review plus that grep is the whole enforcement mechanism.
- Tests no longer leak environment state on panic. Under nextest this is
  belt-and-braces (process isolation already prevents cross-test leakage); on
  the `cargo test` and doctest paths it is load-bearing.
- `use<>` is not expressible on edition 2021, so decision 2 is load-bearing on
  the edition and cannot be back-ported.
- Both idioms are 2024-only, which is a further reason the workspace moves as a
  unit rather than crate by crate: a mixed-edition workspace would make "which
  idiom applies here?" depend on the file.

## Alternatives considered

- **Wrap all 79 sites in `unsafe { … }`.** Smallest diff, truest to "this is
  only an edition flip." Rejected: it multiplies unaudited safety claims and
  preserves the per-module-mutex unsoundness and the panic-leak.
- **An RAII `EnvGuard` handle.** Ergonomic for tests that mutate mid-body.
  Rejected: leakable, and the invariant stays conventional rather than
  structural.
- **A scoped closure taking an up-front delta array.** The first design, and the
  one this ADR originally recorded. Rejected once the reader-only and
  interleaved-state shapes above showed it could not express real call sites.
- **Do nothing beyond `unsafe`, on the grounds that nextest isolates tests.**
  Tempting given the runner, and rejected for hazards (1) and (2) above: the
  in-test multi-threaded read is untouched by process isolation, and the doctest
  path does not use nextest at all.
- **Owned parameters for the view helpers.** Rejected as above; it forfeits the
  expressiveness the migration was undertaken to obtain.

## References

- #826 — the migration issue and its measurements.
- #301 — the `web` lint-suppression work; three of its suppressions are marked
  contingent on this edition and are cleared on that branch, not this one.
- RFC 3543 (unsafe env), RFC 3498 (RPIT lifetime capture), RFC 3617 (precise
  capturing).
- ADR-0033 — `storage`'s in-crate `test_support` module; the precedent for
  feature-gated test support living in a library crate, and the reason this
  helper is _not_ homed there.
- ADR-0046 — the `test-support` **binary** for out-of-process e2e seeding; cited
  only to distinguish it, being host-only and not a library surface.
- ADR-0095 — the doctest gate, whose generated fixtures move to 2024 alongside.
