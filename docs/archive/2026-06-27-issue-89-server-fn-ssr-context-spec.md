# Issue #89 — SSR server-fn context survives awaits (central fix)

* Status: proposed
* Issue: #89 (web: `#[server]` fns fetch storage via `expect_context`, which panics under SSR)
* Date: 2026-06-27

## Problem

A `#[server]` function that reads a Leptos context (`expect_context::<Arc<dyn FooStorage>>()`)
**after** an `.await`, when rendered during SSR, intermittently panics:

```
reactive_graph .../owner/context.rs:306: expected context of type
"alloc::sync::Arc<dyn storage::users::UserStorage>" to be present
```

It is **rare** (measured ~0.4% of SSR renders of an affected fn: 0 in 686 renders one run,
~3/run other runs) and **load-correlated** — a slow `require_auth()` session `UPDATE`
(observed 277–606 ms under e2e parallelism) widens the window. The panic is isolated to a
spawned task so the HTTP response still returns 200, but a panicking SSR resource intermittently
stalls hydration, surfacing as a **flaky `body[data-hydrated]` timeout** (e.g.
`visibility.spec.ts`). It has lived on `main` through many green runs because the e2e VM log is
only captured on failure (see #93).

Affected fns today (all read context *after* an await): `web::backup::backup_warning_visible`,
`web::backup::current_user_is_operator`, `web::backup::get_backup_settings`. Every other server
fn happens to read its context **before** any await — an implicit convention documented only in
a code comment in `web/src/posts/mod.rs` ("task-local context is gone … so a `use_context`
placed after an await returns `None` … reads context first"). The three failing fns are bugs
that violated that unwritten rule.

## Root cause (proven, not inferred)

From `reactive_graph` 0.2.14 source (`src/owner.rs`):

* The thread-local current owner is a **`WeakOwner`** (`static OWNER: RefCell<Option<WeakOwner>>`).
* `expect_context`/`use_context` resolve a value by **upgrading** that weak ref
  (`Owner::current()`) and walking `OwnerInner.contexts` up the parent chain.
* `OwnerInner.contexts` (and the whole owner) is freed when the owner's last **strong** `Arc`
  ref is dropped.

So: while a server-fn future is suspended at an `.await`, the reactive graph drops the fetcher
owner's last strong ref → `OwnerInner` is dropped → context map freed → the post-await
`Owner::current()` weak-upgrade returns `None` → `expect_context` panics. Reading context
*before* the await works because the `Arc<dyn …Storage>` handle is copied out while the owner
is still alive. Rare because it is a race against that drop.

This is reproduced **deterministically** (no SSR runtime, no race) in
`web/src/error.rs::owner_lifetime::context_lost_when_owner_dropped_across_await`, which hand-polls
a future and drops the owner between polls.

## Decision

**Make server-fn context survive awaits centrally, in `server_boundary`** (the function every
`#[server]` body already routes through via the `boundary!` macro), by running the body inside a
`reactive_graph` `ScopedFuture` — which holds a **strong** owner ref (keeping `OwnerInner` alive)
and re-applies it on every poll (`owner.with(...)`).

`web/src/error.rs`, `server_boundary`:

```rust
let outcome = if leptos::reactive::owner::Owner::current().is_some() {
    leptos::reactive::computed::ScopedFuture::new_untracked(future).await
} else {
    future.await
};
match outcome { /* unchanged error handling */ }
```

* `new_untracked` (not `new`): a server-fn body performs no reactive reads, so we keep the owner
  but clear the observer — avoids any accidental reactive subscription.
* **The `Owner::current().is_some()` guard is required.** `ScopedFuture::new` captures
  `Owner::current().unwrap_or_default()`; wrapping when *no* owner is current would capture a
  fresh **empty** owner and turn the rare race into a **deterministic** context loss — strictly
  worse. Proven by `owner_lifetime::scoped_future_with_no_current_owner_sees_empty_context`. With
  the guard the fix is **strictly safe**: when an owner is present it is retained across awaits;
  when absent, behaviour is exactly as today.

This fixes all ~75 server fns and every future one, leaves the documented `expect_context` DI
(ADR-0016) intact, and touches only the server path — on wasm the `#[server]` body is replaced by
the HTTP stub, so `server_boundary`/`ScopedFuture` never run client-side.

### Consequences

* The three buggy fns need **no per-fn change** — the central wrapper covers them. No `#[server]`
  bodies are reordered.
* The implicit "read context before await" convention is **retired, including its defensive
  code** — not just its comments. The ~6 server fns that used
  `use_context::<T>().ok_or_else(server_message)?` graceful-degradation (because context could be
  `None` after an await) revert to the uniform `expect_context::<T>()` DI (ADR-0016), now that
  context is guaranteed present. Statement *ordering* is left as-is (reordering would be
  gratuitous churn that reads naturally once the comments are gone), and `require_auth` is
  untouched (its `Parts`→extensions path is a separate, load-bearing mechanism). The graceful-error
  → panic-if-absent trade-off is accepted: a missing context is a real bug now, and the #93
  zero-panic gate backstops it.
* `server_boundary` holds the owner alive for the body's (short, request-scoped) duration — a
  benign delay of its cleanup.

## Alternatives considered

* **Per-fn convention + lint** — reorder the three fns to read context before any await, and
  enforce. *Rejected as the primary fix:* it is the status quo that already failed three times,
  and "no context read dominated by an `.await`" has no off-the-shelf clippy lint (a sound custom
  one is real work). It remains a correct *fallback* if the central fix is ever backed out.
* **Fix upstream context propagation** — not ours to change; it is `reactive_graph` owner-lifetime
  behaviour, and the framework offers `ScopedFuture` as the supported tool for exactly this.

## Scope of change

* `web/src/error.rs` — guarded `ScopedFuture` wrap in `server_boundary`.
* `web/src/error.rs` — the four `owner_lifetime` deterministic tests (already written: bug repro,
  central fix, per-fn pattern, the empty-owner trap). Permanent regression suite.
* Retire the convention's defensive scaffolding: convert the `use_context(...).ok_or_else(...)`
  workarounds in `list_my_audiences`, `create_audience`, `subscribe_to`, the `SubscribeButton`
  state fn, `default_audience_selection`, and `post_audience_selection` to `expect_context::<T>()`;
  remove/rewrite the stale module-level (`audiences`, `subscriptions`) and per-fn convention
  comments to reference the new guarantee. No statement reordering; `require_auth` untouched.
* `docs/adr/0016-dependency-injection-and-appstate.md` — add a section recording the SSR
  context-lifetime guarantee (the DI mechanism is now reliable regardless of await ordering).

## Testing

* **Mechanism + fix, deterministic (the real proof):** the four `owner_lifetime` tests run in
  milliseconds and assert the bug, the central fix, the per-fn pattern, and the empty-owner trap.
* **End-to-end sanity:** a full e2e run whose captured VM log shows **zero** `UserStorage`
  `expect_context` panics. Because the race is rare, a single green run is corroboration, not
  proof — the deterministic tests are the proof, and #93 (capture e2e logs always + fail on any
  SSR panic) is the ongoing guard. #89 and #93 are companions.

## Separable concerns

* #93 (e2e capture-always + zero-panic gate) — already filed (P1). It is the long-term regression
  guard for this class; it is **not** a blocker for landing #89.
