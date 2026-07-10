# Spec — #359: the `Invalidator` revalidation primitive

**Status:** proposed. **Decision record:**
`docs/adr/drafts/web-invalidator-revalidation-idiom.md` (numbered at ship).
**Supersedes proposal of:** #359 itself — renames `Revalidator`→`Invalidator`
and fires on **success**, not on dispatch (§2).

## Problem

Two revalidation patterns coexist in `web`:

- **`.version()`-sourced resources** —
  `server_resource(move || action.version().get(), …)`, used by ~15 co-located
  pages. Built into `ServerAction`.
- **Hand-rolled `Revalidate`** — a context signal + a per-child
  `Effect { if matches!(action.value().get(), Some(Ok(_))) { counter.update(|n| *n += 1) } }`,
  used only by decomposed `audiences` (post-#314).

`.version()` only works when the action and the resource it invalidates live in
the **same component** — a precondition that holds only for monolithic
components. Small, well-factored components are the discipline (ADR-0056), so as
verticals decompose the `.version()` precondition mostly evaporates and the
cross-component / fan-in case (N per-row actions → one shared list) becomes the
norm — a case `.version()` cannot express. The audiences hand-rolled bridge is
the boilerplate that fills the gap: a counter, a success-gated `Effect` per
action, and a `track()`/`.get()` dance, repeated ~5×.

## Decision

Introduce a single reactive primitive — **`Invalidator`** — as the canonical
revalidation idiom for `web`. A committed mutation `notify()`s an `Invalidator`;
the resources that `track()` it refetch. `.version()` is a special case it
subsumes, not a blessed alternative.

### 1. The primitive (`web::reactive::Invalidator`)

A `Copy` newtype over a counter (a resource source must be a _changing value_ —
`server_resource` requires `S: PartialEq` and leptos `Resource` is memoized on
its source, so a bare notify-only `Trigger` returning `()` would never refetch;
the counter is the mechanism, exactly as `ServerAction::version()` is a
counter). The primitive exists to **encapsulate** that counter behind clean
semantics:

- `Invalidator::new()`
- `notify(&self)` — a mutation committed (bump the counter).
- `track(&self) -> u32` — the resource source subscribes.
- `resource<T>(&self, fetch: impl Fn() -> Fut) -> Resource<T>` — a resource that
  refetches when this invalidator fires. The fetcher is **nullary**: the counter
  is an internal detail callers never see. Internally
  `crate::server_resource(move || self.track(), move |_| fetch())`. `T` carries
  `server_resource`'s bounds
  (`Serialize + DeserializeOwned + Send + Sync + 'static`).
- `action<A>(&self) -> ServerAction<A>` — a `ServerAction` that `notify()`s this
  invalidator on **success**, wiring the `Effect` internally so no caller writes
  it. `A` carries the bounds `ServerAction::<A>::new()` already requires (a
  `ServerFn` action).

`notify`/`track` are the low-level escape hatches; `resource`/`action` are the
primary surface.

### 2. Firing rule: success-gated (fixed)

`action::<A>()` fires only on `Some(Ok(_))` — after the mutation is confirmed
committed. Not on dispatch (which would race the mutation and read stale), and
not on failure (a rejected op changed nothing). This is a fixed rule, not a
per-call knob.

**Precondition:** success-gating is correct iff `Err` ⟹ no committed mutation.
That invariant is _assumed_ here and enforced separately — the ast-grep
heuristic gate ([#362](https://github.com/jaunder-org/jaunder/issues/362)) and
the structural storage-API rework
([#363](https://github.com/jaunder-org/jaunder/issues/363)). #359 does not
enforce it.

### 3. Scoping — single-scope primitive, newtype per context slot

The primitive is single-scope. Because leptos context is keyed by type, a bare
`Invalidator` in context is collision-prone. The rule:

- **Local scope** (action + resource in one component): a bare
  `let inv = Invalidator::new();`. Never enters context.
- **Cross-component scope**: a per-vertical semantic newtype, declared in that
  vertical's module with the `web::reactive::invalidator_scope!` macro (which
  emits the `#[derive(Clone, Copy)] struct <Name>(Invalidator)` +
  `Deref<Target = Invalidator>` boilerplate, forwarding docs/visibility) so
  `.action()`/`.resource()` work on it — provided/expected via context. The
  distinct type names the scope and prevents collision. The primitive never
  learns about any vertical.

No multi-scope / keying machinery is built.

### 4. Blast radius

#359 introduces `Invalidator` and converts **audiences only**. The ~15
co-located `.version()` pages are **not** migrated now — converting a
still-monolithic page from `.version()` to `Invalidator` removes no boilerplate
(one action, one resource) and would be churn. Each migrates when its vertical
is decomposed, under its own convergence issue.

## Acceptance criteria

- **AC1 — primitive exists.** `web::reactive::Invalidator` is a `Copy` type
  exposing `new`, `notify`, `track`, `resource`, `action`, with the semantics
  above; it lives in a new `web::reactive` module. `web::ui` gains no
  reactive-plumbing code.
- **AC2 — `action` is success-gated.** `action::<A>()` notifies only on
  `Some(Ok(_))`. A unit or integration test demonstrates: a failed action (e.g.
  duplicate-name `create_audience`) does **not** fire the invalidator; a
  successful one does.
- **AC3 — audiences converted, no bridge left.** `web::audiences` contains
  **no** hand-rolled `Revalidate` struct, **no** per-child `Effect` matching
  `Some(Ok(_))`, and **no** `RwSignal<u32>`/counter for revalidation. The
  audience list uses a context newtype (`AudienceList(Invalidator)`); each
  `MemberChecklist` uses a bare local `Invalidator`. No `.version()` appears in
  `web::audiences`.
- **AC4 — behavior preserved, each property pinned to a check** (not one bundled
  assertion):
  - **List refetch:** create / rename / delete refetch the audience list —
    covered by the existing web/storage audiences tests + `audiences.spec.ts`,
    which pass unchanged.
  - **Scoped invalidation — the defining property, and a NEW explicit test.** A
    membership toggle refetches only that audience's members and **does not
    refetch the audience list**. The existing `audiences.spec.ts`
    single-`list_audience_members`-fetch count covers the positive; add the
    **negative** assertion — a request count on `/api/list_my_audiences` that
    stays flat across a membership toggle. Without this, an implementation that
    collapses everything onto one shared `Invalidator` (over- invalidating)
    still satisfies AC1/AC3/AC5 and every currently-cited test.
  - **No remount:** the audience-list DOM is not rebuilt on a membership toggle
    — the element-handle `isConnected` guard already in `audiences.spec.ts`.
  - **Sticky retention:** a mutation shows no `Loading…` flash; the last
    resolved value is retained across the refetch (the #314 sticky-signal
    pattern is preserved). Covered by the existing spec's flash checks, which
    pass unchanged.
- **AC5 — the primitive is exercised by tests**, not only through audiences: a
  focused test for `Invalidator` (e.g. `resource` refetches after `notify`;
  `action` gates on success).
- **AC6 — the idiom is recorded.** An ADR (draft, numbered at ship) states
  `Invalidator` as the canonical revalidation idiom: success-gated, subsumes
  `.version()`, the local-vs-newtype scoping rule, and the `Err`⟹no-mutation
  precondition (→ #362/#363).
- **AC7 — gate green.** `cargo xtask validate` passes.

## Non-goals

- Migrating the ~15 co-located `.version()` pages (they migrate as they
  decompose).
- Any multi-scope / scope-keying machinery in the primitive.
- Enforcing `Err` ⟹ no-mutation (that is #362 / #363).
- Removing or changing `ServerAction::version()` itself.
