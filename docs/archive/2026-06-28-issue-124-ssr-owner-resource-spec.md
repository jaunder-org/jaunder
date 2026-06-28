# Issue #124 — SSR reactive-owner loss for server fns invoked from `Resource` fetchers

* Status: approved (design), pending implementation
* Deciders: mdorman, Claude
* Date: 2026-06-28
* Governing ADR: `docs/adr/0016-dependency-injection-and-appstate.md` + its #89 addendum
  (this cycle adds a second addendum, completing the SSR-context-lifetime story)
* Surfaced by: the #93 e2e zero-panic gate (ADR-0032), which turned a flaky panic into a hard CI failure (blocks PR #121 / issue #72).

## Goal

Eliminate the flaky `expect_context` panic during SSR when a server function is
invoked from a `Resource` fetcher — the residual gap left by #89 — while keeping
ADR-0016's per-trait Leptos-context DI unchanged and making the fix **impossible to
forget** (statically guarded), with **zero per-handler boilerplate**.

## Root cause (established)

* **DI:** server fns read deps from the reactive owner's context —
  `expect_context::<Arc<dyn *Storage>>()` and request `Parts` via
  `use_context::<Parts>()`. Both are provided per-request into the owner by
  `provide_app_state_contexts` (`server/src/context.rs:25-39`) inside *both* the
  server-fn handler and the SSR-render handler (`server/src/lib.rs:68-110`). This is
  the ADR-0016-sanctioned channel.
* **#89's fix:** `server_boundary` (`web/src/error.rs:371-398`, the `boundary!`
  macro) wraps the body in `ScopedFuture::new_untracked` — holding a *strong* owner
  ref and re-applying it on each poll — **but only when an owner is current at the
  body's first poll** (else `new_untracked` would capture an empty owner, worse).
* **The gap:** a server fn invoked from an **SSR `Resource` fetcher**
  (`web/src/pages/ui.rs:1059-1063` Sidebar, `web/src/pages/home.rs`, etc.) has the
  request/component owner's strong ref **already dropped** by the time its future is
  first polled on a tokio worker. So `server_boundary`'s guard takes the unprotected
  branch and every context read resolves to `None` → panic at
  `reactive_graph/owner/context.rs:306`. Confirmed: `require_auth`'s
  `use_context::<Parts>()` returns `None` *before* any await (so read-ordering can't
  help). The owner is live **only at fetcher invocation**; an `async fn` body has no
  synchronous prologue, so the capture must happen at the `Resource` layer, not in
  the handler.
* **Not a jaunder anti-pattern:** this is a known Leptos SSR + multi-thread + reactive
  -ownership rough edge (leptos issues #2562 spawn_local, #2341 SSR context-missing,
  #3729 arena-on-worker). The established workaround is exactly #89's mechanism —
  capture the `Owner` and run the future scoped to it — applied at the layer where the
  owner is still alive.

## Design

### 1. `server_resource` — one owner-capturing constructor

Add to `web` (e.g. `web/src/resource.rs`, re-exported from the crate root). It is the
**only** sanctioned way to create a `Resource` in `web`:

```rust
/// Create a `Resource` whose fetcher future keeps the reactive owner alive across
/// every poll. The fetcher runs inside the live component owner; we capture it via
/// `ScopedFuture::new_untracked` *there*, so server-fn context (storage trait objects
/// + request `Parts`) survives even when the future is later polled on a worker thread
/// detached from the owner. This is #89's `server_boundary` mechanism applied at the
/// layer where the owner is still live (the body-level wrap can't help — by the body's
/// first poll the SSR-resource owner is already gone). `new_untracked` so context reads
/// don't create spurious reactive subscriptions.
pub fn server_resource<S, T>(
    source: impl Fn() -> S + Send + Sync + 'static,
    fetcher: impl Fn(S) -> /* Fut: Future<Output = T> + Send */ _ + Send + Sync + 'static,
) -> Resource<T>
where
    S: PartialEq + Clone + Send + Sync + 'static,
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    Resource::new(source, move |s| {
        leptos::reactive::computed::ScopedFuture::new_untracked(fetcher(s))
    })
}
```

Exact generic bounds / the `Future` argument form are matched to `Resource::new`'s
signature in the pinned Leptos at implementation (the body — wrap the fetcher's future
in `ScopedFuture::new_untracked` — is the invariant). The wrapper is unconditional
(it compiles and is harmless on the client/hydrate path, where the owner is also live).

**Server-fn handlers are unchanged** — they keep `boundary!` + `expect_context`
(#89 still covers the HTTP `/api` path where the owner is live at entry).

### 2. Migrate `web`'s `Resource::new` sites to `server_resource`

Every `Resource::new` in `web/src/**` becomes `server_resource(...)` (uniform; the
wrap is harmless for fetchers that read no context). Enumerated in the plan.

### 3. Static guard — make omission a gate failure

A scanning test (a `#[test]` in `web`, or an `xtask` static check) fails if
`web/src/**/*.rs` contains a raw `Resource::new(` (the sanctioned constructor is
`server_resource`). This gives the "statically determine the boilerplate is present"
property without per-site annotations. **First try** clippy `disallowed-methods`
targeting the inherent `Resource::new`; **iff** it binds use it, **else** the scanning
test (the inherent-generic-method path may not bind in clippy — verified at impl).

### 4. `Action::new` assessment

Actions also drive server-fn futures. Assess whether an `Action` invoked from a
detachable context shares the exposure; if so, add a sibling `server_action` wrapper
and extend the guard. If Actions only fire post-hydration with a live owner, document
that and leave them. (Decided in the plan after a focused check; not assumed here.)

### 5. Deterministic proof (mirrors #89's `owner_lifetime` tests)

A unit test that does **not** rely on the flaky e2e:
* Build a future via `server_resource`'s wrapping; drop the owner's last *strong* ref
  before the first poll; assert a `expect_context::<Marker>()` inside still resolves.
* Negative control: the same future **without** the wrap loses the context
  (panics / `use_context` is `None`), proving the wrap is load-bearing.

## Edge cases / tests

* `owner_lifetime`: wrapper preserves context across an owner strong-ref drop before
  first poll; raw `Resource::new` loses it (negative control).
* Static guard: a deliberately-added raw `Resource::new` in `web/src` fails the
  scan/lint.
* The `/` home-page SSR no longer panics — the zero-panic e2e gate passes (validated
  by the existing e2e once this lands; #72's e2e is the acceptance witness after its
  rebase).
* Client/hydrate: resources still load (wrapper is transparent there).
* No storage-backend dimension (web layer); no migration.

## ADR record

A second **addendum to ADR-0016** (2026-06-28, #124): #89's `ScopedFuture` owner-keep
-alive is extended from the server-fn body to the `Resource` layer via
`server_resource`, because server fns invoked from SSR resource fetchers have no live
owner at body entry; the sanctioned per-trait `expect_context` DI is unchanged, and a
static guard makes the wrapper non-optional. (Addendum, not a new ADR — same topic and
precedent as the #89 addendum; no `docs/README.md` row change.)

## Conventions

No `Co-Authored-By`. All work on `worktree-issue-124-userstorage-context`, never
`main`. Gate per `CONTRIBUTING.md`.
