# Issue #908: Shared post save actions

## Outcome

The compact composer, full composer, and editor render their save controls
through one module-private component without changing appearance, selectors, or
behavior.

## Load-bearing decisions

- The shared component is named `PostSaveActions`; its name reflects both
  creation and editing rather than retaining the editor-specific name.
- It owns the existing publication-state branch: drafts render “Save draft” and
  “Publish”; scheduled and live Posts render the lone “Save” control.
- Both creation forms pass draft publication state and their existing shared
  `(disabled, on_save)` contract.
- Each caller retains its current toolbar or aside wrapper. The shared component
  emits only the buttons and does not own layout.
- Button text, `j-btn` / `is-primary` classes, `type="button"`,
  `name="publish"`, boolean values, disabled affordance, and callback arguments
  remain unchanged.
- Existing CSS and end-to-end selectors remain unchanged.
- This local markup refactor introduces no architectural decision or domain
  vocabulary.

## Acceptance

- All three forms use `PostSaveActions`; no duplicate save-button pair or
  `EditSaveActions` symbol remains.
- Compact and full composers still dispatch `false` for “Save draft” and `true`
  for “Publish”.
- The editor still renders the draft pair and the scheduled/live lone “Save”
  branch.
- Existing wrapper classes and layout markup remain at their original call
  sites.
- `SEL.publishButton("false")` and `SEL.publishButton("true")` resolve and
  retain their enabled/disabled behavior on both composers and the draft editor.
  Scheduled/live editors expose only the `true`-valued “Save” control; the
  `false` selector remains absent.
- `cargo xtask validate` passes.

## Boundaries

- Submission gating, payload construction, publication transitions, and server
  APIs are unchanged.
- No CSS, selector, or unrelated composer control changes are included.
