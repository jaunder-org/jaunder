# #1750: unified Theme editor workflow — proposed interface addendum

**APPROVED for implementation; not merge approval.** User's direct answer
**“approved”** resolves `JAU-1750-EDITOR-WORKFLOW-001` for the exact read/write
contracts and outline below. Lead acknowledgement:
`51e03446-a8b8-4315-a0ce-daa671184145`; Manager receipt:
`0d46ad21-67a4-4c70-aa58-789017273df7`. Approval provenance was added without
changing the contract. Pre-approval document SHA256 values:
`dd4e74a1c3ebf01578c4e746f4bd0a93cfe21ef7032f44b9838b905e3545515f` (this spec)
and `9f84abc069f78898c9f46c04daaaa4e13df5ae6b2583e89fd98f1a2aa0966190`
(outline).

This supersedes the narrower CSS-only proposal in
`2026-10-18-issue-1750-editor-preview.md`; that proposal remains withdrawn,
unanswered and unapproved. The approved installed-theme preview and Copy/name
decisions remain unchanged.

## Proposed editor contract

Use one normal editor model for Name, Style and Logo/Header. Existing Theme
Package/raw manifest/assets tools and destructive/management operations remain
separate, clearly labelled actions outside this form. Create starts a local
unsaved new-theme editor; opening or editing does not create a draft, select,
publish, or write. Existing Upload and per-item Copy keep their approved
private, unselected behavior.

Use typed inputs rather than caller-supplied package/HTML data:

- `ThemeEditorInput { name: String, stylesheet: String, logo: ThemeEditorPatch<ThemeBindingInput>, header: ThemeEditorPatch<ThemeBindingInput> }`.
- `ThemeEditorPatch<T> = Keep | Set(T)`; there is no ambiguous `Set(None)`.
  `Keep` preserves the exact stored binding and its original Media
  owner/provenance; it does not deserialize a stored binding into a
  caller-authorized replacement. For both Preview and Save,
  `Set(ThemeBindingInput::PackagedDefault)` restores the package/default
  behavior, `Set(ThemeBindingInput::ExplicitAbsent)` explicitly hides the image,
  and `Set(PackageAsset(path) | Media(reference))` uses existing validation and
  actor authorization. Preview and Save interpret every variant identically. A
  new target cannot use `Keep` and starts with explicit package/default choices.
  Header-pool controls stay in the separately labelled expert Images action.

Proposed read:
`preview_editor(scope: OwnershipScope, target: Option<ThemeId>, input: ThemeEditorInput) -> WebResult<ThemePreview>`,
generated `PreviewEditor`, `POST /api/themes/preview_editor` (`target: None` is
an unsaved new Theme). For existing custom Themes, package assets and other
immutable package context come from that authorized saved draft; for a new
Theme, use the existing empty-package defaults. Validate/compile with the
current package pipeline, resolve submitted image references only under existing
actor authorization, and render through the existing isolated Style Contract
preview. It does not write anything. Preview is explicit and represents all
submitted normal fields, not just CSS; it is not keystroke-driven.

Proposed write:
`save_editor(scope: OwnershipScope, target: Option<ThemeId>, input: ThemeEditorInput) -> WebResult<MutationOutcome<ThemeEditorSaveResult>>`,
generated `SaveEditor`, `POST /api/themes/save_editor`. Existing IDs update the
authorized custom draft's name/style/logo/header together; `None` creates one
private Theme and returns its assigned ID/name. `ThemeEditorSaveResult` contains
the saved ID/name and whether this operation advanced its public revision. Use
existing `owner(scope)` authorization, `private, no-store`, quotas, strict
explicit-name validation, Media ownership/locks, and one principal operation
permit. No arbitrary owner, package bytes, URI, or Media-owner grant is
accepted. Upload/Copy automatic naming is unchanged.

At the write boundary, determine whether that Theme is currently selected for
this same owner. If active, Save commits the edited draft and new published
revision so public appearance updates without another user action; selection
itself is unchanged. If inactive, Save updates the private draft/bindings only.
A pre-existing published revision remains eligible and unchanged: do not revoke
it, publish the edits, or change selection. A new target remains private and
unpublished until explicit Use/publication. Serialize/revalidate selection
against concurrent Use so the save is classified against the actual transaction
boundary. `CommitIndeterminate` is never reported as a confirmed save or applied
revision; reconcile once and never replay the mutation automatically.

## Existing boundary and required implementation constraint

Current source confirms the seam: `web/src/themes/api.rs::replace_css` calls
`ThemeStorage::replace_draft` in `WriteScope`; `replace_binding` and
`replace_pool` call `ThemeManager` methods, each of which acquires Media locks
and runs its own `WriteScope`; `publish` calls `ThemeAssetManager::publish`.
That manager installs immutable files under digest locks, then separately calls
`ThemeStorage::admit_publication` in its own `WriteScope`. The `ThemeStorage`
trait exposes transaction-scoped mutations for `replace_draft`, role/header-pool
updates, `admit_publication`, and `set_selection`, but the current
`ThemeAssetManager::publish` API owns its transaction and does not compose those
operations. `ThemeStorage::selection(owner)` is a separate pool read;
`set_selection` mutates inside a caller transaction but does not itself expose a
selection compare-and-set/lock contract. These are distinct operations today,
not an atomic normal-editor Save or proven concurrent-selection guard. Therefore
the contract above requires factoring the existing package validation,
Media-lock and immutable-file staging machinery into a coordinated editor
operation whose single database transaction writes all normal fields and, only
when still active, admits the publication. Install immutable bytes before making
them eligible; on known transaction failure, remove only files newly installed
by this attempt; preserve existing indeterminate-commit/reconciliation
semantics. Do not claim a cross-filesystem/database atomic transaction. If the
current storage/selection transaction seams cannot safely provide this
conditional active-publication behavior, stop and return that precise contract
blocker rather than silently split Save, publish inactive Themes, or change
selection.

No migration or new generic renderer/service is assumed. Keep exact per-trait
dependency injection; add only the narrowly required orchestration at the
existing Theme ownership/publication boundary. Existing installed built-in
preview remains read-only. Built-ins cannot be edited; copying one creates a
separate custom Theme under the already-approved Copy contract.

## Observable acceptance

- User/Site become bordered tab panels for independent settings; rows align
  Name, Style and Logo/Header controls. Save applies the complete normal form.
- Preview displays all current unsaved normal fields in the existing sandbox;
  owner, target and editor-generation guards prevent stale results replacing
  current output. Errors do not masquerade as current previews.
- Saving an active custom Theme updates its public revision without a second
  action, with no selection change. Saving an inactive existing Theme publishes
  no new revision and preserves any existing published revision and eligibility
  unchanged; it does not select or revoke anything. A new Theme remains private
  and unpublished until explicit Use/publication as required. No save-on-open or
  implicit activation.
- Package/asset authoring, import/export, publish/version operations and
  destructive management remain separate expert actions, not implied by normal
  Save. Preserve existing private Media authorization and package ownership.

## Approval boundary

The User approved the preview/save wire contracts and transaction design in this
spec and its linked outline under `JAU-1750-EDITOR-WORKFLOW-001`. This approval
authorizes implementation of that scope; it is not merge approval or approval
for any scope/contract change. This is not a layout reinterview or testing-seam
request.
