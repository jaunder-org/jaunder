# #1750: unified Theme editor workflow — implementation outline

> **APPROVED for implementation; not merge approval.** User's direct answer
> **“approved”** resolves `JAU-1750-EDITOR-WORKFLOW-001` for this outline and
> its linked spec. Lead acknowledgement / Manager receipt:
> `0d46ad21-67a4-4c70-aa58-789017273df7`. Pre-approval SHA256: spec
> `dd4e74a1c3ebf01578c4e746f4bd0a93cfe21ef7032f44b9838b905e3545515f`; outline
> `9f84abc069f78898c9f46c04daaaa4e13df5ae6b2583e89fd98f1a2aa0966190`. Triggers:
> browser-callable read/write interfaces, coordinated publication, Media
> authorization and selection concurrency. Execute with **jaunder-iterate** /
> **jaunder-dispatch**. The linked
> [addendum](../specs/2026-10-18-issue-1750-theme-editor-workflow.md) is
> authoritative. Earlier installed-preview and Copy/name approvals remain
> intact.

## Approved scope and contracts

In: one normal unsaved editor for Name, Style and Logo/Header; preview all its
current values; one Save for all. Active custom Theme Save publishes the new
revision without changing selection. Inactive existing Themes publish no new
revision and retain any existing published revision unchanged; new Themes remain
private and unpublished. Package/raw asset tools and destructive management
remain separate.

Proposed
`preview_editor(scope, target: Option<ThemeId>, input: ThemeEditorInput) -> WebResult<ThemePreview>`
(`PreviewEditor`, `POST /api/themes/preview_editor`) and
`save_editor(scope, target: Option<ThemeId>, input: ThemeEditorInput) -> WebResult<MutationOutcome<ThemeEditorSaveResult>>`
(`SaveEditor`, `POST /api/themes/save_editor`). Input is
`{name, stylesheet, logo, header}`; role patches are
`Keep | Set(ThemeBindingInput)`, preserving stored Media provenance on Keep.
`PackagedDefault` restores the default, `ExplicitAbsent` explicitly hides the
image even when a package default exists, and PackageAsset/Media choices use
existing authorization; Preview and Save must interpret these identically. Reuse
existing preview DTO, validators/compiler, renderer, Media authorization,
WriteScope and immutable-file lifecycle. Existing upload/copy naming and
preview-installed APIs remain unchanged. These contracts are approved under
`JAU-1750-EDITOR-WORKFLOW-001`; this does not approve merging.

## Task outline

- [ ] **1. Prove a current-form preview and source-bound draft model.**
  - Contract: capture Name, stylesheet, Logo and Header in one typed editor
    input. For existing targets, `Keep` preserves exact stored binding
    provenance; `Set(PackagedDefault)` restores defaults, `Set(ExplicitAbsent)`
    hides the image, and `Set(PackageAsset | Media)` follows existing
    authorization. Existing custom targets load authorized saved package
    context; a new target uses existing empty-package defaults and cannot
    request `Keep`. Preview substitutes all current form values in memory and
    uses the existing validator/compiler, `render_shell`, and sandbox escaping.
    No DB or filesystem write. Preview is explicit; stale
    owner/target/generation results cannot replace current output.
  - Verification: minimum focused cases establish current unsaved CSS and image
    choices appear before Save, package/manifest contexts are not overwritten,
    private/foreign/anonymous access fails closed, and no Save/selection/
    publication occurs. Preserve installed built-in read-only preview. Produce a
    source-bound first visual sample for review before broad test or gate
    campaigns, per the user's requested visual-first cadence.
- [ ] **2. Make normal Save one truthful owner-bound operation.**
  - Contract: Save existing target's Name/Style/Logo/Header together, or create
    one private target from a local unsaved-new model. Preserve strict explicit
    name validation, quotas, and existing Media provenance/authorization.
    Serialize and revalidate selection with the write: publish the resulting
    revision iff the same target remains selected for this owner; never change
    selection. Otherwise persist private draft only. A pre-existing published
    revision for an inactive Theme remains eligible and unchanged. A new Theme
    remains unpublished. One DB transaction covers normal DB fields and any
    active publication admission.
  - Implementation boundary: refactor `ThemeAssetManager::publish`'s immutable
    file staging/admission and `ThemeManager`'s Media locks/binding writes into
    a focused coordinated operation. Install immutable bytes before eligibility;
    clean only newly installed files after known rollback; preserve
    indeterminate commit reconciliation and never replay automatically. If a
    coherent selection-guarded transaction is unsupported, stop and return the
    precise blocker instead of claiming atomic Save or splitting user-visible
    actions.
  - Verification: focused backend-parametric tests (`#[apply(backends)]`) for
    complete multi-field Save, new/private creation, active revision update,
    inactive draft update with no new revision and any old published revision
    preserved unchanged, no-selection-change, concurrent Use/selection
    revalidation, foreign/missing owner, unauthorized Media, quota/package
    errors, known rollback and commit-acknowledgement loss. Confirm existing
    Upload/Copy/explicit Use behavior remains intact.
- [ ] **3. Integrate the workspace and complete final delivery proof.**
  - Contract: bordered User/Site tab panels; aligned Name, Style and Logo/Header
    rows; one normal Save; clear active versus inactive outcome; separate
    Images/Theme Package/Manage Theme expert actions. No side-effecting
    field-level saves. Preserve isolated preview, permanent per-item Copy,
    whole-tile Use and accessibility.
  - Verification: sample-first review, then focused browser/accessibility and
    backend proofs. After approved visual direction, complete relevant consumer
    regression, empirical endpoint-flow coverage from authoritative capture,
    review, normal commit/boundary validation and matched source-qualified
    Before/After evidence. Historical 62+62 and revised draft samples are not
    certification; no baseline/tolerance/population waiver.

## Risk checks and approval handoff

- Existing `replace_css`, binding writes and `ThemeAssetManager::publish` use
  distinct write paths; they do not currently implement the proposed unified
  Save. Confirm the exact transaction/selection-lock seam before coding.
- Preserve ADR-0016 exact DI, ADR-0184 package isolation and Media ownership,
  existing CSS validation and sandbox rendering, strict naming and indeterminate
  commit semantics. Never describe staged files plus DB as cross-store atomic.
- Keep copy/import/export/publish/delete/version management outside normal Save.
  No user layout reinterview or test-seam ceremony; the precise new interface
  and conditional-publication contract is the remaining gate.
- Lead owns source-grounded cold review and one direct User approval packet.
  This text authorizes neither interface implementation nor merging.
