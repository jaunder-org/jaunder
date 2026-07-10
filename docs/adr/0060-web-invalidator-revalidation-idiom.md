# ADR-0060: web revalidation goes through the `Invalidator` primitive, not `action.version()`

- Status: proposed
- Date: 2026-07-10
- Issue: [#359](https://github.com/jaunder-org/jaunder/issues/359)

## Context

`web` had two ways to make a resource refetch after a mutation:

- **`action.version()`** — `server_resource(move || action.version().get(), …)`.
  Built into `ServerAction`, but it only works when the action and the resource
  it invalidates live in the **same component**.
- **A hand-rolled context signal + per-child `Effect`** (audiences, post-#314) —
  for the cross-component case `.version()` can't express.

The co-location `.version()` needs holds only for **monolithic** components.
Small, well-factored components are the discipline (ADR-0056), so as verticals
decompose the `.version()` precondition mostly evaporates: an action lands in a
child form while its resource lives in the parent, or N per-row actions
invalidate one shared list (a fan-in that can't source one resource off N
dynamically-created versions). In a decomposed world the cross-component case is
the norm, and `.version()` is the exception. The hand-rolled bridge that fills
the gap is repetitive boilerplate — a counter, a success-gated `Effect` per
action, and a `track()`/`get()` dance.

## Decision

Revalidation in `web` goes through one primitive,
**`web::reactive::Invalidator`**. A committed mutation `notify()`s an
`Invalidator`; resources that `track()` it refetch. `action.version()` is a
special case this subsumes, not a blessed alternative.

1. **`Invalidator` is a `Copy` newtype over a counter** (a resource source must
   be a changing value — leptos `Resource` is memoized on its source — so a
   notify-only `Trigger` won't do; the counter is the mechanism, as `version()`
   itself is). It encapsulates the counter behind `notify()` / `track()`, and
   offers `resource(fetch)` (a resource wired to it) and `action::<A>()` (a
   `ServerAction` that notifies it).

2. **`action::<A>()` is success-gated** — it fires only on `Some(Ok(_))`, after
   the mutation commits: never on dispatch (which races the write and reads
   stale), never on failure. This assumes **`Err` ⟹ no committed mutation**;
   that invariant is enforced separately (the ast-grep heuristic gate #362 and
   the structural storage-API rework #363), not by this ADR.

3. **Scoping is by type, not machinery.** The primitive is single-scope. A
   _local_ scope (action + resource in one component) is a bare `Invalidator`. A
   _cross-component_ scope is a per-vertical newtype, declared with the
   `invalidator_scope!` macro (which emits the `Deref`-to-`Invalidator` newtype
   boilerplate) and passed through context — the distinct type names the scope
   and prevents the type-keyed-context collision a bare `Invalidator` would
   risk. The primitive never learns about any vertical.

4. **Migration is per-vertical, not a big-bang.** Each vertical adopts
   `Invalidator` when it is decomposed (converting a still-monolithic
   `.version()` page removes no boilerplate). `ServerAction::version()` is not
   removed; it simply stops being reached for as components shrink.

## Consequences

- **One revalidation idiom, consistently applied.** A vertical uses
  `Invalidator` throughout; readers learn one pattern. The per-site counter +
  `Effect` boilerplate disappears (audiences: ~16 lines → ~4 in
  `MemberChecklist`).
- **Correct read-after-write by construction** — success-gating refetches only
  after the mutation lands, contingent on the `Err`⟹no-mutation precondition
  tracked in #362/#363.
- **The counter is encapsulated, not exposed.** No component writes
  `update(|n| *n += 1)` or a raw `RwSignal<u32>` for revalidation again.
- **Relationships.** Extends ADR-0056 (co-located Leptos-CSR verticals) — this
  is the reactive plumbing those decomposed verticals need. Depends on #362/#363
  for the correctness precondition of its success-gated rule.
