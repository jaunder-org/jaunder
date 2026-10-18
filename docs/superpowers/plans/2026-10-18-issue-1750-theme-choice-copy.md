# #1750: theme choices, Copy and automatic names implementation outline

> **APPROVED for implementation — decision JAU-1750-COPY-NAMES-001.** Triggers:
> browser mutation interface, storage concurrency and Media privacy. Execute
> with **jaunder-iterate**, using **jaunder-dispatch** for bounded work. The
> linked [addendum](../specs/2026-10-18-issue-1750-theme-choice-copy.md) is
> authoritative. Existing installed-preview approval remains valid;
> BuiltinCSS001 is withdrawn.
>
> Approval provenance: the user answered **“yes”** to the Lead's request for
> implementation of the reviewed Copy/numbering plan; the request states that
> approval covers implementation and this plan, not merging. Lead
> acknowledgement: `51e03446-a8b8-4315-a0ce-daa671184145`. Reviewed source
> hashes: spec
> `afcb1c71584f8bbf25a10aac9e51e81fadd335633629a60f8dcd19e31f1880fd`, outline
> `747a038c1f75c396cebe89bd4b65047fe984001162e839b8ace1d516c965e437`.

## Scope and key contracts

In: same-library saved-theme Copy, automatic Upload/Copy catalog names, and the
requested tile/action presentation. Preserve independent User/Site settings,
immutable built-in originals, custom editing, explicit Save/Use and approved
isolated preview reuse.

New
`copy_theme(scope: OwnershipScope, source: PublicThemeSelection) -> WebResult<MutationOutcome<CatalogEntry>>`,
generated `web::themes::CopyTheme`, `POST /api/themes/copy_theme`. Author uses
the current User; Site requires operator. Follow site is not a source.
Explicit-name imports, strict create and rename retain existing outcomes except
participation in serialization.

Out: cross-owner/library copying, Media entitlement changes, PNG production, new
package format/renderer, automatic activation, global fonts, #1791 or
dependency/security changes.

## Task outline

- [ ] **1. Establish coherent Copy admission and automatic-name primitives.**
  - Contract: reuse package validation and storage admission, not independent
    public API calls. Add transaction-capable source snapshot/read and copy
    primitives where existing pool-based reads cannot guarantee coherence.
    Snapshot includes catalog name, package and complete presentation.
    Revalidate it after acquiring the existing Media locks; reject changes or
    unauthorized references before committing destination state.
  - DI: extend the focused theme manager with only its required handles,
    existing WriteScope and Media lock dependencies. Retain the verified
    `Arc<host::system_theme::SystemArtifactInventory>` at startup and provide
    that single-purpose context for canonical built-in source access.
    `server::system_artifacts::load` remains server-owned: no web→server
    dependency, new heterogeneous bundle or compile-all-themes per request.
    Check canonical package source/revision against installed metadata.
  - Naming: share one transactional allocator for omitted-name Upload and Copy.
    Coordinate strict create, automatic create and rename using the existing
    site-quota-row serialization boundary and consistent lock order; SQLite
    retains BEGIN IMMEDIATE. Keep owner-local unique keys as final enforcement.
    Preserve strict `create_theme -> ThemeId` compatibility where practical;
    automatic creation returns its actual ID/name to the caller.
  - Verification: dual-backend `#[apply(backends)]` tests for case-folded
    collisions, gaps, reserved bases, numeric endings, 100-scalar and multibyte
    suffix fitting, invalid bases and unchanged explicit-name rejection.
    Concurrent Upload/Copy/strict-create/rename tests must prove valid unique
    outcomes without client prechecks or transaction replay. No migration is
    assumed necessary.

- [ ] **2. Complete the new Copy mutation and Upload naming behavior.**
  - Contract: authenticate once, acquire the operation permit once, and create
    package plus authorized bindings in one WriteScope callback. Keep Media
    provenance, explicit absence/default bindings, pool entries and shuffle
    state; do not reconstruct ownership from ownerless wire Media values.
    Callback rejection leaves no committed destination. Existing filesystem
    staging/cleanup semantics remain explicit, not a distributed transaction.
  - Verification: backend-parametric endpoint integration through existing
    `post_server_fn`/`make_app`: built-ins and saved custom sources; unpublished
    sources; Site/operator and Author access; foreign/missing IDs; unavailable
    canonical source; source changes during lock acquisition; unauthorized Site
    Media; quota/validation/binding failure; source unchanged and no
    activation/publication. Inject commit-acknowledgement loss and prove one
    mutation attempt plus truthful reconciliation, not automatic duplicate Copy.
    Register the generated type and update wire assertions.

- [ ] **3. Integrate tile actions and current-source delivery proof.**
  - Contract: whole-tile explicit Use and entire-current-surface highlight;
    separate accessible Edit action outside the Use button. Upload/Copy are
    labelled peer actions with no name controls; Copy first presents the
    authorized source choices without applying them. Remove empty-library
    filler. Clearly distinguish current selection, editing and saved preview;
    keep current custom Use available when saved edits require publication.
    Opt-in editing remains the proposed Lead choice.
  - Verification: focused browser red/green for keyboard/accessibility,
    current-tile Use, nonactivating Edit/source choice, omitted upload names,
    Copy success/failure and owner switches. Preserve approved preview
    isolation/error/stale guards and existing lifecycle regression consumers.
    Obtain current backend/browser proof and empirical new-endpoint coverage;
    regenerate/verify the flow ledger only from an authoritative real producer.
    Review the cumulative deliverable, run normal hook/boundary gates, and
    produce genuine source-qualified Before/After comparisons. Historical
    preview samples and prior passing counts are not final acceptance.

## Risk checks and handoff

Names must be allocated inside the write boundary, not list-then-create.
Serialization must include rename; duplicate-name conflicts cannot become
unbounded retries. Package-only Copy is not the proposed behavior: it would lose
presentation bindings and requires an explicit alternative decision. Normal
Media authorization remains mandatory even when rejection surprises a Site
operator.

Preserve ADR-0016 exact dependencies, ADR-0184 package/privacy/Media boundaries,
derived endpoint conventions and empirical flow coverage. Task checkboxes track
execution progress; this approval authorizes implementation, not PR merge or
feature completion.
