# Replace Revalidation Counters with Invalidator

## Outcome

`HomePage`, `CockpitPage`, and `MediaPage` use the established reactive
invalidation seam instead of page-local revision counters, while preserving
every current refresh trigger and suppression rule.

## Load-bearing decisions

- Give Home and Cockpit one local `Invalidator` each; their timeline resources
  use `client::reactive::resource(invalidator.track, fetch)`. Delete and
  unpublish settlements retain their current notifications.
- Change `InlineComposer`'s `on_publish: WriteSignal<u32>` prop to a
  notification callback that it invokes only when the existing settlement
  classification says the created post was published. Confirmed and
  commit-indeterminate draft saves remain suppressed. This is the only posts
  seam changed; work owned by #974 remains untouched.
- Give Media one shared `Invalidator` for its usage and media-list resources.
  Upload and delete outcomes notify that same invalidator so both resources
  continue to refresh together.
- Add `client::reactive::action_if`, an outcome-filtered sibling of `action`. It
  creates the server action and notifies only for `Some(Ok(output))` values
  accepted by a caller-supplied predicate.
- Keep the existing `action` interface and unconditional-success semantics
  unchanged for audience mutations.
- Replace `MediaDeletion { deleted, referenced_in_posts }` with an exhaustive
  `MediaDeletion::Deleted | MediaDeletion::RefusedReferenced { post_ids }`
  result.
- Media delete uses `action_if` with an exhaustive predicate: confirmed deletion
  and any commit-indeterminate outcome invalidate; confirmed referenced refusal
  does not.
- Media delete rendering exhaustively matches the same result type: deletion
  confirmation, referenced-refusal explanation and force-delete affordance,
  commit-indeterminate warning, or transport/server error.
- The `MediaDeletion` serialized shape intentionally changes with its state
  model. It is an internal server-function protocol consumed and migrated
  atomically by this web application; visible UI behavior remains unchanged.
- Preserve ADR-0060's local `Invalidator` ownership, success-gated action
  effects, and shared-resource invalidation rules.
- Preserve ADR-0164's rule that confirmed and commit-indeterminate mutations
  revalidate while failed or known-nonmutating outcomes do not.

## Acceptance

- No raw revalidation revision signal remains in Home, Cockpit, or Media.
- Home and Cockpit refresh after the same published-create, delete, and
  unpublish settlements as before; confirmed and commit-indeterminate draft
  saves do not refresh.
- Media upload confirmation and commit-indeterminate settlement refresh both
  usage and list resources; failed upload does not.
- Confirmed media deletion and commit-indeterminate deletion refresh both
  resources.
- Referenced refusal and delete error do not trigger either Media resource
  request.
- Existing audience `client::reactive::action` callers retain their interfaces
  and behavior.
- Focused host tests cover the typed deletion states and Media's pure
  invalidation predicate. Extend the media browser tests to count both list and
  usage requests and prove confirmed deletion refreshes both while referenced
  refusal and delete error refresh neither; those tests exercise the wasm-only
  `action_if` wiring and retain rendered-outcome assertions.
- Affected focused tests pass and `cargo xtask check` passes.

## Boundaries

- Do not change mutation timing, resource fetch order, loading/error
  presentation, force-delete behavior, or persistence semantics.
- Do not migrate posts-owned reactive work beyond the narrow `InlineComposer`
  notification prop required to remove Home/Cockpit counters.
- Do not absorb #974 or #363.
- No domain glossary or ADR change is required; the work applies existing
  invalidation and mutation-outcome decisions.
