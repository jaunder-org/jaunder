# Plan — Issue #359: the `Invalidator` revalidation primitive

Spec of record: `docs/superpowers/specs/2026-07-10-issue-359-invalidator.md`.
Decision record: `docs/adr/drafts/web-invalidator-revalidation-idiom.md`. The
`Err`⟹no-mutation precondition is split out to #362 (ast-grep gate) / #363
(storage-API rework) — not this cycle.

## Tasks

- [ ] **1. `web::reactive::Invalidator`.** New `web/src/reactive/mod.rs`:
      `Invalidator`, a `Copy` newtype over `RwSignal<u32>`, with `new` /
      `notify` (bump) / `track` (`-> u32`) /
      `resource<T>(impl Fn() -> Fut) -> Resource<T>` (nullary fetcher, wraps
      `crate::server_resource(move || self.track(), move |_| fetch())`) /
      `action<A>()     -> ServerAction<A>` (creates the action + a success-gated
      `Effect` that `notify`s on `Some(Ok(_))`). `pub mod reactive;` in
      `lib.rs`. → **AC1**.
- [ ] **2. Tests for the primitive.** (a) Host unit tests under an `Owner` for
      the counter mechanics (`notify` bumps `track`'s value). (b)
      Success-gating: create `inv.action::<A>()` and drive `action.value()` to
      `Some(Ok(_))` then `Some(Err(_))`, asserting the invalidator bumps
      **only** on `Ok` — if `Action::value()` is a writable signal (confirm at
      impl); otherwise pin gating via the audiences integration route (a
      rejected duplicate-name `create_audience` must not refetch the list, a
      successful one must). Refetch-on-`notify` is exercised end-to-end by the
      audiences suite + Task 3. → **AC2, AC5**.
- [ ] **3. Scoped-invalidation guard test — written first, passes on current
      code.** Extend `end2end/tests/audiences.spec.ts`: add a request count on
      `/api/list_my_audiences` and assert it stays **flat** across a membership
      toggle (the negative that a single-shared-invalidator implementation would
      fail). It passes on today's members-local-trigger code, so it guards the
      Task-4 conversion against over-invalidation. → **AC4 (scoped)**.
- [ ] **4a. Convert the list scope.** In `web/src/audiences/mod.rs`, replace
      `Revalidate { list: RwSignal<u32> }` with
      `struct AudienceList(Invalidator);` + `Deref<Target = Invalidator>`.
      `AudiencesPage` provides it and sources the list via
      `list.resource(|| list_my_audiences())`; `CreateAudienceForm` /
      `AudienceHeader` use `expect_context::<AudienceList>().action::<…>()`,
      deleting their success `Effect`s. Verify: the Task-3 guard + list-refetch
      tests stay green; no `Revalidate` struct or list-scope `Some(Ok(_))`
      `Effect` remains. → **AC3 (list), AC4 (list refetch)**.
- [ ] **4b. Convert the members scope.** Each `MemberChecklist`: a bare local
      `Invalidator`; add/remove → `.action::<…>()`; members resource →
      `.resource(|| list_audience_members(id))` — deleting the local
      `RwSignal<u32>` and its two `Effect`s. Sticky-signal retention
      (resource→signal copy `Effect`s) kept as-is. Verify: a membership toggle
      refetches only that audience's members (guard green); **no** revalidation
      `RwSignal<u32>`, `Some(Ok(_))` `Effect`, or `.version()` remains anywhere
      in `web::audiences`. → **AC3 (members), AC4 (scoped / no-remount /
      sticky)**.
- [ ] **5. Green the gate + confirm the ADR.** All existing audiences
      web/storage tests + the Task-3 guard + the Task-2 tests pass;
      `cargo xtask validate`. Confirm the ADR draft states all four points —
      success-gating, `.version()` subsumption, the scoping rule, and the
      `Err`⟹no-mutation hand-off to #362/#363. → **AC4, AC6, AC7**.

## Acceptance (from the spec)

AC1 primitive (Task 1) · AC2 success-gated (Task 2) · AC3 audiences converted,
no bridge (Tasks 4a/4b) · AC4 behavior preserved incl. the scoped negative test
(Tasks 3, 4a, 4b, 5) · AC5 primitive tested (Task 2) · AC6 ADR draft written +
verified (Task 5) · AC7 gate green (Task 5).
