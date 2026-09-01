# Split Post UI implementation outline

**Execution:** `jaunder-iterate`, delegating leaf ownership through
`jaunder-dispatch`.

## Trigger and scope

The approved issue #974 spec preserves behavior, but extracting seven wasm UI
leaves in parallel requires fixed private interfaces, one integration owner, and
explicit ADR-0070 wiring. No product behavior or pure host logic changes.

In scope: reduce `web/src/posts/component.rs` to its existing component-module
facade; create seven private leaves under `web/src/posts/component/`; update
current architecture/style source guidance. Out of scope: APIs/routes/domain
logic, existing host-tested state/parse/render leaves, e2e selectors/baselines,
ADR history, and the coordinated issues' product changes.

## Tasks

- [x] **Extract shared Effect support** — create `component/support.rs` with
      only the existing `on_settled_ok` adapter.
- [x] **Extract Post display/cards** — create `component/display.rs` with
      `PostDisplay`, `PostCard`, and their action/mutation helpers.
- [x] **Extract audience picker** — create `component/audience.rs` with named
      audience loading and picker components.
- [x] **Extract composer surfaces** — create `component/composers.rs` with
      create and inline composers plus shared composer/editor controls.
- [x] **Extract permalink/editor surfaces** — create
      `component/permalink_editor.rs` with permalink first paint/page and
      editor.
- [x] **Extract unpublished management** — create `component/drafts.rs` with
      Draft and Scheduled pages/lists/rows.
- [x] **Extract public listing pages** — create `component/listings.rs` with
      user timeline and site/user tag pages; update only current
      architecture/style guidance that describes the moved layout.
- [x] **Assemble and verify the facade** — replace the monolith body with
      private declarations and explicit re-exports, resolve extraction-only
      imports and visibility, then prove host, wasm, and actual browser
      behavior.

## Stable contracts

- Parallel leaf tasks create only their named new file; the listings task also
  owns `docs/ARCHITECTURE.md` and `docs/web-style-guide.md`. No leaf task edits
  or deletes `component.rs`, `posts/mod.rs`, the spec/outline, tests, or another
  leaf. Integration alone rewrites the facade.
- `support.rs` exposes `on_settled_ok` as `pub(super)`; `display.rs`,
  `composers.rs`, and `permalink_editor.rs` import it directly. No duplicate
  reactive settlement helper.
- `display.rs` exposes `PostDisplay` and `PostCard` to the facade. Other display
  helpers stay private.
- `audience.rs` exposes `AudiencePicker` to the facade. It exposes
  `load_named_audiences` and `AudiencePickerWithState` as `pub(super)` for
  composer/editor siblings; lower option/row/checkbox helpers remain private.
- `composers.rs` exposes `ComposerFields`, `PostCreateForm`, `InlineComposer`,
  and `CreatePostPage` to the facade. `FormatToggle` remains internal. The
  editor imports sibling-visible `PostSaveActions`, `SlugOverrideInput`,
  `ComposeOptions`, `ScheduleControl`, and `MediaSection`; these are not facade
  exports.
- `permalink_editor.rs` exposes `PostPage` and `EditPostPage` to the facade and
  imports shared form controls from `super::composers`, display widgets from
  `super::display`, and settlement support from `super::support`.
- `drafts.rs` exposes `DraftsPage` and `ScheduledPage` to the facade; list/row
  helpers remain private.
- `listings.rs` exposes `UserTimelinePage`, `SiteTagPage`, and `UserTagPage` to
  the facade. `canonical_username_display` is private here.
- `component.rs` contains only module docs, private declarations, and explicit
  re-exports of the exact current effective component surface: AudiencePicker,
  ComposerFields, CreatePostPage, DraftsPage, EditPostPage, InlineComposer,
  PostCard, PostCreateForm, PostDisplay, PostPage, ScheduledPage, SiteTagPage,
  UserTagPage, and UserTimelinePage. It does not re-export `FormatToggle` or any
  new helper. `posts/mod.rs` remains byte-for-byte unchanged.
- All seven leaves inherit the enclosing component module's existing wasm cfg;
  no new target-arch cfg appears. Existing ungated `compose_state`,
  `edit_state`, `page_state`, `parse`, and `render` files/tests remain
  unchanged.
- Preserve code and markup during extraction: no renames, reordered view trees,
  selector/class/text changes, callback rewrites, or helper redesign.

## Risk checks and verification

- Confirm the component facade's effective export set equals `posts/mod.rs` and
  all app/cockpit/timeline callers compile without migration.
- Confirm no target-arch cfg enters a leaf and projector/CSR `inner_html`
  continues to call the same host-tested render twins.
- Confirm every existing test in `compose_state`, `edit_state`, `page_state`,
  `parse`, and `render` remains unchanged and passes through the focused web
  test lane; then run one test-enabled `cargo xtask check` for host/wasm/clippy
  integration.
- Exercise `end2end/tests/posts.spec.ts` through the focused e2e-local lane to
  cover relocated create/audience/permalink/editor/draft/display surfaces.
- Browser-drive the running application and visually inspect a relocated Post
  display/listing surface and the create/auth route outcome; preserve existing
  visual/accessibility baselines rather than adding or updating them.
- The commit hook owns the final precommit gate. No lint suppression is expected
  or approved.
