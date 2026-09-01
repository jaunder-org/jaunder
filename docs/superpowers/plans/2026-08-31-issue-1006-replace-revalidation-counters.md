# Replace Revalidation Counters Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for the independent
> slices. This outline exists because `MediaDeletion` changes a serialized
> server-function protocol and `action_if` establishes a cross-crate reactive
> API.

## Scope

In:

- Migrate Home, Cockpit, and Media resource invalidation under the approved
  spec.
- Atomically migrate every producer and consumer of the `MediaDeletion` wire
  type.
- Add outcome-filtered action construction without changing existing `action`
  callers.
- Strengthen Media browser request-count assertions for both resources.

Out:

- Posts behavior beyond the `InlineComposer` notification prop.
- Audience mutation behavior or callsite migration.
- Work owned by #974 or #363.

## Task outline

- [x] Media typed outcome and shared invalidation
  - Contract:
    `MediaDeletion::Deleted | MediaDeletion::RefusedReferenced { post_ids }`;
    `client::reactive::action_if<A>(notify, predicate)` evaluates predicates
    only for `Some(Ok(&A::Output))`; existing `action<A>(notify)` remains
    unchanged.
  - Ownership: `client/src/reactive.rs`, `web/src/media/**`,
    `server/tests/web/web_media.rs`, and `end2end/tests/media.spec.ts`.
  - Verification: focused Media host tests prove exhaustive state and predicate
    behavior; Media browser coverage counts list and usage requests across
    confirmed deletion, referenced refusal, and delete error.
- [x] Home and Cockpit invalidator migration
  - Contract: each page owns one local `Invalidator`; `InlineComposer` accepts a
    notification callback and invokes it only for published settlements,
    preserving draft suppression.
  - Ownership: `web/src/home/component.rs`, `web/src/cockpit/component.rs`, and
    the narrow `InlineComposer` seam plus its direct tests in
    `web/src/posts/**`.
  - Verification: focused Home/Cockpit/posts host tests preserve published,
    draft, delete, and unpublish settlement behavior.

## Ordering and integration

- The two tasks are independent and may run in parallel because their file
  ownership is disjoint.
- Integrate both before the repository check gate; no compatibility shim or
  transitional wire representation is permitted.
- `jaunder-commit` owns the commit boundary and precommit gate after focused
  verification.

## Risk checks

- Every `MediaDeletion` construction, match, serialization test, and rendered
  consumer migrates atomically; no boolean-state compatibility path remains.
- `action_if` ignores `None` and `Err`, predicates `Ok` outputs, and notifies
  exactly once only when accepted.
- Confirmed referenced refusal and delete error issue zero list and usage
  refetches; confirmed deletion refetches both.
- Confirmed and commit-indeterminate draft saves remain suppressed; published
  settlements retain notification.
- Media upload confirmed and commit-indeterminate paths still notify the one
  shared invalidator; upload failure does not.
- Resource fetch order, force-delete behavior, mutation timing, and visible
  outcomes remain unchanged.
