# #1750: installed-theme visual preview — proposed clarification

**APPROVED — JAU-1750-INSTALLED-PREVIEW-001, 2026-10-17.** Qualified Speaker
relay accepted the exact reviewed 5457-byte specification, SHA-256
`2d9b287c454131ab367047cc8176180e9194afded74622428025a180124f529a`. The human
requested a useful thumbnail for any installed theme, not built-in-only CSS
text. Installed packages currently supply no thumbnail field or PNG; repository
`preview.png` is not a package member. This proposes an isolated **rendered
visual preview**, not an existing packaged thumbnail or new image generator.
JAU-1750-BUILTIN-READ-001 stays withdrawn.

## Exact proposed contract

One new operation in `web/src/themes/api.rs`:
`preview_installed(scope: OwnershipScope, theme: PublicThemeSelection) -> WebResult<ThemePreview>`.
Use `#[macros::server(skip_all)]`: generated `web::themes::PreviewInstalled`,
`POST /api/themes/preview_installed`. The existing DTO remains
`ThemePreview { html: String, css: String }`. Existing
`preview(scope, theme_id)` retains its contract and browser coverage. This is a
new browser protocol operation despite renderer/DTO reuse.

- Authenticate and authorize through existing `owner(scope)`: Author is the
  authenticated User's own catalog; Site requires operator status, including for
  a built-in preview in that target. No caller-supplied User, URI or digest.
- Accept the existing typed BuiltIn(Studio/Terminal/Reader) or Custom(ThemeId)
  selection. “Any installed” is only the current authorized choice set; it does
  not expose other owners' private themes. Follow site theme is an inheritance
  setting, not a theme input; retain its guidance without inventing an effective
  private-theme preview.
- Return `private, no-store`; acquire the existing principal operation permit
  once per request. Custom uses the same saved-draft validation, compilation,
  draft image resolution and real page rendering as existing `preview`.
  Foreign/missing custom IDs remain masked not-found. It does not preview an
  unsaved editor buffer or require the draft to be published.
- BuiltIn reads its installed `SystemThemeRevision` and immutable compiled CSS,
  resolves its existing package image defaults for the same owner page route,
  and uses the same `PageSeed` construction and `app::render_shell`. No
  selection lookup/change, host-source recompilation, or synthetic screenshot
  fixture. Missing installed revision is not-found; storage, invalid metadata,
  file, integrity and UTF-8 failures retain typed sources behind the masked
  WebError boundary. No successful Studio or previous-preview fallback.

The missing byte-read seam is explicit, not claimed to exist: propose a narrow
`ThemeAssetManager::read_system_stylesheet(theme: Theme) -> Result<Option<(SystemThemeRevision, Vec<u8>)>, ThemeAssetError>`.
It reads the installed row, checks existing content eligibility, reads only its
digest-addressed stylesheet using the manager's existing
content-path/verification helpers, and returns that row with those bytes. It
performs no write, collection or compilation. The manager is already injected
into server-function context; no new storage trait, dependency bundle or wire
DTO is required. Factor the existing system-revision presentation conversion
narrowly for reuse, preserving public-projector behavior.

## Observable acceptance

1. One selected-theme preview, labelled as a saved-theme preview, appears in the
   shared workspace for built-ins and authorized custom themes. Clarify that
   custom preview uses the saved draft; unsaved edits require explicit Save
   before they appear. It is not a live activation or a PNG thumbnail.
2. Reuse `preview_document` and the titled `sandbox=""` iframe. Compiled CSS and
   Style Contract HTML enter only that isolated document, never the private page
   cascade. Keep script/navigation isolation and existing asset ownership.
3. Loading/error/unavailable states clear obsolete output; owner-plus-choice
   guards discard late results after either changes. Request only the selected
   preview, not every card; no eager fanout, automatic retries or cache
   framework.
4. Preserve independent User/Site settings, custom content editing and
   draft-only Save, disabled built-in Save, and explicit Use. Opening/previewing
   never selects, publishes or writes. No copy/overwrite or cross-owner catalog
   behavior.

Out: PNG production/storage, package/schema/manifest changes, new renderer or
fixture framework, public content-route changes, auth widening, migrations,
quotas, Save-autoapply, global fonts, #1791, or merge approval.

## Approval and lifecycle

The clarification and
[triggered outline](../plans/2026-10-17-issue-1750-installed-theme-preview.md)
were cold-reviewed and approved through Speaker. Qualified human answer: “#1750:
ah, we might want to log an issue to start packaging a standard thumbnail. But
yeah, reause the preview.” Lead acknowledged the approved scope. Canonical
approval:
`/home/mdorman/.pi/agent/manager-state/jaunder-7d43b9a1/history/issue-1750-installed-preview-approval.md`.
This checkpoint records approval, not implementation completion or merge
approval. The old CSS-text question remains withdrawn.

Existing frozen UXREV002 native/46 Chromium + 46 Firefox evidence proves neither
this interface nor new visual acceptance. Require affected current-source proof,
real server-function flow evidence, applicable normal gates/CI/coverage and
fresh committed-source After comparison; existing `6f44` gallery remains Before
and JAU1788MERGE001 withdrawn. Implementors/conformance reviewers consume this
in-flight clarification; update it if the contract changes, project enduring
behavior into current theme docs, and delete it after conformance.
