# Split Post UI by surface

## Outcome

The posts vertical keeps one ADR-0070 wasm-only `component` boundary while its
browser UI is organized into cohesive surface-owned leaves. Existing routes,
widgets, public paths, DOM, behavior, pure render coincidence, and host-tested
decisions remain unchanged.

## Load-bearing decisions

- Keep `web/src/posts/component.rs` as the existing wasm-gated `component`
  module, but reduce it to module documentation, private declarations, and
  explicit re-exports. `web/src/posts/mod.rs` retains its current paired
  `#[cfg(target_arch = "wasm32")] mod component` and `pub use component::{...}`
  wiring, so all `crate::posts::*` paths and the ADR-0070 boundary remain exact.
- Create private implementation leaves under `web/src/posts/component/`:
  `display.rs`, `audience.rs`, `composers.rs`, `permalink_editor.rs`,
  `drafts.rs`, `listings.rs`, and `support.rs`. No leaf contains a target-arch
  cfg; all inherit the enclosing component module's wasm-only gate.
- `display.rs` owns `PostDisplay`, `PostCard`, author/action/mutation helpers,
  confirmation handling, and publish/unpublish/delete feedback. Preserve the
  exact `render` projector twins, `inner_html`, classes, links, confirmation
  policy, callback precedence, navigation, and indeterminate-outcome messages.
- `audience.rs` owns `AudiencePicker`, named-audience loading/state views, and
  checkbox rows. Preserve Loading/Failed/Ready-empty distinctions, submit
  gating, selection semantics, `#audience-base`, and every label/for/id
  relationship.
- `composers.rs` owns `FormatToggle`, `ComposerFields`, create/compact/full and
  inline composers, `CreatePostPage`, and shared post-form controls used by the
  editor (`PostSaveActions`, slug/options/schedule/media controls). Preserve
  form fields, compact/full differences, state reset, publication intent,
  selectors, classes, flashes, and dispatch gates.
- `permalink_editor.rs` owns permalink first-paint adoption and `PostPage`, plus
  `EditPostPage`, `EditPostForm`, and edit-save outcome handling. Preserve typed
  route parsing, seeded Suspense fallback, post-id short-circuiting, exact
  schedule behavior, confirmed-publish redirect, and current unpublish
  navigation to `/drafts`.
- `drafts.rs` owns Draft and Scheduled management pages, lists, and rows.
  Preserve authenticated loading/error/empty/list states, selectors and rows,
  the current one-page behavior, and all publish/delete actions.
- `listings.rs` owns `UserTimelinePage`, `SiteTagPage`, and `UserTagPage` as the
  public post-listing route surfaces. Preserve typed route guards, seeded-page
  matching, query validation, timeline pagination wiring, discovery metadata,
  headings, empty text, and tag context.
- `support.rs` owns only the shared `on_settled_ok` reactive Effect adapter used
  by display, composer, and editor leaves. `canonical_username_display` stays
  private to `listings.rs`. Do not duplicate helpers or expose new paths.
- Keep all pure/model/parse/render decisions in existing ungated
  `compose_state.rs`, `edit_state.rs`, `page_state.rs`, `parse.rs`, and
  `render.rs`, with their existing host tests unchanged. Do not move pure logic
  behind the wasm gate or add host stubs.
- Update current `docs/ARCHITECTURE.md` source citations and
  `docs/web-style-guide.md` guidance to describe a large wasm-only
  `component.rs` facade over cohesive private leaves. Leave ADR-0070 and
  historical records unchanged because its file-level cfg and host-test
  decisions are preserved.
- Coordination issues are behavior constraints, not prerequisites: preserve
  completed #908 shared save controls and #899 named-audience state; do not fix
  open #907 Field validity debt, decide #896 create-page discoverability, add
  #799 draft pagination, or change #783 unpublish destination.

## Acceptance

- `component.rs` contains only documentation, private module declarations, and
  explicit re-exports; every implementation leaf has one named responsibility.
- `posts/mod.rs` cfg wiring and every existing `crate::posts::*` export remain
  unchanged; app routes, cockpit `InlineComposer`, and timeline `PostCard`
  callers require no migration, alias, or new public export.
- Existing rendered HTML, CSS classes, selectors, accessibility labels,
  first-paint/Suspense behavior, form values and gates, mutations, callbacks,
  redirects, error text, and listing states are observably unchanged.
- Existing host tests remain in the five ungated logic/render leaves and retain
  their names/assertions. The split introduces no wasm-only pure decision that
  needs a new test.
- Focused browser execution proves the relocated posts surfaces still mount and
  behave through the existing `end2end/tests/posts.spec.ts` contract, including
  its visual/accessibility state; the test-enabled repository gate passes.
- Current architecture and style guidance point at and describe the new source
  layout accurately.

## Boundaries

- No Post domain, wire API, route, validation, audience, publication, schedule,
  mutation, pagination, navigation, rendering, styling, accessibility, or
  product-affordance change.
- No selector/baseline update, new e2e scenario, new public helper,
  compatibility shim, lint suppression, cfg inside a leaf, or unrelated
  stale-path cleanup.
- No new ADR: this refactor preserves ADR-0041, ADR-0070, ADR-0083, ADR-0113,
  and ADR-0128 rather than changing their decisions.
