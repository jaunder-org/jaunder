# #1750: installed-theme visual preview implementation outline

> **APPROVED — JAU-1750-INSTALLED-PREVIEW-001, 2026-10-17.** Qualified Speaker
> relay accepted the exact reviewed 6370-byte outline, SHA-256
> `0b78279e1bc78bb70727b9e9f63d7aae283b50e734ffc7c01465f76ccd98afd6`. Trigger:
> new browser-callable generic preview read, plus explicit acceptance of
> rendered preview instead of unavailable installed PNG thumbnails. Execute
> after approval with **jaunder-iterate**, using **jaunder-dispatch** for
> bounded work. The
> [clarification](../specs/2026-10-17-issue-1750-installed-theme-preview.md)
> owns the contract; withdrawn BuiltinRead001 is not authority.

## Scope and key contract

In: a selected-theme rendered visual preview for built-ins and owner-visible
saved custom themes, using the existing semantic renderer and sandbox. Keep the
lower custom editor and independent User/Site settings.

New
`preview_installed(scope: OwnershipScope, theme: PublicThemeSelection) -> WebResult<ThemePreview>`
in `web/src/themes/api.rs`, `#[macros::server(skip_all)]`, generated
`PreviewInstalled`, `POST /api/themes/preview_installed`. Reuse `owner(scope)`,
private/no-store, one operation permit and existing `{html, css}` DTO. Site
requires operator status; Author uses the authenticated User. Existing
`preview(scope, theme_id)` stays unchanged on the wire.

Out: thumbnail generation, new renderer/fixture, package/schema changes, new
storage traits or DTOs, selection/publication writes, anonymous reads, catalog
sharing, copy, Save-autoapply, global fonts, #1791 and merge approval.

## Task outline

- [ ] **1. Complete a built-in/custom selected-preview tracer bullet.**
  - Contract: factor existing `preview`'s saved-draft path and real owner
    `PageSeed`/route construction without changing its observable contract.
    Generic custom requests reuse that behavior. Built-ins use installed
    metadata, existing system image-default conversion and the same
    `app::render_shell`; no selecting a theme to resolve it.
  - Required narrow new read seam: add the manager method named in the spec,
    reading one installed revision plus eligible, verified immutable CSS bytes.
    Reuse `ThemeAssetManager`'s existing path/digest primitives and existing
    request-context injection. Factor system presentation conversion in
    `storage/src/public_theme.rs`; preserve public resolution semantics. Use
    only existing exact contexts: ThemeStorage, ThemeAssetManager, operation
    coordinator, SiteConfigStorage, PostStorage and UserStorage, plus existing
    auth dependencies. No StorageFactory or new dependency bundle crosses a
    seam.
  - Verification: endpoint integration in `server/tests/web/themes.rs` through
    `post_server_fn`/`make_app`; exercise Studio and saved custom CSS through a
    real selected-preview browser flow. Prove no selection/publication/draft
    write and keep the legacy preview flow exercised. Update vertical exports,
    explicit registrar and generated-type wire assertions. DB-backed endpoint
    and manager tests use `#[apply(backends)]` for SQLite/PostgreSQL parity;
    focused endpoint lane:
    `devtool run -- cargo xtask test-local -- -p jaunder web::themes`.
- [ ] **2. Complete isolation, failure and owner/choice coverage.**
  - Contract: all three built-ins and saved custom drafts render through
    existing `preview_document`/titled `sandbox=""` iframe. Label saved-draft
    versus unsaved edits accurately. Clear obsolete documents and discard late
    results by owner-plus-choice; one selected read, no card-wide fanout. Follow
    site remains inheritance guidance, not an installed-theme input.
  - Verification: focused red/green for all built-ins, Author/Site operator
    access, anonymous and nonoperator Site rejection, foreign/missing custom
    masking, unpublished custom draft, missing installed row, storage/file/
    integrity/metadata failures, and delayed responses across choice/owner
    switches. Host-test pure stale-result logic; browser proofs show honest
    loading/error states, no private-page CSS effects, preserved sandbox,
    disabled built-in Save and unchanged custom Save/explicit Use. No real
    cross-origin/executable injection or new image-capture harness is needed.
- [ ] **3. Integrate final-source proof and enduring documentation.**
  - Contract: update `docs/themes.md` and `docs/flows/theme-management.md` with
    rendered preview versus repository thumbnail and saved-draft semantics.
  - Verification: rerun affected consumers and the existing UXREV002
    four-consumer regression population in Chromium/Firefox on the final source.
    The retained 46+46 is historical input evidence, not a new-preview verdict.
    Obtain authoritative SQLite/Chromium flow evidence, then use
    `devtool run -- cargo xtask server-fn-coverage regenerate` and
    `devtool run -- cargo xtask server-fn-coverage verify`; never hand-edit the
    ledger. Normal hook-backed commits, current CI/coverage and applicable
    hermetic proof follow `CONTRIBUTING.md`. Obtain newly committed-source After
    and matched Before comparison for human UX acceptance separately.

## Risk checks and approval handoff

- Existing preview reads only custom IDs; adding a typed built-in branch is a
  new interface, not unchanged reuse. Installed metadata contains digests, not
  CSS bytes; the proposed manager read is necessary plumbing, not an existing
  read capability or permission for a general content/file service.
- Preserve ADR-0184 private/public isolation, ADR-0016 exact dependencies,
  ADR-0082 derived wire naming, ADR-0066 registrar/wire completeness and
  ADR-0081 empirical flow coverage. Typed errors stay errors, never successful
  absence or fallback. No suppression, baseline, tolerance, retry or population
  waiver follows from this outline.
- Custom image reads retain owner authorization; built-ins resolve only system
  defaults and public immutable assets. The preview remains saved-draft content,
  not a promise of published appearance or unsaved-buffer preview.
- Qualified Speaker approval and Lead scope acknowledgment are recorded in the
  linked clarification under JAU-1750-INSTALLED-PREVIEW-001; no repeat approval
  question is pending. Execute tasks 1 → 2 → 3 with one writer, not competing
  API/UI writers. This checkpoint does not complete any task or approve merge;
  Lead owns final acceptance and JAU1788MERGE001 remains withdrawn.

Implementation/conformance consumes this temporary outline; maintain its task
state, project enduring contracts into current docs and delete it after
conformance under the normal artifact lifecycle.
